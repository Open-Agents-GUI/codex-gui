use crate::gui::{
    ChatSettings, ModelOption, PermissionMode, PermissionProfileOption, permission_profile_label,
};
use codex_app_server_client::{
    DEFAULT_IN_PROCESS_CHANNEL_CAPACITY, EnvironmentManager, ExecServerRuntimePaths,
    InProcessAppServerClient, InProcessAppServerRequestHandle, InProcessClientStartArgs,
    InProcessServerEvent, TypedRequestError, legacy_core::config::Config,
};
use codex_app_server_protocol::{
    ApprovalsReviewer, AskForApproval, ClientRequest, ConfigBatchWriteParams, ConfigEdit,
    ConfigReadParams, ConfigReadResponse, ConfigWarningNotification, ConfigWriteResponse,
    JSONRPCErrorError, MergeStrategy, ModelListParams, ModelListResponse,
    PermissionProfileListParams, PermissionProfileListResponse, RequestId, ServerNotification,
    ServerRequest, SortDirection, Thread, ThreadDeleteParams, ThreadDeleteResponse,
    ThreadForkParams, ThreadForkResponse, ThreadListParams, ThreadListResponse, ThreadResumeParams,
    ThreadResumeResponse, ThreadSettingsUpdateParams, ThreadSettingsUpdateResponse, ThreadSortKey,
    ThreadSource, ThreadStartParams, ThreadStartResponse, TurnInterruptParams,
    TurnInterruptResponse, TurnStartParams, TurnStartResponse, TurnSteerParams, TurnSteerResponse,
    UserInput,
};
use codex_arg0::Arg0DispatchPaths;
use codex_protocol::{openai_models::ReasoningEffort, protocol::SessionSource};
use serde::de::DeserializeOwned;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot, watch},
};

#[derive(Clone)]
pub struct AppServerBridge {
    inner: Arc<BridgeInner>,
}

struct BridgeInner {
    client_state: watch::Receiver<ClientState>,
    shutdown_tx: watch::Sender<bool>,
    next_request_id: AtomicI64,
    server_response_tx: mpsc::UnboundedSender<ServerResponseCommand>,
}

enum ServerResponseCommand {
    Resolve {
        request_id: RequestId,
        result: serde_json::Value,
        response_tx: oneshot::Sender<Result<(), String>>,
    },
    Reject {
        request_id: RequestId,
        error: JSONRPCErrorError,
        response_tx: oneshot::Sender<Result<(), String>>,
    },
}

impl Drop for BridgeInner {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

#[derive(Clone)]
enum ClientState {
    Starting,
    Ready(InProcessAppServerRequestHandle),
    Failed(String),
    Stopped,
}

pub enum BridgeEvent {
    Notification(ServerNotification),
    ServerRequest(ServerRequest),
    TransportError(String),
    Lagged { skipped: usize },
}

#[derive(Debug)]
pub enum BridgeError {
    Transport(String),
    Rpc(String),
    Decode(String),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(message) | Self::Rpc(message) | Self::Decode(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for BridgeError {}

type BridgeResult<T> = Result<T, BridgeError>;

#[derive(Clone, Debug, Default)]
pub struct PersistedChatSettings {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub approvals_reviewer: Option<ApprovalsReviewer>,
}

pub fn start_app_server_bridge(
    runtime: Handle,
    arg0_paths: Arg0DispatchPaths,
) -> (AppServerBridge, mpsc::UnboundedReceiver<BridgeEvent>) {
    let (client_state_tx, client_state_rx) = watch::channel(ClientState::Starting);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (server_response_tx, server_response_rx) = mpsc::unbounded_channel();
    runtime.spawn(run_embedded_app_server(
        arg0_paths,
        client_state_tx,
        shutdown_rx,
        event_tx,
        server_response_rx,
    ));

    (
        AppServerBridge {
            inner: Arc::new(BridgeInner {
                client_state: client_state_rx,
                shutdown_tx,
                next_request_id: AtomicI64::new(1),
                server_response_tx,
            }),
        },
        event_rx,
    )
}

impl AppServerBridge {
    pub async fn wait_until_ready(&self) -> BridgeResult<()> {
        self.request_handle().await.map(|_| ())
    }

    pub async fn shutdown(&self) {
        let _ = self.inner.shutdown_tx.send(true);
        let mut state = self.inner.client_state.clone();
        loop {
            match &*state.borrow() {
                ClientState::Failed(_) | ClientState::Stopped => return,
                ClientState::Starting | ClientState::Ready(_) => {}
            }
            if state.changed().await.is_err() {
                return;
            }
        }
    }

    pub async fn resolve_server_request(
        &self,
        request_id: RequestId,
        result: serde_json::Value,
    ) -> BridgeResult<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.inner
            .server_response_tx
            .send(ServerResponseCommand::Resolve {
                request_id,
                result,
                response_tx,
            })
            .map_err(|_| BridgeError::Transport("app-server bridge is closed".into()))?;
        response_rx
            .await
            .map_err(|_| BridgeError::Transport("app-server response channel closed".into()))?
            .map_err(BridgeError::Transport)
    }

    pub async fn reject_server_request(
        &self,
        request_id: RequestId,
        message: String,
    ) -> BridgeResult<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.inner
            .server_response_tx
            .send(ServerResponseCommand::Reject {
                request_id,
                error: JSONRPCErrorError {
                    code: -32000,
                    message,
                    data: None,
                },
                response_tx,
            })
            .map_err(|_| BridgeError::Transport("app-server bridge is closed".into()))?;
        response_rx
            .await
            .map_err(|_| BridgeError::Transport("app-server response channel closed".into()))?
            .map_err(BridgeError::Transport)
    }

    pub async fn list_threads(&self) -> BridgeResult<Vec<Thread>> {
        let mut threads = Vec::new();
        let mut cursor = None;

        loop {
            let response: ThreadListResponse = self
                .request(|request_id| ClientRequest::ThreadList {
                    request_id,
                    params: ThreadListParams {
                        cursor,
                        limit: Some(100),
                        sort_key: Some(ThreadSortKey::UpdatedAt),
                        sort_direction: Some(SortDirection::Desc),
                        model_providers: None,
                        source_kinds: None,
                        archived: Some(false),
                        cwd: None,
                        use_state_db_only: false,
                        search_term: None,
                        parent_thread_id: None,
                        ancestor_thread_id: None,
                        section_id: None,
                    },
                })
                .await?;

            threads.extend(response.data);
            cursor = response.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        Ok(threads)
    }

    pub async fn list_models(&self) -> BridgeResult<Vec<ModelOption>> {
        let response: ModelListResponse = self
            .request(|request_id| ClientRequest::ModelList {
                request_id,
                params: ModelListParams {
                    cursor: None,
                    limit: None,
                    include_hidden: None,
                },
            })
            .await?;
        Ok(response
            .data
            .into_iter()
            .filter(|model| !model.hidden)
            .map(|model| ModelOption {
                id: model.model,
                display_name: model.display_name,
                supported_efforts: model
                    .supported_reasoning_efforts
                    .into_iter()
                    .map(|effort| effort.reasoning_effort.to_string())
                    .collect(),
                default_effort: model.default_reasoning_effort.to_string(),
            })
            .collect())
    }

    pub async fn read_chat_settings(&self) -> BridgeResult<PersistedChatSettings> {
        let response: ConfigReadResponse = self
            .request(|request_id| ClientRequest::ConfigRead {
                request_id,
                params: ConfigReadParams {
                    include_layers: false,
                    cwd: None,
                },
            })
            .await?;
        Ok(PersistedChatSettings {
            model: response.config.model,
            effort: response
                .config
                .model_reasoning_effort
                .map(|effort| effort.to_string()),
            approvals_reviewer: response.config.approvals_reviewer,
        })
    }

    pub async fn write_chat_settings(&self, settings: ChatSettings) -> BridgeResult<()> {
        let _: ConfigWriteResponse = self
            .request(|request_id| ClientRequest::ConfigBatchWrite {
                request_id,
                params: ConfigBatchWriteParams {
                    edits: vec![
                        ConfigEdit {
                            key_path: "model".into(),
                            value: serde_json::json!(settings.model),
                            merge_strategy: MergeStrategy::Replace,
                        },
                        ConfigEdit {
                            key_path: "model_reasoning_effort".into(),
                            value: serde_json::json!(settings.effort),
                            merge_strategy: MergeStrategy::Replace,
                        },
                        ConfigEdit {
                            key_path: "approvals_reviewer".into(),
                            value: serde_json::json!(settings.approvals_reviewer),
                            merge_strategy: MergeStrategy::Replace,
                        },
                    ],
                    file_path: None,
                    expected_version: None,
                    reload_user_config: false,
                },
            })
            .await?;
        Ok(())
    }

    pub async fn list_permission_profiles(
        &self,
        cwd: String,
    ) -> BridgeResult<Vec<PermissionProfileOption>> {
        let response: PermissionProfileListResponse = self
            .request(|request_id| ClientRequest::PermissionProfileList {
                request_id,
                params: PermissionProfileListParams {
                    cursor: None,
                    limit: None,
                    cwd: Some(cwd),
                },
            })
            .await?;
        Ok(response
            .data
            .into_iter()
            .map(|profile| PermissionProfileOption {
                label: permission_profile_label(&profile.id),
                id: profile.id,
                description: profile.description,
            })
            .collect())
    }

    pub async fn start_thread(&self, cwd: String, settings: ChatSettings) -> BridgeResult<Thread> {
        let response: ThreadStartResponse = self
            .request(|request_id| ClientRequest::ThreadStart {
                request_id,
                params: ThreadStartParams {
                    cwd: Some(cwd),
                    model: Some(settings.model.clone()),
                    approval_policy: Some(approval_policy_for(&settings)),
                    approvals_reviewer: Some(settings.approvals_reviewer),
                    permissions: Some(settings.permission_profile.clone()),
                    sandbox: None,
                    thread_source: Some(ThreadSource::User),
                    ..Default::default()
                },
            })
            .await?;
        Ok(response.thread)
    }

    pub async fn resume_thread(&self, thread_id: String) -> BridgeResult<Thread> {
        let response: ThreadResumeResponse = self
            .request(|request_id| ClientRequest::ThreadResume {
                request_id,
                params: ThreadResumeParams {
                    thread_id,
                    ..Default::default()
                },
            })
            .await?;
        Ok(response.thread)
    }

    pub async fn fork_thread(
        &self,
        thread_id: String,
        last_turn_id: Option<String>,
        before_turn_id: Option<String>,
    ) -> BridgeResult<Thread> {
        let response: ThreadForkResponse = self
            .request(|request_id| ClientRequest::ThreadFork {
                request_id,
                params: ThreadForkParams {
                    thread_id,
                    last_turn_id,
                    before_turn_id,
                    ..Default::default()
                },
            })
            .await?;
        Ok(response.thread)
    }

    pub async fn delete_thread(&self, thread_id: String) -> BridgeResult<()> {
        let _: ThreadDeleteResponse = self
            .request(|request_id| ClientRequest::ThreadDelete {
                request_id,
                params: ThreadDeleteParams { thread_id },
            })
            .await?;
        Ok(())
    }

    pub async fn send_turn(
        &self,
        thread_id: String,
        client_user_message_id: String,
        input: Vec<UserInput>,
        settings: ChatSettings,
    ) -> BridgeResult<TurnStartResponse> {
        self.request(|request_id| ClientRequest::TurnStart {
            request_id,
            params: TurnStartParams {
                thread_id,
                client_user_message_id: Some(client_user_message_id),
                input,
                responsesapi_client_metadata: None,
                additional_context: None,
                environments: None,
                cwd: None,
                runtime_workspace_roots: None,
                approval_policy: Some(approval_policy_for(&settings)),
                approvals_reviewer: Some(settings.approvals_reviewer),
                sandbox_policy: None,
                permissions: Some(settings.permission_profile.clone()),
                model: Some(settings.model.clone()),
                service_tier: None,
                effort: Some(reasoning_effort_for(&settings)),
                summary: None,
                personality: None,
                output_schema: None,
                collaboration_mode: None,
                multi_agent_mode: None,
            },
        })
        .await
    }

    pub async fn steer_turn(
        &self,
        thread_id: String,
        turn_id: String,
        client_user_message_id: String,
        input: Vec<UserInput>,
    ) -> BridgeResult<TurnSteerResponse> {
        self.request(|request_id| ClientRequest::TurnSteer {
            request_id,
            params: TurnSteerParams {
                thread_id,
                client_user_message_id: Some(client_user_message_id),
                input,
                responsesapi_client_metadata: None,
                additional_context: None,
                expected_turn_id: turn_id,
            },
        })
        .await
    }

    pub async fn interrupt_turn(&self, thread_id: String, turn_id: String) -> BridgeResult<()> {
        let _: TurnInterruptResponse = self
            .request(|request_id| ClientRequest::TurnInterrupt {
                request_id,
                params: TurnInterruptParams { thread_id, turn_id },
            })
            .await?;
        Ok(())
    }

    pub async fn update_thread_settings(
        &self,
        thread_id: String,
        settings: ChatSettings,
    ) -> BridgeResult<()> {
        let _: ThreadSettingsUpdateResponse = self
            .request(|request_id| ClientRequest::ThreadSettingsUpdate {
                request_id,
                params: ThreadSettingsUpdateParams {
                    thread_id,
                    approval_policy: Some(approval_policy_for(&settings)),
                    approvals_reviewer: Some(settings.approvals_reviewer),
                    permissions: Some(settings.permission_profile.clone()),
                    model: Some(settings.model.clone()),
                    effort: Some(reasoning_effort_for(&settings)),
                    ..Default::default()
                },
            })
            .await?;
        Ok(())
    }

    async fn request<T>(&self, build: impl FnOnce(RequestId) -> ClientRequest) -> BridgeResult<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let client = self.request_handle().await?;
        let request_id =
            RequestId::Integer(self.inner.next_request_id.fetch_add(1, Ordering::Relaxed));
        let request = build(request_id);
        client
            .request_typed(request)
            .await
            .map_err(BridgeError::from)
    }

    async fn request_handle(&self) -> BridgeResult<InProcessAppServerRequestHandle> {
        let mut state = self.inner.client_state.clone();
        loop {
            let snapshot = state.borrow().clone();
            match snapshot {
                ClientState::Ready(client) => return Ok(client),
                ClientState::Failed(message) => return Err(BridgeError::Transport(message)),
                ClientState::Stopped => {
                    return Err(BridgeError::Transport("embedded app-server stopped".into()));
                }
                ClientState::Starting => {}
            }
            state.changed().await.map_err(|_| {
                BridgeError::Transport("embedded app-server startup task stopped".into())
            })?;
        }
    }
}

impl From<TypedRequestError> for BridgeError {
    fn from(error: TypedRequestError) -> Self {
        match error {
            TypedRequestError::Transport { source, .. } => Self::Transport(source.to_string()),
            TypedRequestError::Server { source, .. } => Self::Rpc(source.message),
            TypedRequestError::Deserialize { source, .. } => Self::Decode(source.to_string()),
        }
    }
}

async fn run_embedded_app_server(
    arg0_paths: Arg0DispatchPaths,
    client_state: watch::Sender<ClientState>,
    mut shutdown: watch::Receiver<bool>,
    events: mpsc::UnboundedSender<BridgeEvent>,
    mut server_responses: mpsc::UnboundedReceiver<ServerResponseCommand>,
) {
    let mut client = match build_embedded_client(arg0_paths).await {
        Ok(client) => client,
        Err(error) => {
            client_state.send_replace(ClientState::Failed(error.to_string()));
            return;
        }
    };

    client_state.send_replace(ClientState::Ready(client.request_handle()));
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            command = server_responses.recv() => {
                let Some(command) = command else {
                    break;
                };
                match command {
                    ServerResponseCommand::Resolve { request_id, result, response_tx } => {
                        let result = client
                            .resolve_server_request(request_id, result)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = response_tx.send(result);
                    }
                    ServerResponseCommand::Reject { request_id, error, response_tx } => {
                        let result = client
                            .reject_server_request(request_id, error)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = response_tx.send(result);
                    }
                }
            }
            event = client.next_event() => {
                let Some(event) = event else {
                    if !*shutdown.borrow() {
                        let _ = events.send(BridgeEvent::TransportError(
                            "embedded app-server event stream closed".into(),
                        ));
                    }
                    break;
                };
                match event {
                    InProcessServerEvent::ServerNotification(notification) => {
                        let _ = events.send(BridgeEvent::Notification(*notification));
                    }
                    InProcessServerEvent::ServerRequest(request) => {
                        let _ = events.send(BridgeEvent::ServerRequest(*request));
                    }
                    InProcessServerEvent::Lagged { skipped } => {
                        let _ = events.send(BridgeEvent::Lagged { skipped });
                    }
                }
            }
        }
    }

    if let Err(error) = client.shutdown().await {
        let _ = events.send(BridgeEvent::TransportError(format!(
            "failed to shut down embedded app-server: {error}",
        )));
    }
    client_state.send_replace(ClientState::Stopped);
}

async fn build_embedded_client(
    arg0_paths: Arg0DispatchPaths,
) -> BridgeResult<InProcessAppServerClient> {
    let cli_overrides = Vec::new();
    let config = Config::load_with_cli_overrides(cli_overrides.clone())
        .await
        .map_err(|err| BridgeError::Transport(format!("failed to load Codex config: {err}")))?;
    let config_warnings = config
        .startup_warnings
        .iter()
        .map(|warning| ConfigWarningNotification {
            summary: warning.clone(),
            details: None,
            path: None,
            range: None,
        })
        .collect();
    let runtime_paths = ExecServerRuntimePaths::from_optional_paths(
        arg0_paths.codex_self_exe.clone(),
        arg0_paths.codex_linux_sandbox_exe.clone(),
    )
    .map_err(|err| {
        BridgeError::Transport(format!("failed to configure Codex helper paths: {err}"))
    })?;
    let environment_manager = EnvironmentManager::from_codex_home(
        config.codex_home.clone(),
        Some(runtime_paths),
        config.http_client_factory(),
    )
    .await
    .map_err(|err| {
        BridgeError::Transport(format!("failed to initialize Codex environments: {err}"))
    })?;
    let state_db = codex_core::init_state_db(&config).await;

    InProcessAppServerClient::start(InProcessClientStartArgs {
        arg0_paths,
        config: Arc::new(config),
        cli_overrides,
        loader_overrides: Default::default(),
        strict_config: false,
        cloud_config_bundle: Default::default(),
        feedback: Default::default(),
        log_db: None,
        state_db,
        environment_manager: Arc::new(environment_manager),
        config_warnings,
        session_source: SessionSource::VSCode,
        enable_codex_api_key_env: true,
        client_name: "codex-gui".into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
        experimental_api: true,
        mcp_server_openai_form_elicitation: false,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
    .await
    .map_err(|err| BridgeError::Transport(format!("failed to start embedded app-server: {err}")))
}

fn approval_policy_for(settings: &ChatSettings) -> AskForApproval {
    if settings.permission_profile == PermissionMode::DangerFullAccess.profile_id() {
        AskForApproval::Never
    } else {
        AskForApproval::OnRequest
    }
}

fn reasoning_effort_for(settings: &ChatSettings) -> ReasoningEffort {
    settings
        .effort
        .parse()
        .unwrap_or_else(|_| ReasoningEffort::Medium)
}

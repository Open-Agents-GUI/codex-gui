use super::{
    BridgeError, BridgeEvent, BridgeRequestHandle, BridgeResult, ClientState, ServerResponseCommand,
};
use codex_app_server_protocol::{
    ClientInfo, ClientNotification, ClientRequest, InitializeCapabilities, InitializeParams,
    InitializeResponse, JSONRPCError, JSONRPCMessage, JSONRPCNotification, JSONRPCRequest,
    JSONRPCResponse, RequestId, ServerNotification, ServerRequest,
};
use serde::de::DeserializeOwned;
use std::{collections::HashMap, ffi::OsString, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot, watch},
    time::timeout,
};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(60);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct DshLaunchConfig {
    program: OsString,
    args: Vec<OsString>,
    current_dir: Option<PathBuf>,
}

impl DshLaunchConfig {
    pub fn from_executable(executable: PathBuf, profile: String) -> Self {
        Self {
            program: executable.into_os_string(),
            args: vec!["--profile".into(), profile.into()],
            current_dir: None,
        }
    }

    pub fn from_repository(repository: PathBuf, profile: String) -> Self {
        Self {
            program: "nix".into(),
            args: vec![
                "develop".into(),
                repository.clone().into_os_string(),
                "-c".into(),
                "pnpm".into(),
                "dsh".into(),
                "--profile".into(),
                profile.into(),
            ],
            current_dir: Some(repository),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(current_dir) = &self.current_dir {
            command.current_dir(current_dir);
        }
        command
    }

    fn display(&self) -> String {
        std::iter::once(&self.program)
            .chain(self.args.iter())
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone)]
pub(super) struct StdioRequestHandle {
    requests: mpsc::UnboundedSender<StdioRequestCommand>,
}

struct StdioRequestCommand {
    request: ClientRequest,
    response: oneshot::Sender<BridgeResult<serde_json::Value>>,
}

impl StdioRequestHandle {
    pub(super) async fn request_typed<T>(&self, request: ClientRequest) -> BridgeResult<T>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let method = request.method_name().to_owned();
        let (response, response_rx) = oneshot::channel();
        self.requests
            .send(StdioRequestCommand { request, response })
            .map_err(|_| BridgeError::Transport("dsh app-server transport is closed".into()))?;
        let result = response_rx.await.map_err(|_| {
            BridgeError::Transport("dsh app-server response channel is closed".into())
        })??;
        serde_json::from_value(result).map_err(|error| {
            BridgeError::Decode(format!("{method} response decode error: {error}"))
        })
    }
}

pub(super) async fn run_dsh_app_server(
    launch: DshLaunchConfig,
    client_state: watch::Sender<ClientState>,
    mut shutdown: watch::Receiver<bool>,
    events: mpsc::UnboundedSender<BridgeEvent>,
    mut server_responses: mpsc::UnboundedReceiver<ServerResponseCommand>,
) {
    let command_display = launch.display();
    let mut command = launch.command();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            client_state.send_replace(ClientState::Failed(format!(
                "failed to start dsh app-server `{command_display}`: {error}"
            )));
            return;
        }
    };
    let Some(mut input) = child.stdin.take() else {
        client_state.send_replace(ClientState::Failed(
            "dsh app-server stdin was not piped".into(),
        ));
        return;
    };
    let Some(output) = child.stdout.take() else {
        client_state.send_replace(ClientState::Failed(
            "dsh app-server stdout was not piped".into(),
        ));
        return;
    };
    let mut lines = BufReader::new(output).lines();

    if let Err(error) = initialize(&mut input, &mut lines, &events).await {
        client_state.send_replace(ClientState::Failed(format!(
            "failed to initialize dsh app-server `{command_display}`: {error}"
        )));
        stop_child(child, input, lines).await;
        return;
    }

    let (request_tx, mut request_rx) = mpsc::unbounded_channel();
    client_state.send_replace(ClientState::Ready(BridgeRequestHandle::Stdio(
        StdioRequestHandle {
            requests: request_tx,
        },
    )));

    let mut pending = HashMap::<RequestId, oneshot::Sender<BridgeResult<serde_json::Value>>>::new();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            command = request_rx.recv() => {
                let Some(command) = command else {
                    break;
                };
                let request = match jsonrpc_request(command.request) {
                    Ok(request) => request,
                    Err(error) => {
                        let _ = command
                            .response
                            .send(Err(BridgeError::Decode(error)));
                        continue;
                    }
                };
                let request_id = request.id.clone();
                if let Err(error) = write_message(&mut input, JSONRPCMessage::Request(request)).await {
                    let _ = command.response.send(Err(BridgeError::Transport(error.clone())));
                    let _ = events.send(BridgeEvent::TransportError(error));
                    break;
                }
                pending.insert(request_id, command.response);
            }
            command = server_responses.recv() => {
                let Some(command) = command else {
                    break;
                };
                let (message, response) = match command {
                    ServerResponseCommand::Resolve { request_id, result, response_tx } => (
                        JSONRPCMessage::Response(JSONRPCResponse { id: request_id, result }),
                        response_tx,
                    ),
                    ServerResponseCommand::Reject { request_id, error, response_tx } => (
                        JSONRPCMessage::Error(JSONRPCError { id: request_id, error }),
                        response_tx,
                    ),
                };
                let result = write_message(&mut input, message).await;
                let failed = result.is_err();
                let _ = response.send(result.clone());
                if let Err(error) = result {
                    let _ = events.send(BridgeEvent::TransportError(error));
                }
                if failed {
                    break;
                }
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Err(error) = route_server_message(&line, &mut input, &mut pending, &events).await {
                            let _ = events.send(BridgeEvent::TransportError(error));
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = events.send(BridgeEvent::TransportError(
                            "dsh app-server closed its stdout stream".into(),
                        ));
                        break;
                    }
                    Err(error) => {
                        let _ = events.send(BridgeEvent::TransportError(format!(
                            "failed to read dsh app-server stdout: {error}"
                        )));
                        break;
                    }
                }
            }
        }
    }

    for (_, response) in pending {
        let _ = response.send(Err(BridgeError::Transport(
            "dsh app-server stopped before responding".into(),
        )));
    }
    stop_child(child, input, lines).await;
    client_state.send_replace(ClientState::Stopped);
}

async fn initialize(
    input: &mut ChildStdin,
    lines: &mut Lines<BufReader<ChildStdout>>,
    events: &mpsc::UnboundedSender<BridgeEvent>,
) -> Result<(), String> {
    let request_id = RequestId::String("dsh-gui-initialize".into());
    let request = ClientRequest::Initialize {
        request_id: request_id.clone(),
        params: InitializeParams {
            client_info: ClientInfo {
                name: "dsh-gui".into(),
                title: Some("DSH GUI".into()),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            capabilities: Some(InitializeCapabilities {
                experimental_api: true,
                request_attestation: false,
                mcp_server_openai_form_elicitation: false,
                opt_out_notification_methods: None,
                extensions: None,
            }),
        },
    };
    write_message(input, JSONRPCMessage::Request(jsonrpc_request(request)?)).await?;

    timeout(INITIALIZE_TIMEOUT, async {
        loop {
            let line = lines
                .next_line()
                .await
                .map_err(|error| format!("failed to read initialize response: {error}"))?
                .ok_or_else(|| "server closed before initialize response".to_string())?;
            let message = serde_json::from_str::<JSONRPCMessage>(&line)
                .map_err(|error| format!("invalid JSON-RPC during initialize: {error}"))?;
            match message {
                JSONRPCMessage::Response(response) if response.id == request_id => {
                    serde_json::from_value::<InitializeResponse>(response.result)
                        .map_err(|error| format!("invalid initialize result: {error}"))?;
                    break Ok(());
                }
                JSONRPCMessage::Error(error) if error.id == request_id => {
                    break Err(format!(
                        "initialize failed: {} (code {})",
                        error.error.message, error.error.code
                    ));
                }
                JSONRPCMessage::Notification(notification) => {
                    if let Ok(notification) = ServerNotification::try_from(notification) {
                        let _ = events.send(BridgeEvent::Notification(notification));
                    }
                }
                JSONRPCMessage::Request(request) => {
                    let rejection = JSONRPCMessage::Error(JSONRPCError {
                        id: request.id,
                        error: codex_app_server_protocol::JSONRPCErrorError {
                            code: -32601,
                            message: format!(
                                "server request `{}` is not available during initialization",
                                request.method
                            ),
                            data: None,
                        },
                    });
                    write_message(input, rejection).await?;
                }
                JSONRPCMessage::Response(_) | JSONRPCMessage::Error(_) => {}
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for initialize response".to_string())??;

    write_message(
        input,
        JSONRPCMessage::Notification(jsonrpc_notification(ClientNotification::Initialized)?),
    )
    .await
}

async fn route_server_message(
    line: &str,
    input: &mut ChildStdin,
    pending: &mut HashMap<RequestId, oneshot::Sender<BridgeResult<serde_json::Value>>>,
    events: &mpsc::UnboundedSender<BridgeEvent>,
) -> Result<(), String> {
    let message = serde_json::from_str::<JSONRPCMessage>(line)
        .map_err(|error| format!("dsh app-server sent invalid JSON-RPC: {error}"))?;
    match message {
        JSONRPCMessage::Response(response) => {
            if let Some(sender) = pending.remove(&response.id) {
                let _ = sender.send(Ok(response.result));
            } else {
                tracing::warn!(request_id = %response.id, "ignored unexpected dsh response");
            }
        }
        JSONRPCMessage::Error(error) => {
            if let Some(sender) = pending.remove(&error.id) {
                let _ = sender.send(Err(BridgeError::Rpc(format!(
                    "{} (code {})",
                    error.error.message, error.error.code
                ))));
            } else {
                tracing::warn!(request_id = %error.id, "ignored unexpected dsh error response");
            }
        }
        JSONRPCMessage::Notification(notification) => {
            let method = notification.method.clone();
            match ServerNotification::try_from(notification) {
                Ok(notification) => {
                    let _ = events.send(BridgeEvent::Notification(notification));
                }
                Err(error) => tracing::warn!(%method, %error, "ignored unknown dsh notification"),
            }
        }
        JSONRPCMessage::Request(request) => {
            let request_id = request.id.clone();
            let method = request.method.clone();
            match ServerRequest::try_from(request) {
                Ok(request) => {
                    let _ = events.send(BridgeEvent::ServerRequest(request));
                }
                Err(error) => {
                    tracing::warn!(%method, %error, "rejecting unknown dsh server request");
                    write_message(
                        input,
                        JSONRPCMessage::Error(JSONRPCError {
                            id: request_id,
                            error: codex_app_server_protocol::JSONRPCErrorError {
                                code: -32601,
                                message: format!("unsupported server request `{method}`"),
                                data: None,
                            },
                        }),
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

fn jsonrpc_request(request: ClientRequest) -> Result<JSONRPCRequest, String> {
    serde_json::to_value(request)
        .and_then(serde_json::from_value)
        .map_err(|error| format!("failed to encode app-server request: {error}"))
}

fn jsonrpc_notification(notification: ClientNotification) -> Result<JSONRPCNotification, String> {
    serde_json::to_value(notification)
        .and_then(serde_json::from_value)
        .map_err(|error| format!("failed to encode app-server notification: {error}"))
}

async fn write_message(input: &mut ChildStdin, message: JSONRPCMessage) -> Result<(), String> {
    let mut encoded = serde_json::to_vec(&message)
        .map_err(|error| format!("failed to encode app-server message: {error}"))?;
    encoded.push(b'\n');
    input
        .write_all(&encoded)
        .await
        .map_err(|error| format!("failed to write dsh app-server stdin: {error}"))?;
    input
        .flush()
        .await
        .map_err(|error| format!("failed to flush dsh app-server stdin: {error}"))
}

async fn stop_child(mut child: Child, input: ChildStdin, lines: Lines<BufReader<ChildStdout>>) {
    drop(input);
    drop(lines);
    if timeout(SHUTDOWN_TIMEOUT, child.wait()).await.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

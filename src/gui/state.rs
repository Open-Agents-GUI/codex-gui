use std::collections::HashMap;

use codex_app_server_protocol::{
    ApprovalsReviewer, CommandExecutionApprovalDecision, CommandExecutionRequestApprovalResponse,
    FileChangeApprovalDecision, FileChangeRequestApprovalResponse, FileUpdateChange,
    GrantedPermissionProfile, McpServerElicitationAction, McpServerElicitationRequestResponse,
    PermissionGrantScope, PermissionsRequestApprovalResponse, RequestId, Thread, ThreadItem,
    ThreadStatus, ToolRequestUserInputAnswer, ToolRequestUserInputQuestion,
    ToolRequestUserInputResponse, Turn, TurnPlanStep, TurnStatus, UserInput,
};
use gpui::{AppContext, Context, Entity, SharedString};
use uuid::Uuid;

pub(crate) fn new_client_user_message_id() -> String {
    format!("codex-gui-{}", Uuid::new_v4())
}

pub(crate) fn single_line_title(title: &str) -> String {
    title
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct WorkspaceState {
    pub projects: Vec<Entity<ProjectState>>,
    pub projectless_chats: Vec<Entity<ChatState>>,
    pub active_project: usize,
    pub active_chat: usize,
    pub active_projectless_chat: Option<usize>,
    /// Whether the app server has answered the initial thread listing.
    pub threads_loaded: bool,
    /// Defaults used by the new-chat composer. Existing chats own their settings.
    pub new_chat_settings: ChatSettings,
    pub available_models: Vec<ModelOption>,
    pub permission_profiles: Vec<PermissionProfileOption>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self {
            projects: Vec::new(),
            projectless_chats: Vec::new(),
            active_project: 0,
            active_chat: 0,
            active_projectless_chat: None,
            threads_loaded: false,
            new_chat_settings: ChatSettings::default(),
            available_models: Vec::new(),
            permission_profiles: default_permission_profiles(),
        }
    }

    /// The currently selected chat, whether it lives under a project or in the
    /// project-less chat list.
    pub fn active_chat_entity(&self, cx: &impl AppContext) -> Option<Entity<ChatState>> {
        if let Some(index) = self.active_projectless_chat {
            return self.projectless_chats.get(index).cloned();
        }
        let project = self.projects.get(self.active_project)?;
        let chats = project.read_with(cx, |project, _| project.chats.clone());
        chats.get(self.active_chat).cloned()
    }

    pub fn active_project(&self) -> Option<Entity<ProjectState>> {
        self.projects.get(self.active_project).cloned()
    }

    pub fn project_index_by_path(&self, path: &str, cx: &mut Context<Self>) -> Option<usize> {
        self.projects
            .iter()
            .position(|project| project.read(cx).path.as_ref() == path)
    }

    pub fn select_project(&mut self, index: usize) {
        self.active_project = index;
        self.active_chat = 0;
        self.active_projectless_chat = None;
    }

    pub fn sort_projects_by_recent_activity(&mut self, cx: &mut Context<Self>) {
        let active_path = self
            .active_project()
            .map(|project| project.read(cx).path.to_string());
        let mut projects = self
            .projects
            .iter()
            .enumerate()
            .map(|(index, project)| {
                let project_state = project.read(cx);
                (
                    index,
                    project_state.latest_thread_updated_at,
                    project_state.path.to_string(),
                    project.clone(),
                )
            })
            .collect::<Vec<_>>();

        projects.sort_by(|(a_index, a_updated_at, ..), (b_index, b_updated_at, ..)| {
            b_updated_at
                .cmp(a_updated_at)
                .then_with(|| a_index.cmp(b_index))
        });

        self.projects = projects
            .into_iter()
            .map(|(_, _, _, project)| project)
            .collect();

        if let Some(active_path) = active_path
            && let Some(index) = self
                .projects
                .iter()
                .position(|project| project.read(cx).path.as_ref() == active_path.as_str())
        {
            self.active_project = index;
        } else {
            self.active_project = self
                .active_project
                .min(self.projects.len().saturating_sub(1));
        }
    }

    pub fn select_chat(&mut self, index: usize) {
        self.active_chat = index;
        self.active_projectless_chat = None;
    }

    pub fn select_projectless_chat(&mut self, index: usize) {
        self.active_projectless_chat = Some(index);
        self.active_chat = 0;
    }

    pub fn select_chat_entity(
        &mut self,
        selected: &Entity<ChatState>,
        cx: &impl AppContext,
    ) -> bool {
        if let Some(index) = self
            .projectless_chats
            .iter()
            .position(|chat| chat == selected)
        {
            self.select_projectless_chat(index);
            return true;
        }
        if let Some((project_index, chat_index)) =
            self.projects
                .iter()
                .enumerate()
                .find_map(|(project_index, project)| {
                    project
                        .read_with(cx, |project, _| {
                            project.chats.iter().position(|chat| chat == selected)
                        })
                        .map(|chat_index| (project_index, chat_index))
                })
        {
            self.active_project = project_index;
            self.select_chat(chat_index);
            return true;
        }
        false
    }

    pub fn add_project(&mut self, project: Entity<ProjectState>) -> usize {
        self.projects.push(project);
        self.active_project = self.projects.len() - 1;
        self.active_chat = 0;
        self.active_projectless_chat = None;
        self.active_project
    }

    pub fn select_first_chat(&mut self) {
        self.active_chat = 0;
    }

    pub fn set_model(&mut self, model: String) {
        self.new_chat_settings.model = model;
        if let Some(option) = self
            .available_models
            .iter()
            .find(|option| option.id == self.new_chat_settings.model)
        {
            self.new_chat_settings.effort = option.default_effort.clone();
        }
    }

    pub fn set_effort(&mut self, effort: String) {
        self.new_chat_settings.effort = effort;
    }

    pub fn set_permission_profile(&mut self, permission_profile: String) {
        self.new_chat_settings.permission_profile = permission_profile;
    }

    pub fn set_approvals_reviewer(&mut self, approvals_reviewer: ApprovalsReviewer) {
        self.new_chat_settings.approvals_reviewer = approvals_reviewer;
    }

    pub fn set_available_models(&mut self, models: Vec<ModelOption>) {
        if let Some(default_model) = models
            .first()
            .filter(|_| self.new_chat_settings.model.is_empty())
        {
            self.new_chat_settings.model = default_model.id.clone();
            self.new_chat_settings.effort = default_model.default_effort.clone();
        }
        self.available_models = models;
    }

    pub fn set_permission_profiles(&mut self, profiles: Vec<PermissionProfileOption>) {
        self.permission_profiles = profiles;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSettings {
    pub model: String,
    pub effort: String,
    pub permission_profile: String,
    pub approvals_reviewer: ApprovalsReviewer,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            model: "gpt-5.5".into(),
            effort: "medium".into(),
            permission_profile: PermissionMode::WorkspaceWrite.profile_id().into(),
            approvals_reviewer: ApprovalsReviewer::User,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOption {
    pub id: String,
    pub display_name: String,
    pub supported_efforts: Vec<String>,
    pub default_effort: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionProfileOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl PermissionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read only",
            Self::WorkspaceWrite => "Workspace write",
            Self::DangerFullAccess => "Full access",
        }
    }

    pub fn profile_id(self) -> &'static str {
        match self {
            Self::ReadOnly => ":read-only",
            Self::WorkspaceWrite => ":workspace",
            Self::DangerFullAccess => ":danger-full-access",
        }
    }
}

pub fn permission_profile_label(id: &str) -> String {
    match id {
        ":read-only" => PermissionMode::ReadOnly.label().into(),
        ":workspace" => PermissionMode::WorkspaceWrite.label().into(),
        ":danger-full-access" => PermissionMode::DangerFullAccess.label().into(),
        other => other.trim_start_matches(':').to_string(),
    }
}

fn default_permission_profiles() -> Vec<PermissionProfileOption> {
    [
        PermissionMode::ReadOnly,
        PermissionMode::WorkspaceWrite,
        PermissionMode::DangerFullAccess,
    ]
    .into_iter()
    .map(|mode| PermissionProfileOption {
        id: mode.profile_id().into(),
        label: mode.label().into(),
        description: None,
    })
    .collect()
}

pub fn approvals_reviewer_label(reviewer: ApprovalsReviewer) -> &'static str {
    match reviewer {
        ApprovalsReviewer::User => "Ask me",
        ApprovalsReviewer::AutoReview => "Approve for me",
    }
}

pub struct ProjectState {
    pub name: SharedString,
    pub path: SharedString,
    pub chats: Vec<Entity<ChatState>>,
    pub latest_thread_updated_at: Option<i64>,
}

impl ProjectState {
    pub fn new(name: SharedString, path: SharedString, chats: Vec<Entity<ChatState>>) -> Self {
        Self {
            name,
            path,
            chats,
            latest_thread_updated_at: None,
        }
    }

    pub fn replace_loaded_chats(
        &mut self,
        chats: Vec<Entity<ChatState>>,
        latest_thread_updated_at: Option<i64>,
    ) {
        self.chats = chats;
        self.latest_thread_updated_at = latest_thread_updated_at;
    }

    pub fn mark_thread_updated_at(&mut self, updated_at: i64) {
        self.latest_thread_updated_at = Some(
            self.latest_thread_updated_at
                .map(|current| current.max(updated_at))
                .unwrap_or(updated_at),
        );
    }

    pub fn chat_index_by_id(&self, chat_id: &str, cx: &mut Context<Self>) -> Option<usize> {
        self.chats
            .iter()
            .position(|chat| chat.read(cx).id == chat_id)
    }

    pub fn upsert_chat(
        &mut self,
        chat: Entity<ChatState>,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) -> usize {
        if let Some(index) = self.chat_index_by_id(chat_id, cx) {
            self.chats[index] = chat;
            index
        } else {
            self.chats.insert(0, chat);
            0
        }
    }

    pub fn append_chat(&mut self, chat: Entity<ChatState>) {
        self.chats.push(chat);
    }
}

pub struct ChatState {
    pub id: String,
    pub title: SharedString,
    pub subtitle: SharedString,
    pub thread: Option<Thread>,
    pub notices: Vec<HistoryNotice>,
    pub pending_approvals: Vec<PendingApproval>,
    pub pending_inputs: Vec<PendingUserInputRequest>,
    pub turn_plans: HashMap<String, TurnPlanView>,
    pub tool_progress: HashMap<String, Vec<SharedString>>,
    pub settings: ChatSettings,
    pub draft: String,
    pub editing_message: Option<EditingMessage>,
    pub active_turn: Option<ActiveTurn>,
    pub is_loading: bool,
    pub is_creating: bool,
    pending_user_message: Option<PendingUserMessage>,
    message_states: HashMap<String, MessageState>,
    item_locations: HashMap<String, ThreadItemLocation>,
    transcript_layout_revision: u64,
    transcript_layout_all_revision: u64,
    transcript_layout_targets: HashMap<TranscriptLayoutTarget, u64>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TranscriptLayoutTarget {
    Item(String),
    Notice(String),
    Approval(String),
    InputRequest(String),
    PendingUser(String),
    TurnPlan(String),
}

pub(crate) struct TranscriptLayoutChanges {
    pub revision: u64,
    pub all: bool,
    pub targets: Vec<TranscriptLayoutTarget>,
}

impl ChatState {
    pub fn new(
        id: String,
        title: SharedString,
        subtitle: SharedString,
        notices: Vec<HistoryNotice>,
    ) -> Self {
        Self {
            id,
            title,
            subtitle,
            thread: None,
            notices,
            pending_approvals: Vec::new(),
            pending_inputs: Vec::new(),
            turn_plans: HashMap::new(),
            tool_progress: HashMap::new(),
            settings: ChatSettings::default(),
            draft: String::new(),
            editing_message: None,
            active_turn: None,
            is_loading: false,
            is_creating: false,
            pending_user_message: None,
            message_states: HashMap::new(),
            item_locations: HashMap::new(),
            transcript_layout_revision: 0,
            transcript_layout_all_revision: 0,
            transcript_layout_targets: HashMap::new(),
        }
    }

    pub fn from_thread(
        thread: Thread,
        title: SharedString,
        subtitle: SharedString,
        settings: ChatSettings,
    ) -> Self {
        let id = thread.id.clone();
        let item_locations = thread_item_locations(&thread);
        let active_turn = thread
            .turns
            .iter()
            .rev()
            .find(|turn| matches!(turn.status, TurnStatus::InProgress))
            .map(|turn| ActiveTurn {
                turn_id: turn.id.clone(),
            });
        Self {
            id,
            title,
            subtitle,
            thread: Some(thread),
            notices: Vec::new(),
            pending_approvals: Vec::new(),
            pending_inputs: Vec::new(),
            turn_plans: HashMap::new(),
            tool_progress: HashMap::new(),
            settings,
            draft: String::new(),
            editing_message: None,
            active_turn,
            is_loading: false,
            is_creating: false,
            pending_user_message: None,
            message_states: HashMap::new(),
            item_locations,
            transcript_layout_revision: 0,
            transcript_layout_all_revision: 0,
            transcript_layout_targets: HashMap::new(),
        }
    }

    pub(crate) fn transcript_layout_changes_since(&self, revision: u64) -> TranscriptLayoutChanges {
        TranscriptLayoutChanges {
            revision: self.transcript_layout_revision,
            all: self.transcript_layout_all_revision > revision,
            targets: self
                .transcript_layout_targets
                .iter()
                .filter_map(|(target, changed_at)| {
                    (*changed_at > revision).then_some(target.clone())
                })
                .collect(),
        }
    }

    pub(crate) fn command_output(&self, item_id: &str) -> Option<&str> {
        let location = *self.item_locations.get(item_id)?;
        let item = self
            .thread
            .as_ref()?
            .turns
            .get(location.turn_index)?
            .items
            .get(location.item_index)?;
        let ThreadItem::CommandExecution {
            aggregated_output, ..
        } = item
        else {
            return None;
        };
        aggregated_output.as_deref()
    }

    pub(crate) fn file_changes(&self, item_id: &str) -> Option<&[FileUpdateChange]> {
        let location = *self.item_locations.get(item_id)?;
        let item = self
            .thread
            .as_ref()?
            .turns
            .get(location.turn_index)?
            .items
            .get(location.item_index)?;
        let ThreadItem::FileChange { changes, .. } = item else {
            return None;
        };
        Some(changes)
    }

    fn mark_transcript_layout(&mut self, target: TranscriptLayoutTarget) {
        self.transcript_layout_revision = self.transcript_layout_revision.wrapping_add(1);
        self.transcript_layout_targets
            .insert(target, self.transcript_layout_revision);
    }

    fn mark_all_transcript_layout(&mut self) {
        self.transcript_layout_revision = self.transcript_layout_revision.wrapping_add(1);
        self.transcript_layout_all_revision = self.transcript_layout_revision;
    }

    pub fn upsert_notice(&mut self, id: String, body: String) {
        if let Some(notice) = self.notices.iter_mut().find(|notice| notice.id == id) {
            notice.body = body.into();
        } else {
            self.notices.push(HistoryNotice {
                id: id.clone(),
                body: body.into(),
            });
        }
        self.mark_transcript_layout(TranscriptLayoutTarget::Notice(id));
    }

    pub fn remove_notice(&mut self, id: &str) {
        self.notices.retain(|notice| notice.id != id);
    }

    pub fn upsert_approval(&mut self, approval: PendingApproval) {
        let target = TranscriptLayoutTarget::Approval(approval.request_id.to_string());
        if let Some(existing) = self
            .pending_approvals
            .iter_mut()
            .find(|existing| existing.request_id == approval.request_id)
        {
            *existing = approval;
        } else {
            self.pending_approvals.push(approval);
        }
        self.mark_transcript_layout(target);
    }

    pub fn remove_approval(&mut self, request_id: &RequestId) {
        self.pending_approvals
            .retain(|approval| &approval.request_id != request_id);
    }

    pub fn upsert_input_request(&mut self, request: PendingUserInputRequest) {
        let target = TranscriptLayoutTarget::InputRequest(request.request_id.to_string());
        if let Some(existing) = self
            .pending_inputs
            .iter_mut()
            .find(|existing| existing.request_id == request.request_id)
        {
            *existing = request;
        } else {
            self.pending_inputs.push(request);
        }
        self.mark_transcript_layout(target);
    }

    pub fn answer_input_request(
        &mut self,
        request_id: &RequestId,
        question_id: String,
        answer: String,
    ) -> Option<ToolRequestUserInputResponse> {
        let request = self
            .pending_inputs
            .iter_mut()
            .find(|request| &request.request_id == request_id)?;
        request.answers.insert(question_id, vec![answer]);
        if request
            .questions
            .iter()
            .all(|question| request.answers.contains_key(&question.id))
        {
            Some(ToolRequestUserInputResponse {
                answers: request
                    .answers
                    .iter()
                    .map(|(id, answers)| {
                        (
                            id.clone(),
                            ToolRequestUserInputAnswer {
                                answers: answers.clone(),
                            },
                        )
                    })
                    .collect(),
            })
        } else {
            None
        }
    }

    pub fn remove_input_request(&mut self, request_id: &RequestId) {
        self.pending_inputs
            .retain(|request| &request.request_id != request_id);
    }

    pub fn pending_freeform_input(&self) -> Option<(RequestId, String)> {
        self.pending_inputs.iter().find_map(|request| {
            request.questions.iter().find_map(|question| {
                (!request.answers.contains_key(&question.id)
                    && question.options.as_ref().is_none_or(Vec::is_empty))
                .then(|| (request.request_id.clone(), question.id.clone()))
            })
        })
    }

    pub fn update_turn_plan(
        &mut self,
        turn_id: String,
        explanation: Option<String>,
        plan: Vec<TurnPlanStep>,
    ) {
        self.turn_plans.insert(
            turn_id.clone(),
            TurnPlanView {
                explanation: explanation.map(Into::into),
                steps: plan,
            },
        );
        self.mark_transcript_layout(TranscriptLayoutTarget::TurnPlan(turn_id));
    }

    pub fn append_tool_progress(&mut self, item_id: &str, message: String) {
        let messages = self.tool_progress.entry(item_id.to_string()).or_default();
        if messages.last().is_none_or(|last| last.as_ref() != message) {
            messages.push(message.into());
            self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id.to_string()));
        }
    }

    pub fn set_title(&mut self, title: String) {
        if let Some(thread) = &mut self.thread {
            thread.name = Some(title.clone());
        }
        self.title = single_line_title(&title).into();
    }

    pub fn set_thread_status(&mut self, status: ThreadStatus) {
        if let Some(thread) = &mut self.thread {
            thread.status = status;
        }
    }

    pub fn adopt_thread(
        &mut self,
        mut thread: Thread,
        title: SharedString,
        subtitle: SharedString,
    ) {
        if thread.turns.is_empty()
            && let Some(existing) = self.thread.as_ref()
            && !existing.turns.is_empty()
        {
            thread.turns = existing.turns.clone();
        }
        let resumed_active_turn = thread
            .turns
            .iter()
            .rev()
            .find(|turn| matches!(turn.status, TurnStatus::InProgress))
            .map(|turn| ActiveTurn {
                turn_id: turn.id.clone(),
            });
        self.id = thread.id.clone();
        self.title = title;
        self.subtitle = subtitle;
        self.thread = Some(thread);
        if resumed_active_turn.is_some() || self.active_turn.is_none() {
            self.active_turn = resumed_active_turn;
        }
        self.is_creating = false;
        self.is_loading = false;
        self.rebuild_item_locations();
        self.reconcile_pending_user_message();
        self.mark_all_transcript_layout();
    }

    pub fn start_turn(&mut self, turn_id: String) {
        self.active_turn = Some(ActiveTurn { turn_id });
    }

    pub fn finish_turn(&mut self, turn_id: Option<&str>) {
        if self
            .active_turn
            .as_ref()
            .is_some_and(|active| turn_id.is_none_or(|turn_id| active.turn_id == turn_id))
        {
            self.active_turn = None;
        }
    }

    pub fn active_turn_id(&self) -> Option<&str> {
        self.active_turn.as_ref().map(|turn| turn.turn_id.as_str())
    }

    pub fn begin_user_message(&mut self, client_id: String, text: String) -> bool {
        if self.user_message_is_sending() {
            return false;
        }
        self.pending_user_message = Some(PendingUserMessage {
            client_id: client_id.clone(),
            content: vec![UserInput::Text {
                text,
                text_elements: Vec::new(),
            }],
            delivery: PendingUserMessageDelivery::Sending,
        });
        self.mark_transcript_layout(TranscriptLayoutTarget::PendingUser(client_id));
        true
    }

    pub fn pending_user_message(&self) -> Option<&PendingUserMessage> {
        self.pending_user_message.as_ref()
    }

    pub fn pending_user_message_request(&self) -> Option<(String, String)> {
        let message = self.pending_user_message.as_ref()?;
        let text = message
            .content
            .iter()
            .filter_map(|input| match input {
                UserInput::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        Some((message.client_id.clone(), text))
    }

    pub fn user_message_is_sending(&self) -> bool {
        self.pending_user_message
            .as_ref()
            .is_some_and(|message| matches!(message.delivery, PendingUserMessageDelivery::Sending))
    }

    pub fn fail_user_message(&mut self, client_id: &str, error: String) -> bool {
        let Some(message) = self
            .pending_user_message
            .as_mut()
            .filter(|message| message.client_id == client_id)
        else {
            return false;
        };
        message.delivery = PendingUserMessageDelivery::Failed(error.into());
        self.mark_transcript_layout(TranscriptLayoutTarget::PendingUser(client_id.to_string()));
        true
    }

    pub fn upsert_turn(&mut self, turn: Turn) {
        self.acknowledge_user_message_in(&turn.items);
        if !matches!(turn.status, TurnStatus::InProgress) {
            for item in &turn.items {
                self.message_states.remove(item.id());
            }
        }
        let Some(thread) = &mut self.thread else {
            return;
        };
        if let Some(existing) = thread
            .turns
            .iter_mut()
            .find(|existing| existing.id == turn.id)
        {
            *existing = turn;
        } else {
            thread.turns.push(turn);
        }
        self.rebuild_item_locations();
        self.mark_all_transcript_layout();
    }

    pub fn complete_turn(&mut self, completed: Turn) {
        self.acknowledge_user_message_in(&completed.items);
        let completed_id = completed.id.clone();
        let item_ids = {
            let Some(thread) = &mut self.thread else {
                return;
            };
            if let Some(existing) = thread
                .turns
                .iter_mut()
                .find(|existing| existing.id == completed_id)
            {
                apply_turn_completion(existing, completed);
                existing
                    .items
                    .iter()
                    .map(|item| item.id().to_string())
                    .collect::<Vec<_>>()
            } else {
                let item_ids = completed
                    .items
                    .iter()
                    .map(|item| item.id().to_string())
                    .collect::<Vec<_>>();
                thread.turns.push(completed);
                item_ids
            }
        };
        for item_id in item_ids {
            self.message_states.remove(&item_id);
        }
        self.rebuild_item_locations();
        self.mark_all_transcript_layout();
    }

    pub fn start_item(&mut self, turn_id: &str, item: ThreadItem) {
        self.acknowledge_user_message_in(std::slice::from_ref(&item));
        let item_id = item.id().to_string();
        self.message_states
            .insert(item_id.clone(), MessageState::streaming());
        self.upsert_thread_item(turn_id, item);
        self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id));
    }

    pub fn complete_item(&mut self, turn_id: &str, item: ThreadItem) {
        self.acknowledge_user_message_in(std::slice::from_ref(&item));
        let item_id = item.id().to_string();
        self.upsert_thread_item(turn_id, item);
        if let Some(state) = self.message_states.get_mut(&item_id) {
            state.lifecycle = MessageLifecycle::Complete;
        }
        self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id));
    }

    fn upsert_thread_item(&mut self, turn_id: &str, item: ThreadItem) {
        let Some(thread) = &mut self.thread else {
            return;
        };
        let item_id = item.id().to_string();
        let mut changed = false;
        if let Some(turn) = thread.turns.iter_mut().find(|turn| turn.id == turn_id) {
            if let Some(existing) = turn
                .items
                .iter_mut()
                .find(|existing| existing.id() == item_id)
            {
                *existing = item;
                changed = true;
            } else {
                turn.items.push(item);
                changed = true;
            }
        } else if let Some(turn) = thread.turns.last_mut() {
            if let Some(existing) = turn
                .items
                .iter_mut()
                .find(|existing| existing.id() == item_id)
            {
                *existing = item;
                changed = true;
            } else {
                turn.items.push(item);
                changed = true;
            }
        }
        if changed {
            self.rebuild_item_locations();
        }
    }

    pub fn append_agent_text_delta(&mut self, item_id: &str, delta: &str) {
        let Some(ThreadItem::AgentMessage { text, .. }) = self.thread_item_mut(item_id) else {
            return;
        };
        text.push_str(delta);
    }

    pub fn append_plan_delta(&mut self, item_id: &str, delta: &str) {
        let Some(ThreadItem::Plan { text, .. }) = self.thread_item_mut(item_id) else {
            return;
        };
        text.push_str(delta);
        self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id.to_string()));
    }

    pub fn append_reasoning_summary_delta(
        &mut self,
        item_id: &str,
        summary_index: i64,
        delta: &str,
    ) {
        let Ok(summary_index) = usize::try_from(summary_index) else {
            return;
        };
        let Some(ThreadItem::Reasoning { summary, .. }) = self.thread_item_mut(item_id) else {
            return;
        };
        summary.resize_with(summary_index + 1, String::new);
        summary[summary_index].push_str(delta);
        self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id.to_string()));
    }

    pub fn add_reasoning_summary_part(&mut self, item_id: &str, summary_index: i64) {
        let Ok(summary_index) = usize::try_from(summary_index) else {
            return;
        };
        let Some(ThreadItem::Reasoning { summary, .. }) = self.thread_item_mut(item_id) else {
            return;
        };
        summary.resize_with(summary_index + 1, String::new);
        self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id.to_string()));
    }

    pub fn append_reasoning_content_delta(
        &mut self,
        item_id: &str,
        content_index: i64,
        delta: &str,
    ) {
        let Ok(content_index) = usize::try_from(content_index) else {
            return;
        };
        let Some(ThreadItem::Reasoning { content, .. }) = self.thread_item_mut(item_id) else {
            return;
        };
        content.resize_with(content_index + 1, String::new);
        content[content_index].push_str(delta);
        self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id.to_string()));
    }

    pub fn append_command_output_delta(&mut self, item_id: &str, delta: &str) {
        let Some(ThreadItem::CommandExecution {
            aggregated_output, ..
        }) = self.thread_item_mut(item_id)
        else {
            return;
        };
        aggregated_output
            .get_or_insert_with(String::new)
            .push_str(delta);
        self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id.to_string()));
    }

    pub fn update_file_change_item(&mut self, item_id: &str, changes: Vec<FileUpdateChange>) {
        let Some(ThreadItem::FileChange {
            changes: existing, ..
        }) = self.thread_item_mut(item_id)
        else {
            return;
        };
        *existing = changes;
        self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id.to_string()));
    }

    pub fn append_file_change_output_delta(&mut self, item_id: &str, delta: &str) {
        let Some(ThreadItem::FileChange { changes, .. }) = self.thread_item_mut(item_id) else {
            return;
        };
        if let Some(last_change) = changes.last_mut() {
            last_change.diff.push_str(delta);
            self.mark_transcript_layout(TranscriptLayoutTarget::Item(item_id.to_string()));
        }
    }

    pub fn item_is_streaming(&self, item_id: &str) -> bool {
        self.message_states
            .get(item_id)
            .is_some_and(MessageState::is_streaming)
    }

    fn thread_item_mut(&mut self, item_id: &str) -> Option<&mut ThreadItem> {
        let location = *self.item_locations.get(item_id)?;
        self.thread
            .as_mut()?
            .turns
            .get_mut(location.turn_index)?
            .items
            .get_mut(location.item_index)
    }

    fn rebuild_item_locations(&mut self) {
        self.item_locations = self
            .thread
            .as_ref()
            .map(thread_item_locations)
            .unwrap_or_default();
    }

    fn acknowledge_user_message_in(&mut self, items: &[ThreadItem]) {
        let Some(pending_client_id) = self
            .pending_user_message
            .as_ref()
            .map(|message| message.client_id.as_str())
        else {
            return;
        };
        let acknowledged = items.iter().any(|item| {
            matches!(
                item,
                ThreadItem::UserMessage {
                    client_id: Some(client_id),
                    ..
                } if client_id == pending_client_id
            )
        });
        if acknowledged {
            self.pending_user_message = None;
        }
    }

    fn reconcile_pending_user_message(&mut self) {
        let Some(pending_client_id) = self
            .pending_user_message
            .as_ref()
            .map(|message| message.client_id.as_str())
        else {
            return;
        };
        let acknowledged = self.thread.as_ref().is_some_and(|thread| {
            thread
                .turns
                .iter()
                .flat_map(|turn| turn.items.iter())
                .any(|item| {
                    matches!(
                        item,
                        ThreadItem::UserMessage {
                            client_id: Some(client_id),
                            ..
                        } if client_id == pending_client_id
                    )
                })
        });
        if acknowledged {
            self.pending_user_message = None;
        }
    }
}

#[derive(Clone)]
pub struct PendingUserMessage {
    pub client_id: String,
    pub content: Vec<UserInput>,
    pub delivery: PendingUserMessageDelivery,
}

#[derive(Clone)]
pub enum PendingUserMessageDelivery {
    Sending,
    Failed(SharedString),
}

#[derive(Clone)]
pub struct HistoryNotice {
    pub id: String,
    pub body: SharedString,
}

#[derive(Clone)]
pub struct TurnPlanView {
    pub explanation: Option<SharedString>,
    pub steps: Vec<TurnPlanStep>,
}

#[derive(Clone)]
pub struct PendingApproval {
    pub request_id: RequestId,
    pub title: SharedString,
    pub body: SharedString,
    pub kind: PendingApprovalKind,
}

#[derive(Clone)]
pub struct PendingUserInputRequest {
    pub request_id: RequestId,
    pub questions: Vec<ToolRequestUserInputQuestion>,
    pub answers: HashMap<String, Vec<String>>,
}

#[derive(Clone)]
pub enum PendingApprovalKind {
    Command,
    FileChange,
    Permissions {
        permissions: GrantedPermissionProfile,
    },
    McpElicitation {
        accept_content: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    },
}

impl PendingApproval {
    pub fn response(&self, approved: bool) -> serde_json::Result<serde_json::Value> {
        match &self.kind {
            PendingApprovalKind::Command => {
                serde_json::to_value(CommandExecutionRequestApprovalResponse {
                    decision: if approved {
                        CommandExecutionApprovalDecision::Accept
                    } else {
                        CommandExecutionApprovalDecision::Decline
                    },
                })
            }
            PendingApprovalKind::FileChange => {
                serde_json::to_value(FileChangeRequestApprovalResponse {
                    decision: if approved {
                        FileChangeApprovalDecision::Accept
                    } else {
                        FileChangeApprovalDecision::Decline
                    },
                })
            }
            PendingApprovalKind::Permissions { permissions } => {
                serde_json::to_value(PermissionsRequestApprovalResponse {
                    permissions: approved.then(|| permissions.clone()).unwrap_or_default(),
                    scope: PermissionGrantScope::Turn,
                    strict_auto_review: None,
                })
            }
            PendingApprovalKind::McpElicitation {
                accept_content,
                meta,
            } => serde_json::to_value(McpServerElicitationRequestResponse {
                action: if approved {
                    McpServerElicitationAction::Accept
                } else {
                    McpServerElicitationAction::Decline
                },
                content: approved.then(|| accept_content.clone()).flatten(),
                meta: meta.clone(),
            }),
        }
    }
}

/// Runtime lifecycle supplied by item notifications but absent from `ThreadItem`.
pub struct MessageState {
    lifecycle: MessageLifecycle,
}

enum MessageLifecycle {
    Streaming,
    Complete,
}

impl MessageState {
    fn streaming() -> Self {
        Self {
            lifecycle: MessageLifecycle::Streaming,
        }
    }

    fn is_streaming(&self) -> bool {
        matches!(self.lifecycle, MessageLifecycle::Streaming)
    }
}

#[derive(Clone, Copy)]
struct ThreadItemLocation {
    turn_index: usize,
    item_index: usize,
}

fn thread_item_locations(thread: &Thread) -> HashMap<String, ThreadItemLocation> {
    let mut locations = HashMap::new();
    for (turn_index, turn) in thread.turns.iter().enumerate() {
        for (item_index, item) in turn.items.iter().enumerate() {
            locations.insert(
                item.id().to_string(),
                ThreadItemLocation {
                    turn_index,
                    item_index,
                },
            );
        }
    }
    locations
}

fn apply_turn_completion(existing: &mut Turn, completed: Turn) {
    for item in completed.items {
        if let Some(existing_item) = existing
            .items
            .iter_mut()
            .find(|existing_item| existing_item.id() == item.id())
        {
            *existing_item = item;
        } else {
            existing.items.push(item);
        }
    }
    existing.status = completed.status;
    existing.error = completed.error;
    existing.started_at = completed.started_at;
    existing.completed_at = completed.completed_at;
    existing.duration_ms = completed.duration_ms;
}

pub struct WindowState {
    pub side_chat_open: bool,
    pub new_chat_open: bool,
    pub new_chat_projectless: bool,
    pub new_chat_draft: String,
}

impl WindowState {
    pub fn new() -> Self {
        Self {
            side_chat_open: false,
            new_chat_open: true,
            new_chat_projectless: false,
            new_chat_draft: String::new(),
        }
    }

    pub fn open_new_chat(&mut self, projectless: bool) {
        self.new_chat_open = true;
        self.new_chat_projectless = projectless;
    }

    pub fn close_new_chat(&mut self) {
        self.new_chat_open = false;
    }

    pub fn select_new_chat_project(&mut self) {
        self.new_chat_projectless = false;
    }

    pub fn select_new_chat_projectless(&mut self) {
        self.new_chat_projectless = true;
    }

    pub fn toggle_side_chat(&mut self) {
        self.side_chat_open = !self.side_chat_open;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveTurn {
    pub turn_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditingMessage {
    pub turn_id: String,
    pub previous_turn_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::{
        CommandExecutionRequestApprovalResponse, ToolRequestUserInputOption, TurnItemsView,
        UserInput,
    };

    #[test]
    fn canonical_user_message_acknowledges_matching_pending_message() {
        let mut chat = ChatState::new(
            "thread-1".into(),
            "Thread".into(),
            "idle".into(),
            Vec::new(),
        );
        assert!(chat.begin_user_message("client-1".into(), "hello".into()));

        chat.start_item(
            "turn-1",
            ThreadItem::UserMessage {
                id: "user-1".into(),
                client_id: Some("client-1".into()),
                content: vec![UserInput::Text {
                    text: "hello".into(),
                    text_elements: Vec::new(),
                }],
            },
        );

        assert!(chat.pending_user_message().is_none());
    }

    #[test]
    fn unrelated_user_message_does_not_acknowledge_pending_message() {
        let mut chat = ChatState::new(
            "thread-1".into(),
            "Thread".into(),
            "idle".into(),
            Vec::new(),
        );
        assert!(chat.begin_user_message("client-1".into(), "hello".into()));

        chat.start_item(
            "turn-1",
            ThreadItem::UserMessage {
                id: "user-2".into(),
                client_id: Some("client-2".into()),
                content: vec![UserInput::Text {
                    text: "other".into(),
                    text_elements: Vec::new(),
                }],
            },
        );

        assert!(chat.user_message_is_sending());
    }

    #[test]
    fn failed_pending_message_no_longer_blocks_another_submission() {
        let mut chat = ChatState::new(
            "thread-1".into(),
            "Thread".into(),
            "idle".into(),
            Vec::new(),
        );
        assert!(chat.begin_user_message("client-1".into(), "hello".into()));
        assert!(chat.fail_user_message("client-1", "offline".into()));

        assert!(!chat.user_message_is_sending());
        assert!(chat.begin_user_message("client-2".into(), "retry".into()));
    }

    #[test]
    fn turn_runtime_is_isolated_per_chat() {
        let mut first =
            ChatState::new("thread-1".into(), "First".into(), "idle".into(), Vec::new());
        let mut second = ChatState::new(
            "thread-2".into(),
            "Second".into(),
            "idle".into(),
            Vec::new(),
        );

        first.start_turn("turn-1".into());
        second.start_turn("turn-2".into());
        first.finish_turn(Some("turn-1"));

        assert!(first.active_turn.is_none());
        assert_eq!(second.active_turn_id(), Some("turn-2"));
    }

    #[test]
    fn composer_and_settings_are_isolated_per_chat() {
        let mut first =
            ChatState::new("thread-1".into(), "First".into(), "idle".into(), Vec::new());
        let second = ChatState::new(
            "thread-2".into(),
            "Second".into(),
            "idle".into(),
            Vec::new(),
        );

        first.draft = "draft for first".into();
        first.settings.model = "different-model".into();
        first.editing_message = Some(EditingMessage {
            turn_id: "turn-1".into(),
            previous_turn_id: None,
        });

        assert!(second.draft.is_empty());
        assert_ne!(first.settings.model, second.settings.model);
        assert!(second.editing_message.is_none());
    }

    #[test]
    fn turn_completion_preserves_live_items_and_updates_metadata() {
        let mut live_turn = Turn {
            id: "turn-1".into(),
            items: vec![
                ThreadItem::UserMessage {
                    id: "user-1".into(),
                    client_id: None,
                    content: vec![UserInput::Text {
                        text: "hello".into(),
                        text_elements: Vec::new(),
                    }],
                },
                ThreadItem::AgentMessage {
                    id: "progress-1".into(),
                    text: "working".into(),
                    phase: None,
                    memory_citation: None,
                },
                ThreadItem::AgentMessage {
                    id: "agent-1".into(),
                    text: "streamed response".into(),
                    phase: None,
                    memory_citation: None,
                },
            ],
            items_view: TurnItemsView::NotLoaded,
            status: TurnStatus::InProgress,
            error: None,
            started_at: Some(10),
            completed_at: None,
            duration_ms: None,
        };
        let completed_turn = Turn {
            id: "turn-1".into(),
            items: vec![ThreadItem::AgentMessage {
                id: "agent-1".into(),
                text: "canonical response".into(),
                phase: None,
                memory_citation: None,
            }],
            items_view: TurnItemsView::Summary,
            status: TurnStatus::Completed,
            error: None,
            started_at: Some(10),
            completed_at: Some(12),
            duration_ms: Some(2_000),
        };

        apply_turn_completion(&mut live_turn, completed_turn);

        assert_eq!(live_turn.items.len(), 3);
        assert!(matches!(
            &live_turn.items[0],
            ThreadItem::UserMessage { id, .. } if id == "user-1"
        ));
        assert!(matches!(
            &live_turn.items[1],
            ThreadItem::AgentMessage { id, .. } if id == "progress-1"
        ));
        assert!(matches!(
            &live_turn.items[2],
            ThreadItem::AgentMessage { id, text, .. }
                if id == "agent-1" && text == "canonical response"
        ));
        assert_eq!(live_turn.items_view, TurnItemsView::NotLoaded);
        assert_eq!(live_turn.status, TurnStatus::Completed);
        assert_eq!(live_turn.completed_at, Some(12));
        assert_eq!(live_turn.duration_ms, Some(2_000));
    }

    #[test]
    fn command_approval_encodes_allow_and_reject_decisions() {
        let approval = PendingApproval {
            request_id: RequestId::Integer(7),
            title: "Allow?".into(),
            body: "cargo check".into(),
            kind: PendingApprovalKind::Command,
        };

        let allow: CommandExecutionRequestApprovalResponse =
            serde_json::from_value(approval.response(true).unwrap()).unwrap();
        let reject: CommandExecutionRequestApprovalResponse =
            serde_json::from_value(approval.response(false).unwrap()).unwrap();

        assert_eq!(allow.decision, CommandExecutionApprovalDecision::Accept);
        assert_eq!(reject.decision, CommandExecutionApprovalDecision::Decline);
    }

    #[test]
    fn input_request_resolves_only_after_every_question_is_answered() {
        let mut chat = ChatState::new(
            "thread-1".into(),
            "Thread".into(),
            "running".into(),
            Vec::new(),
        );
        let request_id = RequestId::Integer(9);
        let question = |id: &str| ToolRequestUserInputQuestion {
            id: id.into(),
            header: id.into(),
            question: format!("Choose {id}"),
            is_other: false,
            is_secret: false,
            options: Some(vec![ToolRequestUserInputOption {
                label: "yes".into(),
                description: "Continue".into(),
            }]),
        };
        chat.upsert_input_request(PendingUserInputRequest {
            request_id: request_id.clone(),
            questions: vec![question("first"), question("second")],
            answers: HashMap::new(),
        });

        assert!(
            chat.answer_input_request(&request_id, "first".into(), "yes".into())
                .is_none()
        );
        let response = chat
            .answer_input_request(&request_id, "second".into(), "yes".into())
            .unwrap();

        assert_eq!(response.answers.len(), 2);
        assert_eq!(response.answers["first"].answers, vec!["yes"]);
        assert_eq!(response.answers["second"].answers, vec!["yes"]);
    }
}

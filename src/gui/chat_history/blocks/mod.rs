mod activity;
mod approval;
mod context;
mod input_request;
mod messages;
mod notices;
mod plan;
mod tools;
mod worked_summary;

use std::{
    collections::hash_map::DefaultHasher,
    fmt,
    hash::{Hash as _, Hasher as _},
    sync::Arc,
    time::Duration,
};

use gpui::{AnyElement, App, IntoElement, SharedString, WeakEntity, Window};
use gpui::{InteractiveElement as _, StatefulInteractiveElement as _};
use gpui_component::ActiveTheme as _;

use super::{motion::Appearing, view::ChatHistory};
use crate::gui::{PendingApproval, PendingUserInputRequest};
pub use tools::ToolGallery;
pub(super) use tools::{ToolCall, is_tool_item, tool_calls, tools_done};

#[derive(Clone)]
pub(super) enum UserMessageDelivery {
    Sent,
    Sending,
    Failed,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct BlockId(String);

impl BlockId {
    pub fn new(kind: &'static str, key: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        kind.hash(&mut hasher);
        key.hash(&mut hasher);
        Self(format!("block-{:016x}", hasher.finish()))
    }

    pub fn from_marker(value: String) -> Self {
        Self(value)
    }

    pub fn tool_group(key: &str) -> Self {
        Self::new("tools", key)
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone)]
pub(super) enum HistoryBlock {
    User {
        key: String,
        turn_id: Option<String>,
        previous_turn_id: Option<String>,
        body: SharedString,
        delivery: UserMessageDelivery,
    },
    AssistantHeader {
        key: String,
        label: &'static str,
    },
    AssistantActions {
        key: String,
        turn_id: String,
        body: SharedString,
    },
    Notice {
        key: String,
        body: SharedString,
    },
    Activity {
        key: String,
        title: SharedString,
        body: SharedString,
        running: bool,
    },
    Plan {
        key: String,
        body: SharedString,
        running: bool,
    },
    HookPrompt {
        key: String,
        body: SharedString,
    },
    Approval {
        approval: PendingApproval,
    },
    InputRequest {
        request: PendingUserInputRequest,
    },
    ToolGroup {
        key: String,
        tools: Arc<[ToolCall]>,
        expanded: bool,
        collapsible: bool,
        tail: bool,
    },
    WorkedSummary {
        turn_id: String,
        duration: Duration,
        expanded: bool,
    },
}

impl HistoryBlock {
    pub fn id(&self) -> BlockId {
        match self {
            Self::User { key, .. } => BlockId::new("user", key),
            Self::AssistantHeader { key, .. } => BlockId::new("assistant-header", key),
            Self::AssistantActions { key, .. } => BlockId::new("assistant-actions", key),
            Self::Notice { key, .. } => BlockId::new("notice", key),
            Self::Activity { key, .. } => BlockId::new("activity", key),
            Self::Plan { key, .. } => BlockId::new("plan", key),
            Self::HookPrompt { key, .. } => BlockId::new("hook-prompt", key),
            Self::Approval { approval } => {
                BlockId::new("approval", &approval.request_id.to_string())
            }
            Self::InputRequest { request } => {
                BlockId::new("input-request", &request.request_id.to_string())
            }
            Self::ToolGroup { key, .. } => BlockId::tool_group(key),
            Self::WorkedSummary { turn_id, .. } => BlockId::new("worked-summary", turn_id),
        }
    }
}

pub(super) fn render(
    history: &WeakEntity<ChatHistory>,
    block: HistoryBlock,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match block {
        HistoryBlock::User {
            key,
            turn_id,
            previous_turn_id,
            body,
            delivery,
        } => {
            let edit_history = history.clone();
            let edit_turn_id = turn_id.clone();
            let edit_body = body.clone();
            let animation = history
                .read_with(cx, |history, _| history.send_animation_launch(&key))
                .unwrap_or(None);
            let completion_history = history.clone();
            let completion_key = key.clone();
            messages::render_user(
                &key,
                body,
                delivery,
                turn_id.is_some(),
                animation,
                cx.theme(),
                move |cx| {
                    let _ = completion_history.update(cx, |history, cx| {
                        history.finish_send_animation(&completion_key, cx)
                    });
                },
                move |_, _, cx| {
                    cx.stop_propagation();
                    let Some(turn_id) = edit_turn_id.clone() else {
                        return;
                    };
                    let previous_turn_id = previous_turn_id.clone();
                    let body = edit_body.to_string();
                    let _ = edit_history.update(cx, |history, cx| {
                        history.edit_user_message(turn_id, previous_turn_id, body, cx)
                    });
                },
            )
        }
        HistoryBlock::AssistantHeader { label, .. } => {
            messages::render_assistant_header(label, cx.theme()).into_any_element()
        }
        HistoryBlock::AssistantActions { key, turn_id, body } => {
            let fork_history = history.clone();
            messages::render_assistant_actions(&key, body, move |_, _, cx| {
                cx.stop_propagation();
                let turn_id = turn_id.clone();
                let _ = fork_history.update(cx, |history, cx| history.fork_turn(turn_id, cx));
            })
        }
        HistoryBlock::Notice { key, body } => {
            let dismiss_history = history.clone();
            let notice_id = key.clone();
            notices::render(&key, body, move |cx| {
                let notice_id = notice_id.clone();
                let _ =
                    dismiss_history.update(cx, |history, cx| history.dismiss_notice(notice_id, cx));
            })
            .into_any_element()
        }
        HistoryBlock::Activity {
            title,
            body,
            running,
            ..
        } => activity::render(title, body, running, cx.theme()).into_any_element(),
        HistoryBlock::Plan { body, running, .. } => {
            plan::render(body, running, cx.theme()).into_any_element()
        }
        HistoryBlock::HookPrompt { body, .. } => {
            context::render_hook_prompt(body, cx.theme()).into_any_element()
        }
        HistoryBlock::Approval { approval } => {
            let key = approval.request_id.to_string();
            let allow_history = history.clone();
            let allow_request_id = approval.request_id.clone();
            let reject_history = history.clone();
            let reject_request_id = approval.request_id.clone();
            approval::render(
                &key,
                approval.title,
                approval.body,
                cx.theme(),
                move |cx| {
                    let request_id = allow_request_id.clone();
                    let _ = allow_history.update(cx, |history, cx| {
                        history.resolve_approval(request_id, true, cx)
                    });
                },
                move |cx| {
                    let request_id = reject_request_id.clone();
                    let _ = reject_history.update(cx, |history, cx| {
                        history.resolve_approval(request_id, false, cx)
                    });
                },
            )
            .into_any_element()
        }
        HistoryBlock::InputRequest { request } => {
            let answer_history = history.clone();
            let answer_request_id = request.request_id.clone();
            let reject_history = history.clone();
            let reject_request_id = request.request_id.clone();
            input_request::render(
                request,
                cx.theme(),
                Arc::new(move |question_id, answer, cx| {
                    let request_id = answer_request_id.clone();
                    let _ = answer_history.update(cx, |history, cx| {
                        history.answer_input(request_id, question_id, answer, cx)
                    });
                }),
                move |cx| {
                    let request_id = reject_request_id.clone();
                    let _ = reject_history
                        .update(cx, |history, cx| history.reject_input(request_id, cx));
                },
            )
            .into_any_element()
        }
        HistoryBlock::ToolGroup {
            key,
            tools,
            expanded,
            collapsible,
            tail,
        } => {
            let history = history.clone();
            let toggle_key = key.clone();
            let block_id = BlockId::tool_group(&key);
            let appeared_at = history
                .read_with(cx, |history, _| history.block_appeared_at(&block_id))
                .unwrap_or(None);
            let content = tools::render_group(
                &key,
                &tools,
                collapsible,
                tail,
                expanded,
                cx.theme(),
                move |cx| {
                    let key = toggle_key.clone();
                    let _ = history.update(cx, |history, cx| history.toggle_tools(&key, cx));
                },
            );
            Appearing::new(
                format!("tool-group-appear-{key}"),
                content,
                appeared_at,
                !cx.reduce_motion(),
            )
            .into_any_element()
        }
        HistoryBlock::WorkedSummary {
            turn_id,
            duration,
            expanded,
        } => {
            let history = history.clone();
            let element_id = format!("worked-summary-{turn_id}");
            worked_summary::render(duration, cx.theme(), expanded)
                .id(element_id)
                .on_click(move |_, _, cx| {
                    let turn_id = turn_id.clone();
                    let _ = history.update(cx, |history, cx| history.toggle_turn(&turn_id, cx));
                })
                .into_any_element()
        }
    }
}

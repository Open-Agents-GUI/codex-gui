mod collaboration;
mod command;
mod file_change;
mod gallery;
mod group;
mod media;
mod reasoning;
mod remote;
mod simple;
mod sleep;
mod web_search;

use std::{collections::HashMap, sync::Arc};

use codex_app_server_protocol::{
    CollabAgentToolCallStatus, CommandExecutionStatus, DynamicToolCallStatus, McpToolCallStatus,
    PatchApplyStatus, ThreadItem,
};
use gpui::{App, IntoElement, RenderOnce, SharedString, WeakEntity, Window};

use crate::gui::ChatState;
use collaboration::CollaborationTool;
use command::CommandTool;
use file_change::FileChangeTool;
use media::{ImageGenerationTool, ImageViewTool};
use reasoning::ReasoningTool;
use remote::{DynamicTool, McpTool};
use simple::{SimpleTool as _, SimpleToolElement, ToolStatus};
use sleep::SleepTool;
use web_search::WebSearchTool;

pub use gallery::ToolGallery;
pub(super) use group::tool_group;

#[derive(Clone, IntoElement)]
pub(in crate::gui::chat_history) enum ToolCall {
    Reasoning(ReasoningTool),
    Command(CommandTool),
    FileChange(FileChangeTool),
    Mcp(McpTool),
    Dynamic(DynamicTool),
    WebSearch(WebSearchTool),
    ImageView(ImageViewTool),
    Collaboration(CollaborationTool),
    Sleep(SleepTool),
    ImageGeneration(ImageGenerationTool),
}

impl ToolCall {
    fn new(
        item: &ThreadItem,
        progress: Option<&[SharedString]>,
        chat: WeakEntity<ChatState>,
        streaming: bool,
    ) -> Option<Self> {
        let status = tool_status(item, streaming);
        Some(match item {
            ThreadItem::Reasoning {
                summary, content, ..
            } => Self::Reasoning(ReasoningTool::new(summary, content, status)),
            ThreadItem::CommandExecution { .. } => {
                Self::Command(CommandTool::new(item, status, progress, chat)?)
            }
            ThreadItem::FileChange { .. } => {
                Self::FileChange(FileChangeTool::new(item, status, progress, chat)?)
            }
            ThreadItem::McpToolCall { .. } => Self::Mcp(McpTool::new(item, status, progress)?),
            ThreadItem::DynamicToolCall { .. } => {
                Self::Dynamic(DynamicTool::new(item, status, progress)?)
            }
            ThreadItem::WebSearch(_) => Self::WebSearch(WebSearchTool::new(item, status)?),
            ThreadItem::ImageView { .. } => Self::ImageView(ImageViewTool::new(item, status)?),
            ThreadItem::CollabAgentToolCall { .. } => {
                Self::Collaboration(CollaborationTool::new(item, status, progress)?)
            }
            ThreadItem::Sleep(_) => Self::Sleep(SleepTool::new(item, status)?),
            ThreadItem::ImageGeneration(_) => {
                Self::ImageGeneration(ImageGenerationTool::new(item, status, progress)?)
            }
            _ => return None,
        })
    }

    fn status(&self) -> ToolStatus {
        match self {
            Self::Reasoning(reasoning) => reasoning.status(),
            Self::Command(tool) => tool.status(),
            Self::FileChange(tool) => tool.status(),
            Self::Mcp(tool) => tool.status(),
            Self::Dynamic(tool) => tool.status(),
            Self::WebSearch(tool) => tool.status(),
            Self::ImageView(tool) => tool.status(),
            Self::Collaboration(tool) => tool.status(),
            Self::Sleep(tool) => tool.status(),
            Self::ImageGeneration(tool) => tool.status(),
        }
    }

    fn is_reasoning(&self) -> bool {
        matches!(self, Self::Reasoning(_))
    }
}

impl RenderOnce for ToolCall {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match self {
            Self::Reasoning(reasoning) => SimpleToolElement::new(reasoning).into_any_element(),
            Self::Command(tool) => tool.into_any_element(),
            Self::FileChange(tool) => tool.into_any_element(),
            Self::Mcp(tool) => SimpleToolElement::new(tool).into_any_element(),
            Self::Dynamic(tool) => SimpleToolElement::new(tool).into_any_element(),
            Self::WebSearch(tool) => SimpleToolElement::new(tool).into_any_element(),
            Self::ImageView(tool) => SimpleToolElement::new(tool).into_any_element(),
            Self::Collaboration(tool) => SimpleToolElement::new(tool).into_any_element(),
            Self::Sleep(tool) => SimpleToolElement::new(tool).into_any_element(),
            Self::ImageGeneration(tool) => tool.into_any_element(),
        }
    }
}

pub(in crate::gui::chat_history) fn tool_calls(
    tools: &[&ThreadItem],
    progress: &HashMap<String, Vec<SharedString>>,
    chat: WeakEntity<ChatState>,
    is_streaming: impl Fn(&str) -> bool,
) -> Arc<[ToolCall]> {
    tools
        .iter()
        .filter_map(|tool| {
            ToolCall::new(
                tool,
                progress.get(tool.id()).map(Vec::as_slice),
                chat.clone(),
                is_streaming(tool.id()),
            )
        })
        .collect()
}

pub(in crate::gui::chat_history) fn is_tool_item(item: &ThreadItem) -> bool {
    matches!(
        item,
        ThreadItem::CommandExecution { .. }
            | ThreadItem::FileChange { .. }
            | ThreadItem::McpToolCall { .. }
            | ThreadItem::DynamicToolCall { .. }
            | ThreadItem::WebSearch(_)
            | ThreadItem::ImageView { .. }
            | ThreadItem::CollabAgentToolCall { .. }
            | ThreadItem::Sleep(_)
            | ThreadItem::ImageGeneration(_)
    )
}

pub(in crate::gui::chat_history) fn tools_done(
    tools: &[&ThreadItem],
    is_streaming: impl Fn(&str) -> bool,
) -> bool {
    !tools.is_empty()
        && tools
            .iter()
            .all(|tool| tool_status(tool, is_streaming(tool.id())).done())
}

fn tool_status(item: &ThreadItem, streaming: bool) -> ToolStatus {
    match item {
        ThreadItem::Reasoning { .. } => {
            if streaming {
                ToolStatus::Running
            } else {
                ToolStatus::Succeeded
            }
        }
        ThreadItem::CommandExecution { status, .. } => match status {
            CommandExecutionStatus::InProgress => ToolStatus::Running,
            CommandExecutionStatus::Completed => ToolStatus::Succeeded,
            CommandExecutionStatus::Failed | CommandExecutionStatus::Declined => ToolStatus::Failed,
        },
        ThreadItem::FileChange { status, .. } => match status {
            PatchApplyStatus::InProgress => ToolStatus::Running,
            PatchApplyStatus::Completed => ToolStatus::Succeeded,
            PatchApplyStatus::Failed | PatchApplyStatus::Declined => ToolStatus::Failed,
        },
        ThreadItem::McpToolCall { status, .. } => match status {
            McpToolCallStatus::InProgress => ToolStatus::Running,
            McpToolCallStatus::Completed => ToolStatus::Succeeded,
            McpToolCallStatus::Failed => ToolStatus::Failed,
        },
        ThreadItem::DynamicToolCall {
            status, success, ..
        } => match status {
            DynamicToolCallStatus::InProgress => ToolStatus::Running,
            DynamicToolCallStatus::Completed if success != &Some(false) => ToolStatus::Succeeded,
            DynamicToolCallStatus::Completed | DynamicToolCallStatus::Failed => ToolStatus::Failed,
        },
        ThreadItem::WebSearch(_) | ThreadItem::ImageView { .. } if streaming => ToolStatus::Running,
        ThreadItem::WebSearch(_) | ThreadItem::ImageView { .. } => ToolStatus::Succeeded,
        ThreadItem::CollabAgentToolCall { status, .. } => match status {
            CollabAgentToolCallStatus::InProgress => ToolStatus::Running,
            CollabAgentToolCallStatus::Completed => ToolStatus::Succeeded,
            CollabAgentToolCallStatus::Failed => ToolStatus::Failed,
        },
        ThreadItem::Sleep(_) if streaming => ToolStatus::Running,
        ThreadItem::Sleep(_) => ToolStatus::Succeeded,
        ThreadItem::ImageGeneration(item)
            if streaming || item.status.eq_ignore_ascii_case("in_progress") =>
        {
            ToolStatus::Running
        }
        ThreadItem::ImageGeneration(item)
            if item.status.eq_ignore_ascii_case("failed")
                || item.status.eq_ignore_ascii_case("error") =>
        {
            ToolStatus::Failed
        }
        ThreadItem::ImageGeneration(_) => ToolStatus::Succeeded,
        _ => ToolStatus::Failed,
    }
}

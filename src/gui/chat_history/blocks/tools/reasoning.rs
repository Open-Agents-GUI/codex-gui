// Reasoning is not a tool: it is the model's own thinking, not an action it takes.
// It lives in this module only because it is surfaced through the same
// `ToolCall`/`SimpleToolElement` presentation and grouped alongside real tool calls.

use gpui::SharedString;
use gpui_component::IconName;

use super::simple::{SimpleTool, ToolStatus};

#[derive(Clone)]
pub(in crate::gui::chat_history) struct ReasoningTool {
    title: SharedString,
    detail: Option<SharedString>,
    status: ToolStatus,
}

impl ReasoningTool {
    pub(super) fn new(summary: &[String], content: &[String], status: ToolStatus) -> Self {
        let body = summary
            .iter()
            .chain(content)
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n\n")
            .replace("**", "");
        let last_line = body
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty());
        Self {
            title: last_line
                .map(|line| format!("Reasoning: {line}"))
                .unwrap_or_else(|| "Reasoning".to_string())
                .into(),
            detail: (!body.is_empty()).then(|| body.into()),
            status,
        }
    }

    pub(super) fn status(&self) -> ToolStatus {
        self.status
    }
}

impl SimpleTool for ReasoningTool {
    fn icon(&self) -> IconName {
        IconName::Bot
    }

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn detail(&self) -> Option<SharedString> {
        self.detail.clone()
    }

    fn status(&self) -> ToolStatus {
        self.status
    }
}

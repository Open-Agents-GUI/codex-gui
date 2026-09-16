// Reasoning is not a tool: it is the model's own thinking, not an action it takes.
// It lives in this module only because it is surfaced through the same
// `ToolCall`/`SimpleToolElement` presentation and grouped alongside real tool calls.

use gpui::SharedString;
use gpui_component::IconName;

use super::simple::{DetailStyle, SimpleTool, ToolStatus};

#[derive(Clone)]
pub(in crate::gui::chat_history) struct ReasoningTool {
    title: SharedString,
    detail: Option<SharedString>,
    status: ToolStatus,
}

impl ReasoningTool {
    pub(super) fn new(summary: &[String], content: &[String], status: ToolStatus) -> Self {
        let raw = summary
            .iter()
            .chain(content)
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n\n");
        let body = raw.replace("**", "");
        let title = bold_title(&raw)
            .map(|title| format!("Reasoning: {title}"))
            .unwrap_or_else(|| "Reasoning".to_string());
        Self {
            title: title.into(),
            detail: (!body.trim().is_empty()).then(|| body.trim().to_string().into()),
            status,
        }
    }

    pub(super) fn status(&self) -> ToolStatus {
        self.status
    }
}

/// Extracts a title from a trailing Markdown bold segment, which may span
/// multiple lines. Returns `None` when the text does not end with a
/// line-initial `**...**` segment — e.g. raw thinking without bold headers —
/// so callers fall back to a plain "Reasoning" label.
fn bold_title(text: &str) -> Option<String> {
    let without_close = text.trim_end().strip_suffix("**")?;
    let opener = without_close.rfind("**")? + 2;
    // The opening `**` must start its own line; otherwise the trailing `**`
    // is inline emphasis or an unbalanced marker, not a title.
    let before_opener = &without_close[..opener - 2];
    let line_start = before_opener.rfind('\n').map_or(0, |idx| idx + 1);
    if !before_opener[line_start..].trim().is_empty() {
        return None;
    }
    let title = without_close[opener..]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!title.is_empty()).then_some(title)
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

    fn detail_style(&self) -> DetailStyle {
        DetailStyle::Prose
    }
}

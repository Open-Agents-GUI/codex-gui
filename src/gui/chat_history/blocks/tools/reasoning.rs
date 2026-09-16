// Reasoning is not a tool: it is the model's own thinking, not an action it takes.
// It lives in this module only because it is surfaced through the same
// `ToolCall`/`ToolFrame` presentation and grouped alongside real tool calls.

use gpui::{App, IntoElement, Pixels, RenderOnce, SharedString, Styled, Window, px};
use gpui_component::{IconName, text::TextView};

use super::simple::{DetailStyle, ToolFrame, ToolStatus};

/// Fixed height of one reasoning block. Longer thinking scrolls inside the
/// block instead of pushing the rest of the transcript off screen.
const REASONING_HEIGHT: Pixels = px(200.);

#[derive(Clone, IntoElement)]
pub(in crate::gui::chat_history) struct ReasoningTool {
    item_id: String,
    title: SharedString,
    body: Option<SharedString>,
    status: ToolStatus,
}

impl ReasoningTool {
    pub(super) fn new(
        item_id: &str,
        summary: &[String],
        content: &[String],
        status: ToolStatus,
    ) -> Self {
        let raw = summary
            .iter()
            .chain(content)
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n\n");
        let title = bold_title(&raw)
            .map(|title| format!("Reasoning: {title}"))
            .unwrap_or_else(|| "Reasoning".to_string());
        Self {
            item_id: item_id.to_string(),
            title: title.into(),
            body: (!raw.trim().is_empty()).then(|| raw.trim().to_string().into()),
            status,
        }
    }

    pub(super) fn status(&self) -> ToolStatus {
        self.status
    }
}

impl RenderOnce for ReasoningTool {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let frame = ToolFrame::new(IconName::Bot, self.title, None, self.status)
            .detail_style(DetailStyle::Prose);
        match self.body {
            // A scrollable `TextView` virtualizes its blocks and owns its
            // scrollbar, so a fixed height is all the block needs.
            Some(body) => frame.custom_detail(
                TextView::markdown(format!("reasoning-{}", self.item_id), body)
                    .scrollable(true)
                    .h(REASONING_HEIGHT)
                    .text_sm()
                    .line_height(px(20.)),
            ),
            None => frame,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_trailing_bold_title() {
        assert_eq!(bold_title("thinking\n\n**Plan**").as_deref(), Some("Plan"));
        assert_eq!(bold_title("no title here"), None);
    }
}

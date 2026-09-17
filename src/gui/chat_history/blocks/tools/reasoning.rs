// Reasoning is not a tool: it is the model's own thinking, not an action it takes.
// It lives in this module only because it is surfaced through the same
// `ToolCall`/`ToolFrame` presentation and grouped alongside real tool calls.

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FollowMode, IntoElement, Pixels,
    SharedString, Styled, Window, px,
};
use gpui_component::{
    IconName,
    text::{TextView, TextViewState},
};

use super::simple::{DetailStyle, ToolFrame, ToolRow, ToolStatus};

/// Fixed height of one reasoning block. Longer thinking scrolls inside the
/// block instead of pushing the rest of the transcript off screen.
const REASONING_HEIGHT: Pixels = px(200.);

#[derive(Clone)]
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

impl ReasoningTool {
    pub(super) fn render_row(self, row: ToolRow, window: &mut Window, cx: &mut App) -> AnyElement {
        let frame = ToolFrame::new(IconName::Bot, self.title, None, self.status)
            .detail_style(DetailStyle::Prose)
            .row(row);
        match self.body {
            // A scrollable `TextView` virtualizes its blocks and owns its
            // scrollbar, so a fixed height is all the block needs. The state is
            // keyed by item id, survives the per-delta transcript rebuilds, and
            // follows the tail so streamed thinking stays pinned to its end.
            Some(body) => {
                let text: Entity<ReasoningText> = window.use_keyed_state(
                    format!("reasoning-text-{}", self.item_id),
                    cx,
                    |_, cx| {
                        let state = cx.new(|cx| {
                            let mut state = TextViewState::markdown("", cx);
                            state.set_follow_mode(FollowMode::Tail, cx);
                            state
                        });
                        ReasoningText {
                            state,
                            fed: SharedString::default(),
                        }
                    },
                );
                text.update(cx, |text, cx| text.feed(&body, cx));
                let state = text.read(cx).state.clone();
                frame
                    .custom_detail(
                        TextView::new(&state)
                            .scrollable(true)
                            .h(REASONING_HEIGHT)
                            .text_sm()
                            .line_height(px(20.)),
                    )
                    .into_any_element()
            }
            None => frame.into_any_element(),
        }
    }
}

/// The rendered Markdown of one reasoning item, plus the prefix of the body that
/// has already been handed to it.
///
/// The app server streams thinking as an append-only sequence, so the view feeds
/// the Markdown state with `push_str` deltas. Replacing the whole body with
/// `set_text` on every rendered frame re-parses the entire document — the parser
/// can never catch up with a fast stream, and the UI thread stalls until the
/// stream ends. Tracking the fed prefix also lets a rewritten body (or an id
/// reused by another thread) fall back to a full replace.
struct ReasoningText {
    state: Entity<TextViewState>,
    fed: SharedString,
}

impl ReasoningText {
    fn feed(&mut self, body: &SharedString, cx: &mut Context<Self>) {
        if self.fed == *body {
            return;
        }
        if !self.fed.is_empty() && body.starts_with(self.fed.as_str()) {
            let delta = &body[self.fed.len()..];
            self.state.update(cx, |state, cx| state.push_str(delta, cx));
        } else {
            self.state.update(cx, |state, cx| state.set_text(body, cx));
        }
        self.fed = body.clone();
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

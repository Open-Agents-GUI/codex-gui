use std::sync::Arc;

use gpui::{
    App, Axis, Context, Entity, FollowMode, IntoElement, ListAlignment, ListSizingBehavior,
    ListState, ParentElement, Pixels, Render, Styled, Window, div, list, prelude::*, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    accordion::Accordion,
    h_flex,
    scroll::{ScrollableElement as _, ScrollableMask},
    theme::Theme,
};

use super::{ToolCall, simple::ToolStatus};

/// Hard cap on the height of one expanded tool group.
const TOOL_GROUP_MAX_HEIGHT: Pixels = px(600.);
/// Fraction of the window height an expanded tool group may occupy. The group
/// scrolls internally past this height, so a long run of tool calls cannot push
/// the rest of the transcript off screen.
const TOOL_GROUP_MAX_VIEWPORT_FRACTION: f32 = 0.7;
/// Extra rows rendered above and below the visible window of a tool group, so
/// scrolling does not flash empty rows.
const TOOL_GROUP_OVERDRAW: Pixels = px(240.);
/// Initial height assumed for an unmeasured tool row. Only used to estimate the
/// group's content height (and therefore its capped viewport) until real rows are
/// measured; measured heights replace it as rows become visible.
const TOOL_GROUP_ROW_HINT: Pixels = px(48.);

/// The tallest an expanded tool group may grow in this window.
fn group_max_height(window: &Window) -> Pixels {
    (window.viewport_size().height * TOOL_GROUP_MAX_VIEWPORT_FRACTION).min(TOOL_GROUP_MAX_HEIGHT)
}

/// One tool group in the transcript.
///
/// Owns the accordion, its summary, and the virtualized rows inside it, so the
/// group's scroll state lives with the group instead of in the transcript. Like
/// the command and file-change output lists, the group is a keyed window state
/// created on first render and reused afterwards.
pub(in crate::gui::chat_history) struct ToolGroup {
    key: String,
    tools: Arc<[ToolCall]>,
    collapsible: bool,
    tail: bool,
    expanded: bool,
    on_toggle: Arc<dyn Fn(&mut App) + Send + Sync>,
    list_state: ListState,
}

impl ToolGroup {
    fn new(key: String) -> Self {
        Self {
            key,
            tools: Arc::default(),
            collapsible: true,
            tail: false,
            expanded: false,
            on_toggle: Arc::new(|_| {}),
            list_state: ListState::new(0, ListAlignment::Top, TOOL_GROUP_OVERDRAW),
        }
    }

    /// Reconcile the group with the current projection.
    fn configure(
        &mut self,
        tools: Arc<[ToolCall]>,
        collapsible: bool,
        tail: bool,
        expanded: bool,
        on_toggle: Arc<dyn Fn(&mut App) + Send + Sync>,
    ) {
        self.sync_rows(&tools);
        self.tools = tools;
        self.collapsible = collapsible;
        self.expanded = expanded;
        self.on_toggle = on_toggle;

        if tail != self.tail {
            self.tail = tail;
            self.list_state.set_follow_mode(if tail {
                FollowMode::Tail
            } else {
                FollowMode::Normal
            });
        }
    }

    /// Reconcile the projected tool calls with the list's item count.
    ///
    /// Tool groups only ever grow at the end as the turn streams, so an append
    /// is spliced in place; any other count change resets the list.
    fn sync_rows(&self, tools: &[ToolCall]) {
        if tools.len() == self.tools.len() {
            return;
        }
        if tools.len() > self.tools.len() {
            self.list_state.splice(
                self.tools.len()..self.tools.len(),
                tools.len() - self.tools.len(),
            );
        } else {
            self.list_state.reset(tools.len());
        }
        // Re-seed uniform hints so the viewport height and scrollbar are
        // estimated before the new rows have been measured. The builder consumes
        // and returns the handle, but mutates the shared list state in place.
        self.list_state
            .clone()
            .with_uniform_item_height(TOOL_GROUP_ROW_HINT);
    }

    /// A streaming group is always open; a finished one can be folded away.
    fn can_toggle(&self) -> bool {
        self.collapsible && !self.tail
    }

    /// The virtualized rows, capped so the group cannot grow without bound.
    fn render_rows(&self, window: &Window, cx: &App) -> gpui::Div {
        let border = cx.theme().border.opacity(0.55);
        let rows = self.tools.clone();
        let list_state = self.list_state.clone();
        div()
            .relative()
            .w_full()
            .min_w_0()
            .child(
                list(list_state.clone(), move |index, _, _| {
                    div()
                        .w_full()
                        .min_w_0()
                        .when(index > 0, |row| row.border_t_1().border_color(border))
                        .children(rows.get(index).cloned())
                        .into_any_element()
                })
                // `Infer` shrinks the list to its rows; `max_h` then caps it. The
                // default `Auto` instead expects the parent to hand down a definite
                // size, which would leave the group with no height at all.
                .with_sizing_behavior(ListSizingBehavior::Infer)
                .w_full()
                .min_w_0()
                .max_h(group_max_height(window)),
            )
            .vertical_scrollbar(&list_state)
            // The mask consumes the wheel while the group still has room to
            // scroll and hands it back to the transcript at either edge.
            .child(ScrollableMask::new(Axis::Vertical, &list_state))
    }
}

impl Render for ToolGroup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tools.is_empty() {
            return div();
        }

        let theme = cx.theme();
        let can_toggle = self.can_toggle();
        let on_toggle = self.on_toggle.clone();
        let title_style = gpui::StyleRefinement::default().px_3().py_2();
        let content_style = gpui::StyleRefinement::default().px_2().pb_2();
        let accordion = Accordion::new(format!("tool-group-accordion-{}", self.key))
            .bordered(false)
            .xsmall()
            .w_full()
            .min_w_0()
            .border_1()
            .border_color(theme.border.opacity(0.75))
            .bg(theme.muted.opacity(0.35))
            .rounded_lg()
            .overflow_hidden()
            .item(|item| {
                item.open(!can_toggle || self.expanded)
                    .disabled(!can_toggle)
                    .title(render_summary(&self.tools, theme))
                    .title_style(title_style)
                    .content_style(content_style)
                    .hover(|style| style.bg(theme.accent.opacity(0.45)))
                    .bg(theme.transparent)
                    .child(self.render_rows(window, cx))
            })
            .when(can_toggle, |accordion| {
                accordion.on_toggle_click(move |_, _, cx| on_toggle(cx))
            });

        div()
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .py_2()
            .child(accordion)
    }
}

/// The tool group for `key`, created on first render and reused afterwards.
pub(in crate::gui::chat_history) fn tool_group(
    key: &str,
    tools: Arc<[ToolCall]>,
    collapsible: bool,
    tail: bool,
    expanded: bool,
    window: &mut Window,
    cx: &mut App,
    on_toggle: impl Fn(&mut App) + Send + Sync + 'static,
) -> Entity<ToolGroup> {
    let group = window.use_keyed_state(format!("tool-group-{key}"), cx, |_, _| {
        ToolGroup::new(key.to_string())
    });
    group.update(cx, |group, _| {
        group.configure(tools, collapsible, tail, expanded, Arc::new(on_toggle))
    });
    group
}

fn render_summary(tools: &[ToolCall], theme: &Theme) -> gpui::Div {
    let tool_count = tools.iter().filter(|tool| !tool.is_reasoning()).count();
    let running = tools
        .iter()
        .filter(|tool| !tool.is_reasoning())
        .filter(|tool| matches!(tool.status(), ToolStatus::Running))
        .count();
    let failed = tools
        .iter()
        .filter(|tool| !tool.is_reasoning())
        .filter(|tool| matches!(tool.status(), ToolStatus::Failed))
        .count();
    h_flex()
        .min_w_0()
        .items_center()
        .gap_2()
        .text_sm()
        .child(
            div()
                .size_6()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(theme.accent.opacity(0.7))
                .child(
                    Icon::new(IconName::Asterisk)
                        .xsmall()
                        .text_color(theme.accent_foreground),
                ),
        )
        .child(div().min_w_0().flex_1().truncate().child(if running > 0 {
            format!(
                "Running {tool_count} {}",
                pluralize(tool_count, "tool call")
            )
        } else {
            format!("Ran {tool_count} {}", pluralize(tool_count, "tool call"))
        }))
        .when(failed > 0, |summary| {
            summary.child(
                div()
                    .flex_none()
                    .rounded_full()
                    .bg(theme.danger.opacity(0.12))
                    .px_1p5()
                    .py_0p5()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(format!("{failed} failed")),
            )
        })
        .when(running > 0, |summary| {
            summary.child(
                div()
                    .flex_none()
                    .rounded_full()
                    .bg(theme.warning.opacity(0.14))
                    .px_1p5()
                    .py_0p5()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(format!("{running} active")),
            )
        })
}

fn pluralize(count: usize, singular: &'static str) -> &'static str {
    if count == 1 { singular } else { "tool calls" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::ThreadItem;
    use gpui::{TestAppContext, VisualTestContext, WeakEntity};

    /// Renders one expanded tool group and keeps the entity so the test can
    /// inspect the rows list after layout.
    struct GroupProbe {
        tools: Arc<[ToolCall]>,
        group: Option<Entity<ToolGroup>>,
    }

    impl Render for GroupProbe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let group = tool_group(
                "probe",
                self.tools.clone(),
                true,
                false,
                true,
                window,
                cx,
                |_| {},
            );
            self.group = Some(group.clone());
            div().w(px(800.)).child(group)
        }
    }

    fn reasoning_tools(count: usize) -> Arc<[ToolCall]> {
        (0..count)
            .map(|index| {
                ToolCall::new(
                    &ThreadItem::Reasoning {
                        id: format!("reasoning-{index}"),
                        summary: vec![format!("Step {index}")],
                        content: Vec::new(),
                    },
                    None,
                    WeakEntity::new_invalid(),
                    false,
                )
                .expect("reasoning items always produce a tool call")
            })
            .collect()
    }

    /// Lays out one group and reports the rows area's height plus the cap it
    /// should respect.
    fn rows_height(cx: &mut TestAppContext, tool_count: usize) -> (Pixels, Pixels) {
        cx.update(gpui_component::init);
        let (probe, cx) = cx.add_window_view(|_, _| GroupProbe {
            tools: reasoning_tools(tool_count),
            group: None,
        });
        let cx: &mut VisualTestContext = cx;
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let (group, cap) = cx.update(|window, cx| {
            (
                probe.read(cx).group.clone().expect("group rendered"),
                group_max_height(window),
            )
        });
        let rows = cx.update(|_, cx| group.read(cx).list_state.viewport_bounds());
        (rows.size.height, cap)
    }

    /// `gpui::list` defaults to `ListSizingBehavior::Auto`, which lays out no
    /// children and leaves the group collapsed-looking unless `Infer` is asked
    /// for. A long group must still take a real, capped height.
    #[gpui::test]
    fn long_group_fills_the_cap(cx: &mut TestAppContext) {
        let (height, cap) = rows_height(cx, 67);

        assert!(
            height <= cap && height > cap - px(2.),
            "expected the rows area to fill the {cap} cap, got {height}"
        );
    }

    /// A short group shrinks to its rows instead of always taking the cap.
    #[gpui::test]
    fn short_group_shrinks_to_its_rows(cx: &mut TestAppContext) {
        let (height, cap) = rows_height(cx, 2);

        assert!(height > px(0.), "the rows area collapsed to {height}");
        assert!(
            height < cap,
            "expected the rows area to shrink below the {cap} cap, got {height}"
        );
    }
}

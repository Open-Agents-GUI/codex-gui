use std::sync::Arc;

use gpui::{
    AnyElement, App, Axis, Context, Entity, FollowMode, Hsla, InteractiveElement as _,
    IntoElement, ListAlignment, ListState, ParentElement, Pixels, Render, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Window, div, list, prelude::*, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    accordion::Accordion,
    h_flex,
    scroll::{ScrollableElement as _, ScrollableMask},
    theme::Theme,
    v_flex,
};

use super::{
    ToolCall,
    simple::{ToolRow, ToolStatus},
};

/// Hard cap on the height of one expanded tool group.
const TOOL_GROUP_MAX_HEIGHT: Pixels = px(600.);
/// Fraction of the window height an expanded tool group may occupy. The group
/// scrolls internally past this height, so a long run of tool calls cannot push
/// the rest of the transcript off screen.
const TOOL_GROUP_MAX_VIEWPORT_FRACTION: f32 = 0.7;
/// Extra rows rendered above and below the visible window of a tool group, so
/// scrolling does not flash empty rows.
const TOOL_GROUP_OVERDRAW: Pixels = px(240.);
/// Conservative lower bound on one collapsed tool row's height.
///
/// A collapsed row is a `size_7` icon plus `py_2`, so 32px is a safe
/// under-estimate. When even this minimum total exceeds the cap, the group is
/// guaranteed to overflow and can be virtualized at a definite capped height;
/// otherwise it is rendered in full so its height is known during layout.
const MIN_TOOL_ROW_HEIGHT: Pixels = px(32.);

/// The tallest an expanded tool group may grow in this window.
fn group_max_height(window: &Window) -> Pixels {
    (window.viewport_size().height * TOOL_GROUP_MAX_VIEWPORT_FRACTION).min(TOOL_GROUP_MAX_HEIGHT)
}

/// One tool group in the transcript.
///
/// Owns its header, rows, and scroll state, so the group's scroll position
/// lives with the group instead of in the transcript. The group is a keyed
/// window state created on first render and reused afterwards.
///
/// Short groups render every row, so their height is resolved by the layout
/// pass in the same frame it changes. Only groups whose minimum possible
/// content already exceeds the cap are virtualized, and those can use a
/// definite capped height without a measurement pass. This avoids feeding a
/// post-layout measurement back into the transcript, which would make the
/// transcript's cached block height lag a frame and jump the scroll.
pub(in crate::gui::chat_history) struct ToolGroup {
    key: String,
    tools: Arc<[ToolCall]>,
    row_ids: Arc<[String]>,
    expanded_rows: Arc<[bool]>,
    collapsible: bool,
    tail: bool,
    expanded: bool,
    on_toggle: Arc<dyn Fn(&mut App) + Send + Sync>,
    on_row_toggle: Arc<dyn Fn(&str, &mut App) + Send + Sync>,
    /// Scroll state for the non-virtualized rows of a short group.
    scroll_handle: ScrollHandle,
    /// Virtualized rows for a group whose content certainly exceeds the cap.
    list_state: ListState,
}

impl ToolGroup {
    fn new(key: String) -> Self {
        Self {
            key,
            tools: Arc::default(),
            row_ids: Arc::default(),
            expanded_rows: Arc::default(),
            collapsible: true,
            tail: false,
            expanded: false,
            on_toggle: Arc::new(|_| {}),
            on_row_toggle: Arc::new(|_, _| {}),
            scroll_handle: ScrollHandle::new(),
            // Rows range from a collapsed header to a tall expanded detail, so
            // the list cannot estimate unmeasured rows. Measuring all of them
            // keeps `content_size` exact, which is what lets the scroll mask
            // detect the true bottom and hand the wheel back to the transcript.
            list_state: ListState::new(0, ListAlignment::Top, TOOL_GROUP_OVERDRAW).measure_all(),
        }
    }

    /// Reconcile the group with the current projection.
    #[allow(clippy::too_many_arguments)]
    fn configure(
        &mut self,
        tools: Arc<[ToolCall]>,
        row_ids: Arc<[String]>,
        expanded_rows: Arc<[bool]>,
        collapsible: bool,
        tail: bool,
        expanded: bool,
        on_toggle: Arc<dyn Fn(&mut App) + Send + Sync>,
        on_row_toggle: Arc<dyn Fn(&str, &mut App) + Send + Sync>,
    ) {
        // The same rows can change height when their disclosure state or
        // identity changes. The list only re-measures rows near the viewport,
        // so off-screen rows would keep a stale (or zero, if never measured)
        // height; `summary().height` then stops short of the real content and
        // the scroll mask hands the wheel to the transcript before the group's
        // true bottom. Invalidate every row so the next layout measures them
        // all before the mask reads the content size.
        let rows_changed = self.row_ids.len() == row_ids.len()
            && (self.row_ids != row_ids || self.expanded_rows != expanded_rows);

        self.sync_rows(&tools);
        self.tools = tools;
        self.row_ids = row_ids;
        self.expanded_rows = expanded_rows;
        self.collapsible = collapsible;
        self.expanded = expanded;
        self.on_toggle = on_toggle;
        self.on_row_toggle = on_row_toggle;

        if rows_changed {
            // `measure_all` only measures rows that are still unmeasured, so it
            // cannot refresh already-cached rows. Invalidate every row so the
            // next layout re-renders and re-measures them all.
            self.list_state
                .remeasure_items(0..self.list_state.item_count());
        }

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
        // New rows are unmeasured; ask for an exact measure of all of them so
        // the scroll mask's bottom detection stays correct. The builder consumes
        // and returns the handle, but mutates the shared list state in place.
        self.list_state.clone().measure_all();
    }

    /// The accordion is forced open while this group's reasoning is still
    /// streaming, so live thinking stays visible. Otherwise it is foldable and
    /// starts collapsed: tool calls are one-line cards until the user clicks.
    fn can_toggle(&self) -> bool {
        self.collapsible && !self.reasoning_is_streaming()
    }

    /// Whether any reasoning item in this group is currently being streamed.
    fn reasoning_is_streaming(&self) -> bool {
        self.tools
            .iter()
            .any(|tool| tool.is_reasoning() && matches!(tool.status(), ToolStatus::Running))
    }

    /// The rows below the header, sized so the group never grows past its cap.
    fn render_rows(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let cap = group_max_height(window);
        if self.certainly_over_cap(cap) {
            self.render_virtualized_rows(cap, cx)
        } else {
            self.render_natural_rows(cap, window, cx)
        }
    }

    /// Whether the group's content is guaranteed to exceed `cap`.
    ///
    /// Uses a lower bound on a collapsed row, so a `true` answer is always
    /// correct: the rows cannot fit and the group will scroll.
    fn certainly_over_cap(&self, cap: Pixels) -> bool {
        MIN_TOOL_ROW_HEIGHT * self.tools.len() as f32 >= cap
    }

    /// A short group, rendered in full so its height is resolved by layout in
    /// the same frame its rows change.
    fn render_natural_rows(&self, cap: Pixels, window: &mut Window, cx: &mut App) -> AnyElement {
        let border = cx.theme().border.opacity(0.55);
        let mut rows = Vec::with_capacity(self.tools.len());
        for index in 0..self.tools.len() {
            rows.push(tool_row_element(
                index,
                self.tools[index].clone(),
                self.row_ids.get(index).cloned().unwrap_or_default(),
                self.expanded_rows.get(index).copied().unwrap_or(false),
                self.on_row_toggle.clone(),
                border,
                window,
                cx,
            ));
        }

        div()
            .relative()
            .w_full()
            .min_w_0()
            .debug_selector({
                let key = self.key.clone();
                move || format!("tool-group-rows-{key}")
            })
            .child(
                div()
                    .id(gpui::ElementId::Name(
                        format!("tool-group-rows-{}", self.key).into(),
                    ))
                    .w_full()
                    .min_w_0()
                    .max_h(cap)
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .child(v_flex().w_full().min_w_0().children(rows)),
            )
            // The scrollbar and wheel mask are siblings of the scroll area, not
            // children: as absolute overlays inside a tracked scroller they
            // inflate the computed content size and make a group look
            // scrollable when it is not.
            .vertical_scrollbar(&self.scroll_handle)
            // The mask consumes the wheel while the group still has room to
            // scroll and hands it back to the transcript at either edge. Each
            // group needs its own id so groups do not share a gesture axis lock.
            .child(
                ScrollableMask::new(Axis::Vertical, &self.scroll_handle)
                    .id(format!("tool-group-mask-{}", self.key)),
            )
            .into_any_element()
    }

    /// A long group whose content certainly overflows the cap. The list is
    /// given the definite cap height, so no measurement has to round-trip
    /// through layout to size the group.
    fn render_virtualized_rows(&self, cap: Pixels, cx: &App) -> AnyElement {
        let border = cx.theme().border.opacity(0.55);
        let rows = self.tools.clone();
        let row_ids = self.row_ids.clone();
        let expanded_rows = self.expanded_rows.clone();
        let on_row_toggle = self.on_row_toggle.clone();
        let list_state = self.list_state.clone();
        div()
            .relative()
            .w_full()
            .min_w_0()
            .debug_selector({
                let key = self.key.clone();
                move || format!("tool-group-rows-{key}")
            })
            .child(
                list(list_state.clone(), move |index, window, cx| {
                    let Some(tool) = rows.get(index).cloned() else {
                        return div().into_any_element();
                    };
                    tool_row_element(
                        index,
                        tool,
                        row_ids.get(index).cloned().unwrap_or_default(),
                        expanded_rows.get(index).copied().unwrap_or(false),
                        on_row_toggle.clone(),
                        border,
                        window,
                        cx,
                    )
                })
                .w_full()
                .min_w_0()
                .h(cap),
            )
            .vertical_scrollbar(&list_state)
            .child(
                ScrollableMask::new(Axis::Vertical, &list_state)
                    .id(format!("tool-group-mask-{}", self.key)),
            )
            .into_any_element()
    }
}

/// One row, separated from the row above it, ready for either rows layer.
#[allow(clippy::too_many_arguments)]
fn tool_row_element(
    index: usize,
    tool: ToolCall,
    id: String,
    expanded: bool,
    on_row_toggle: Arc<dyn Fn(&str, &mut App) + Send + Sync>,
    border: Hsla,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let on_toggle = {
        let id = id.clone();
        Arc::new(move |cx: &mut App| on_row_toggle(&id, cx))
    };
    let row_selector = id.clone();
    let row = ToolRow {
        id,
        expanded,
        on_toggle: Some(on_toggle),
    };
    div()
        .w_full()
        .min_w_0()
        .when(index > 0, |row| row.border_t_1().border_color(border))
        .debug_selector(move || format!("tool-row-{row_selector}"))
        .child(tool.render_row(row, window, cx))
        .into_any_element()
}

impl Render for ToolGroup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tools.is_empty() {
            return div();
        }

        let theme = cx.theme();
        let can_toggle = self.can_toggle();
        let open = !can_toggle || self.expanded;
        let on_toggle = self.on_toggle.clone();
        let title_style = gpui::StyleRefinement::default().px_3().py_2();
        // The accordion owns only the toggle header. Its animated panel is
        // deliberately left empty: the panel caches its height from prepaint and
        // uses it in the next frame's layout, so putting the rows inside it fed
        // the transcript a one-frame-stale group height. The rows are rendered
        // as a sibling below instead, where their height is resolved directly.
        let accordion = Accordion::new(format!("tool-group-accordion-{}", self.key))
            .bordered(false)
            .xsmall()
            .w_full()
            .min_w_0()
            .item(|item| {
                item.open(open)
                    .disabled(!can_toggle)
                    .title(render_summary(&self.tools, theme))
                    .title_style(title_style)
                    .hover(|style| style.bg(theme.accent.opacity(0.45)))
                    .bg(theme.transparent)
            })
            .when(can_toggle, |accordion| {
                accordion.on_toggle_click(move |_, _, cx| on_toggle(cx))
            });

        div()
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .py_2()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .border_1()
                    .border_color(theme.border.opacity(0.75))
                    .bg(theme.muted.opacity(0.35))
                    .rounded_lg()
                    .overflow_hidden()
                    .child(accordion)
                    .when(open, |this| {
                        this.child(
                            div()
                                .px_2()
                                .pb_2()
                                .w_full()
                                .min_w_0()
                                .child(self.render_rows(window, cx)),
                        )
                    }),
            )
    }
}

/// The tool group for `key`, created on first render and reused afterwards.
#[allow(clippy::too_many_arguments)]
pub(in crate::gui::chat_history) fn tool_group(
    key: &str,
    tools: Arc<[ToolCall]>,
    row_ids: Arc<[String]>,
    expanded_rows: Arc<[bool]>,
    collapsible: bool,
    tail: bool,
    expanded: bool,
    window: &mut Window,
    cx: &mut App,
    on_toggle: impl Fn(&mut App) + Send + Sync + 'static,
    on_row_toggle: impl Fn(&str, &mut App) + Send + Sync + 'static,
) -> Entity<ToolGroup> {
    let group = window.use_keyed_state(format!("tool-group-{key}"), cx, |_, _| {
        ToolGroup::new(key.to_string())
    });
    group.update(cx, |group, _| {
        group.configure(
            tools,
            row_ids,
            expanded_rows,
            collapsible,
            tail,
            expanded,
            Arc::new(on_toggle),
            Arc::new(on_row_toggle),
        )
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
    /// inspect its rows after layout.
    struct GroupProbe {
        tools: Arc<[ToolCall]>,
        expand_rows: bool,
        group: Option<Entity<ToolGroup>>,
    }

    impl Render for GroupProbe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let group = render_group_probe(
                "probe",
                self.tools.clone(),
                expand_all(&self.tools, self.expand_rows),
                false,
                true,
                window,
                cx,
            );
            self.group = Some(group.clone());
            div().w(px(800.)).child(group)
        }
    }

    /// A group sitting inside an outer list, so wheel handoff can be observed.
    struct NestedGroupProbe {
        tools: Arc<[ToolCall]>,
        outer_state: ListState,
        expand_rows: bool,
        tail: bool,
        group: Option<Entity<ToolGroup>>,
    }

    impl Render for NestedGroupProbe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let group = render_group_probe(
                "probe",
                self.tools.clone(),
                expand_all(&self.tools, self.expand_rows),
                self.tail,
                true,
                window,
                cx,
            );
            self.group = Some(group.clone());
            list(self.outer_state.clone(), move |index, _, _| {
                if index == 0 {
                    div().w(px(800.)).child(group.clone()).into_any_element()
                } else {
                    div().w(px(800.)).h(px(200.)).into_any_element()
                }
            })
            .w(px(800.))
            .h(px(300.))
        }
    }

    fn render_group_probe(
        key: &str,
        tools: Arc<[ToolCall]>,
        expanded_rows: Arc<[bool]>,
        tail: bool,
        expanded: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<ToolGroup> {
        let row_ids: Arc<[String]> = (0..tools.len()).map(|index| format!("row-{index}")).collect();
        tool_group(
            key,
            tools,
            row_ids,
            expanded_rows,
            true,
            tail,
            expanded,
            window,
            cx,
            |_| {},
            |_, _| {},
        )
    }

    fn expand_all(tools: &[ToolCall], expand_rows: bool) -> Arc<[bool]> {
        vec![expand_rows; tools.len()].into()
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

    /// Command rows with a mix of short and long (two-line) titles, all
    /// collapsed so each row is only a header.
    fn mixed_command_tools(count: usize) -> Arc<[ToolCall]> {
        (0..count)
            .map(|index| {
                let command = if index % 2 == 0 {
                    "cargo check".to_string()
                } else {
                    "cargo check --workspace --all-targets --all-features \
                     --message-format=json --keep-going"
                        .to_string()
                };
                let item: ThreadItem = serde_json::from_value(serde_json::json!({
                    "type": "commandExecution",
                    "id": format!("command-{index}"),
                    "pluginId": null,
                    "scriptPath": null,
                    "command": command,
                    "cwd": "/workspace",
                    "processId": null,
                    "source": "agent",
                    "status": "completed",
                    "commandActions": [],
                    "aggregatedOutput": null,
                    "exitCode": 0,
                    "durationMs": 42,
                }))
                .expect("valid command execution item");
                ToolCall::new(&item, None, WeakEntity::new_invalid(), false)
                    .expect("command items produce a tool call")
            })
            .collect()
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    /// Lay out one group and settle its rows over a couple of frames.
    fn settle_group(
        cx: &mut TestAppContext,
        tools: Arc<[ToolCall]>,
        expand_rows: bool,
    ) -> (Entity<GroupProbe>, &mut VisualTestContext) {
        cx.update(gpui_component::init);
        let (probe, cx) = cx.add_window_view(move |_, _| GroupProbe {
            tools: tools.clone(),
            expand_rows,
            group: None,
        });
        draw(cx);
        draw(cx);
        (probe, cx)
    }

    /// A short group renders every row and shrinks to their height instead of
    /// always taking the cap.
    #[gpui::test]
    fn short_group_shrinks_to_its_rows(cx: &mut TestAppContext) {
        let (probe, cx) = settle_group(cx, reasoning_tools(2), true);
        let cap = cx.update(|window, _| group_max_height(window));
        let bounds = cx
            .debug_bounds("tool-group-rows-probe")
            .expect("rows rendered");

        assert!(bounds.size.height > px(0.), "the rows area collapsed");
        assert!(
            bounds.size.height < cap,
            "expected the rows area to shrink below the {cap} cap, got {}",
            bounds.size.height
        );
        let max_offset = cx.update(|_, cx| {
            probe
                .read(cx)
                .group
                .as_ref()
                .expect("group rendered")
                .read(cx)
                .scroll_handle
                .max_offset()
                .y
        });
        assert_eq!(max_offset, px(0.), "a short group should not scroll");
    }

    /// A group whose rows cannot possibly fit the cap takes the cap height.
    #[gpui::test]
    fn long_group_fills_the_cap(cx: &mut TestAppContext) {
        let (probe, cx) = settle_group(cx, reasoning_tools(67), true);
        let cap = cx.update(|window, _| group_max_height(window));
        let bounds = cx
            .debug_bounds("tool-group-rows-probe")
            .expect("rows rendered");

        assert!(
            bounds.size.height <= cap && bounds.size.height > cap - px(2.),
            "expected the rows area to fill the {cap} cap, got {}",
            bounds.size.height
        );
        let viewport = cx.update(|_, cx| {
            probe
                .read(cx)
                .group
                .as_ref()
                .expect("group rendered")
                .read(cx)
                .list_state
                .viewport_bounds()
        });
        assert!(
            viewport.size.height > px(0.),
            "the virtualized list should be laid out"
        );
    }

    /// A collapsed group whose rows have mixed heights (some titles wrap to two
    /// lines) and whose content only just exceeds the cap must still reveal its
    /// last row when scrolled to the end.
    #[gpui::test]
    fn collapsed_group_with_wrapped_titles_reaches_its_last_row(cx: &mut TestAppContext) {
        use gpui::{ScrollDelta, ScrollWheelEvent, point};

        let mut tools = mixed_command_tools(11).to_vec();
        tools.extend(reasoning_tools(3).iter().cloned());
        let (probe, cx) = settle_group(cx, tools.into(), false);

        let group = cx.update(|_, cx| probe.read(cx).group.clone().expect("group rendered"));
        let max_offset = cx.update(|_, cx| group.read(cx).scroll_handle.max_offset().y);
        assert!(
            max_offset > px(0.),
            "the group should be scrollable (max_offset={max_offset:?})"
        );

        // Scroll all the way down. Events past the group's end bubble into the
        // surrounding div, so the group stays put for the final measurement.
        for _ in 0..200 {
            let offset = cx.update(|_, cx| group.read(cx).scroll_handle.offset().y);
            if offset <= -max_offset + px(1.) {
                break;
            }
            let container = cx.update(|_, cx| group.read(cx).scroll_handle.bounds());
            cx.simulate_event(ScrollWheelEvent {
                position: point(container.origin.x + px(20.), container.origin.y + px(20.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
                ..Default::default()
            });
            draw(cx);
        }

        let last = cx
            .debug_bounds("tool-row-row-13")
            .expect("the last row should be rendered");
        let container = cx
            .debug_bounds("tool-group-rows-probe")
            .expect("rows rendered");
        assert!(
            last.bottom() <= container.bottom() + px(1.),
            "the last row should be fully visible (last={last:?}, container={container:?})"
        );
    }

    /// Expanding rows after the group has already been laid out must grow the
    /// group and make it scroll, rather than clipping the new detail.
    #[gpui::test]
    fn expanding_rows_makes_the_group_scrollable(cx: &mut TestAppContext) {
        use gpui::{ListAlignment, ScrollDelta, ScrollWheelEvent, point};

        cx.update(gpui_component::init);
        let outer_state = ListState::new(10, ListAlignment::Top, px(0.));
        let (probe, cx) = cx.add_window_view({
            let outer_state = outer_state.clone();
            move |_, _| NestedGroupProbe {
                tools: reasoning_tools(3),
                outer_state: outer_state.clone(),
                expand_rows: false,
                tail: false,
                group: None,
            }
        });
        let cx: &mut VisualTestContext = cx;
        draw(cx);
        draw(cx);

        // Rows are collapsed: three short rows fit the cap, so the wheel must
        // bubble to the outer list.
        let group = cx.update(|_, cx| probe.read(cx).group.clone().expect("group rendered"));
        let collapsed_bounds = cx.update(|_, cx| group.read(cx).scroll_handle.bounds());
        let collapsed_max = cx.update(|_, cx| group.read(cx).scroll_handle.max_offset().y);
        assert_eq!(collapsed_max, px(0.), "collapsed rows should fit the cap");
        cx.simulate_event(ScrollWheelEvent {
            position: point(
                collapsed_bounds.origin.x + px(20.),
                collapsed_bounds.origin.y + px(20.),
            ),
            delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
            ..Default::default()
        });
        draw(cx);
        let outer_top = outer_state.logical_scroll_top();
        assert_ne!(
            (outer_top.item_ix, outer_top.offset_in_item),
            (0, px(0.)),
            "with short rows the outer list should scroll"
        );

        // Expand every row: the group grows past the cap, and the same wheel
        // gesture must stay inside the group.
        probe.update(cx, |probe, cx| {
            probe.expand_rows = true;
            cx.notify();
        });
        draw(cx);
        draw(cx);
        let group = cx.update(|_, cx| probe.read(cx).group.clone().expect("group rendered"));
        let expanded_bounds = cx.update(|_, cx| group.read(cx).scroll_handle.bounds());
        let expanded_max = cx.update(|_, cx| group.read(cx).scroll_handle.max_offset().y);
        assert!(
            expanded_max > collapsed_max,
            "expanding rows should grow the group ({collapsed_max:?} -> {expanded_max:?})"
        );

        let outer_before = outer_state.logical_scroll_top();
        cx.simulate_event(ScrollWheelEvent {
            position: point(
                expanded_bounds.origin.x + px(20.),
                expanded_bounds.origin.y + px(20.),
            ),
            delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
            ..Default::default()
        });
        draw(cx);
        let group_offset = cx.update(|_, cx| group.read(cx).scroll_handle.offset().y);
        assert!(
            group_offset < px(0.),
            "the group should have grown and scrolled"
        );
        assert_eq!(
            (
                outer_state.logical_scroll_top().item_ix,
                outer_state.logical_scroll_top().offset_in_item
            ),
            (outer_before.item_ix, outer_before.offset_in_item),
            "the outer list should not scroll while the group consumes the wheel"
        );
    }

    /// A short group can still cross the cap when one row expands. It stays on
    /// the full-render path, where `max_h(cap)` turns the overflow into an
    /// internal scroll, so the height is still resolved in the same layout.
    #[gpui::test]
    fn expanding_one_row_of_a_short_group_crosses_the_cap(cx: &mut TestAppContext) {
        struct SingleRowProbe {
            tools: Arc<[ToolCall]>,
            expanded_row: Option<usize>,
            group: Option<Entity<ToolGroup>>,
        }

        impl Render for SingleRowProbe {
            fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let expanded_rows: Arc<[bool]> = (0..self.tools.len())
                    .map(|index| Some(index) == self.expanded_row)
                    .collect::<Vec<_>>()
                    .into();
                let group = render_group_probe(
                    "probe",
                    self.tools.clone(),
                    expanded_rows,
                    false,
                    true,
                    window,
                    cx,
                );
                self.group = Some(group.clone());
                div().w(px(800.)).child(group)
            }
        }

        cx.update(gpui_component::init);
        let (probe, cx) = cx.add_window_view(move |_, _| SingleRowProbe {
            // Enough collapsed rows to sit just under the cap, but few enough
            // that the group is still rendered in full.
            tools: reasoning_tools(10),
            expanded_row: None,
            group: None,
        });
        let cx: &mut VisualTestContext = cx;
        draw(cx);
        draw(cx);

        let cap = cx.update(|window, _| group_max_height(window));
        let collapsed = cx
            .debug_bounds("tool-group-rows-probe")
            .expect("rows rendered")
            .size
            .height;
        assert!(
            collapsed < cap,
            "collapsed rows should fit the cap ({collapsed:?} < {cap:?})"
        );

        probe.update(cx, |probe, cx| {
            probe.expanded_row = Some(5);
            cx.notify();
        });
        draw(cx);

        let expanded = cx
            .debug_bounds("tool-group-rows-probe")
            .expect("rows rendered")
            .size
            .height;
        let max_offset = cx.update(|_, cx| {
            probe
                .read(cx)
                .group
                .as_ref()
                .expect("group rendered")
                .read(cx)
                .scroll_handle
                .max_offset()
                .y
        });
        assert!(
            expanded > collapsed,
            "the single expanded row should grow the group ({collapsed:?} -> {expanded:?})"
        );
        assert!(
            expanded <= cap + px(1.),
            "the group should be capped (expanded={expanded:?}, cap={cap:?})"
        );
        assert!(
            max_offset > px(0.),
            "the capped group should scroll (max_offset={max_offset:?})"
        );
    }

    /// The regression this layout exists to prevent: a short group that grows
    /// past the cap must report its new height to the surrounding list during
    /// the same layout. A one-frame-stale block height makes the transcript
    /// scroll jump when it corrects.
    #[gpui::test]
    fn expansion_crossing_the_cap_updates_the_outer_height_same_frame(cx: &mut TestAppContext) {
        use gpui::ListAlignment;

        struct OuterProbe {
            tools: Arc<[ToolCall]>,
            outer_state: ListState,
            expand_rows: bool,
        }

        impl Render for OuterProbe {
            fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let group = render_group_probe(
                    "probe",
                    self.tools.clone(),
                    expand_all(&self.tools, self.expand_rows),
                    false,
                    true,
                    window,
                    cx,
                );
                list(self.outer_state.clone(), move |index, _, _| {
                    if index == 0 {
                        div().w(px(800.)).child(group.clone()).into_any_element()
                    } else {
                        div().w(px(800.)).h(px(200.)).into_any_element()
                    }
                })
                .w(px(800.))
                .h(px(400.))
            }
        }

        cx.update(gpui_component::init);
        let outer_state = ListState::new(10, ListAlignment::Top, px(0.)).measure_all();
        let (probe, cx) = cx.add_window_view({
            let outer_state = outer_state.clone();
            move |_, _| OuterProbe {
                tools: reasoning_tools(3),
                outer_state: outer_state.clone(),
                expand_rows: false,
            }
        });
        let cx: &mut VisualTestContext = cx;
        draw(cx);
        draw(cx);

        let cap = cx.update(|window, _| group_max_height(window));
        let collapsed = outer_state
            .bounds_for_item(0)
            .map(|bounds| bounds.size.height)
            .expect("the group should be measured");
        assert!(
            collapsed < cap,
            "the collapsed group should fit the cap (collapsed={collapsed:?}, cap={cap:?})"
        );

        // Expand every row. Invalidate the group's transcript block the way
        // `ChatHistory` does, then draw once.
        probe.update(cx, |probe, cx| {
            probe.expand_rows = true;
            cx.notify();
        });
        outer_state.remeasure_items(0..1);
        draw(cx);

        let expanded = outer_state
            .bounds_for_item(0)
            .map(|bounds| bounds.size.height)
            .expect("the group should be measured");
        assert!(
            expanded > collapsed,
            "the group height must grow in the same frame (collapsed={collapsed:?}, expanded={expanded:?})"
        );
        assert!(
            expanded <= cap + px(80.),
            "the rows should be capped (expanded={expanded:?}, cap={cap:?})"
        );
    }

    /// A tall group must be scrollable all the way to its last row.
    #[gpui::test]
    fn tall_group_scrolls_to_its_last_row(cx: &mut TestAppContext) {
        use gpui::{ScrollDelta, ScrollWheelEvent, point};

        let (probe, cx) = settle_group(cx, reasoning_tools(12), true);
        let group = cx.update(|_, cx| probe.read(cx).group.clone().expect("group rendered"));
        let max_offset = cx.update(|_, cx| group.read(cx).scroll_handle.max_offset().y);
        assert!(
            max_offset > px(0.),
            "expanded rows should make the group scrollable (max_offset={max_offset:?})"
        );

        for _ in 0..200 {
            let offset = cx.update(|_, cx| group.read(cx).scroll_handle.offset().y);
            if offset <= -max_offset + px(1.) {
                break;
            }
            let container = cx.update(|_, cx| group.read(cx).scroll_handle.bounds());
            cx.simulate_event(ScrollWheelEvent {
                position: point(container.origin.x + px(20.), container.origin.y + px(20.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
                ..Default::default()
            });
            draw(cx);
        }

        let offset = cx.update(|_, cx| group.read(cx).scroll_handle.offset().y);
        assert!(
            offset <= -max_offset + px(1.),
            "the group should reach its end (offset={offset:?}, max_offset={max_offset:?})"
        );
        let last = cx
            .debug_bounds("tool-row-row-11")
            .expect("the last row should be rendered");
        let container = cx
            .debug_bounds("tool-group-rows-probe")
            .expect("rows rendered");
        assert!(
            last.bottom() <= container.bottom() + px(1.),
            "the last row should be visible (last={last:?}, container={container:?})"
        );
    }

    /// At the group's bottom edge the wheel must bubble to the transcript, just
    /// like it does at the top edge.
    #[gpui::test]
    fn tall_group_hands_wheel_back_at_its_bottom(cx: &mut TestAppContext) {
        use gpui::{ListAlignment, ScrollDelta, ScrollWheelEvent, point};

        cx.update(gpui_component::init);
        let outer_state = ListState::new(10, ListAlignment::Top, px(0.));
        let (probe, cx) = cx.add_window_view({
            let outer_state = outer_state.clone();
            move |_, _| NestedGroupProbe {
                tools: reasoning_tools(12),
                outer_state: outer_state.clone(),
                expand_rows: true,
                tail: true,
                group: None,
            }
        });
        let cx: &mut VisualTestContext = cx;
        draw(cx);
        draw(cx);

        let group = cx.update(|_, cx| probe.read(cx).group.clone().expect("group rendered"));
        let max_offset = cx.update(|_, cx| group.read(cx).scroll_handle.max_offset().y);
        for _ in 0..200 {
            let offset = cx.update(|_, cx| group.read(cx).scroll_handle.offset().y);
            if offset <= -max_offset + px(1.) {
                break;
            }
            let container = cx.update(|_, cx| group.read(cx).scroll_handle.bounds());
            cx.simulate_event(ScrollWheelEvent {
                position: point(container.origin.x + px(20.), container.origin.y + px(20.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
                ..Default::default()
            });
            draw(cx);
        }
        assert!(
            cx.update(|_, cx| group.read(cx).scroll_handle.offset().y) <= -max_offset + px(1.),
            "the group should be scrolled to its end before the handoff"
        );

        let outer_before = outer_state.logical_scroll_top();
        let container = cx.update(|_, cx| group.read(cx).scroll_handle.bounds());
        cx.simulate_event(ScrollWheelEvent {
            position: point(container.origin.x + px(20.), container.origin.y + px(20.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
            ..Default::default()
        });
        draw(cx);
        let outer_after = outer_state.logical_scroll_top();
        assert_ne!(
            (outer_after.item_ix, outer_after.offset_in_item),
            (outer_before.item_ix, outer_before.offset_in_item),
            "the outer list should scroll once the group reaches its bottom"
        );
    }
}

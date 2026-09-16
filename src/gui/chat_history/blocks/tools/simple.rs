use gpui::{
    AnyElement, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div,
    prelude::*, px,
};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, spinner::Spinner};

use crate::gui::chat_history::motion::ShimmerText;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ToolStatus {
    Running,
    Succeeded,
    Failed,
}

impl ToolStatus {
    pub(super) fn done(self) -> bool {
        !matches!(self, Self::Running)
    }
}

pub(super) trait SimpleTool: 'static {
    fn icon(&self) -> IconName;
    fn title(&self) -> SharedString;
    fn detail(&self) -> Option<SharedString>;
    fn status(&self) -> ToolStatus;
    fn detail_style(&self) -> DetailStyle {
        DetailStyle::Code
    }
}

/// How a `SimpleTool`'s detail text is presented.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum DetailStyle {
    /// Code-like output: monospace, small, height-capped and scrollable.
    Code,
    /// Prose: soft-wrapped at a readable size and shown in full.
    Prose,
}

#[derive(IntoElement)]
pub(super) struct SimpleToolElement<T: SimpleTool>(T);

impl<T: SimpleTool> SimpleToolElement<T> {
    pub(super) fn new(tool: T) -> Self {
        Self(tool)
    }
}

impl<T: SimpleTool> RenderOnce for SimpleToolElement<T> {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        ToolFrame::new(
            self.0.icon(),
            self.0.title(),
            self.0.detail(),
            self.0.status(),
        )
        .detail_style(self.0.detail_style())
    }
}

#[derive(IntoElement)]
pub(super) struct ToolFrame {
    icon: IconName,
    title: SharedString,
    detail: Option<(AnyElement, bool)>,
    detail_style: DetailStyle,
    status: ToolStatus,
    diff: Option<(usize, usize)>,
}

impl ToolFrame {
    pub(super) fn new(
        icon: IconName,
        title: SharedString,
        detail: Option<SharedString>,
        status: ToolStatus,
    ) -> Self {
        Self {
            icon,
            title,
            detail: detail.map(|detail| (detail.into_any_element(), true)),
            detail_style: DetailStyle::Code,
            status,
            diff: None,
        }
    }

    pub(super) fn detail_style(mut self, style: DetailStyle) -> Self {
        self.detail_style = style;
        self
    }

    pub(super) fn diff(mut self, additions: usize, deletions: usize) -> Self {
        self.diff = Some((additions, deletions));
        self
    }

    pub(super) fn custom_detail(mut self, detail: impl IntoElement) -> Self {
        self.detail = Some((detail.into_any_element(), false));
        self
    }
}

impl RenderOnce for ToolFrame {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let title = if self.status == ToolStatus::Running && !cx.reduce_motion() {
            ShimmerText::new(
                "tool-title-shimmer",
                self.title,
                theme.muted_foreground.opacity(0.62),
                theme.foreground,
            )
            .into_any_element()
        } else {
            div().child(self.title).into_any_element()
        };
        h_flex()
            .max_w_full()
            .min_w_0()
            .flex_shrink(1.)
            .items_start()
            .gap_2()
            .px_1()
            .py_2()
            .text_sm()
            .child(
                div()
                    .flex_none()
                    .size_7()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(theme.accent.opacity(0.72))
                    .child(
                        Icon::new(self.icon)
                            .small()
                            .text_color(theme.accent_foreground),
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().child(title))
                    .when_some(self.detail, |this, (detail, scrollable)| {
                        let detail = div()
                            .id("tool-detail")
                            .min_w_0()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border.opacity(0.7))
                            .bg(theme.background.opacity(0.58))
                            .px_2()
                            // .py_1p5()
                            .text_color(theme.muted_foreground)
                            .whitespace_normal()
                            .when(self.detail_style == DetailStyle::Code, |detail| {
                                detail
                                    .max_h(px(176.))
                                    .font_family(theme.mono_font_family.clone())
                                    .text_xs()
                                    .line_height(px(18.))
                            })
                            .when(self.detail_style == DetailStyle::Prose, |detail| {
                                detail.text_sm().line_height(px(20.))
                            })
                            .child(detail);
                        this.child(if scrollable && self.detail_style == DetailStyle::Code {
                            detail.overflow_scrollbar().into_any_element()
                        } else {
                            detail.overflow_hidden().into_any_element()
                        })
                    }),
            )
            .child(render_trailing(self.diff, self.status, cx))
    }
}

fn render_trailing(diff: Option<(usize, usize)>, status: ToolStatus, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .when_some(diff, |trailing, (additions, deletions)| {
            trailing.child(
                h_flex()
                    .gap_1()
                    .rounded_full()
                    .bg(theme.muted.opacity(0.8))
                    .px_1p5()
                    .py_0p5()
                    .text_xs()
                    .child(
                        div()
                            .text_color(theme.success)
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_color(theme.danger)
                            .child(format!("-{deletions}")),
                    ),
            )
        })
        .child(render_status(status, cx))
        .into_any_element()
}

fn render_status(status: ToolStatus, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match status {
        ToolStatus::Running => h_flex()
            .gap_1()
            .rounded_full()
            .bg(theme.warning.opacity(0.14))
            .px_1p5()
            .py_0p5()
            .text_xs()
            .text_color(theme.warning)
            .child(Spinner::new().xsmall().color(theme.warning))
            .child("Running")
            .into_any_element(),
        ToolStatus::Succeeded => h_flex()
            .gap_1()
            .rounded_full()
            .bg(theme.success.opacity(0.12))
            .px_1p5()
            .py_0p5()
            .text_xs()
            .text_color(theme.success)
            .child(Icon::new(IconName::Check).xsmall())
            .child("Done")
            .into_any_element(),
        ToolStatus::Failed => h_flex()
            .gap_1()
            .rounded_full()
            .bg(theme.danger.opacity(0.12))
            .px_1p5()
            .py_0p5()
            .text_xs()
            .text_color(theme.danger)
            .child(Icon::new(IconName::CircleX).xsmall())
            .child("Failed")
            .into_any_element(),
    }
}

pub(super) fn append_progress(
    detail: Option<String>,
    progress: Option<&[SharedString]>,
) -> Option<SharedString> {
    let mut parts = Vec::new();
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        parts.push(detail);
    }
    if let Some(progress) = progress {
        let progress = progress
            .iter()
            .map(AsRef::<str>::as_ref)
            .collect::<Vec<_>>()
            .join("\n");
        if !progress.is_empty() {
            parts.push(progress);
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n\n").into())
}

pub(super) fn format_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "<unavailable>".into())
}

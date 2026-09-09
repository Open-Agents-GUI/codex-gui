use crate::gui::{WorkspaceState, widgets::render_notice};
use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div, px,
};
use gpui_component::ActiveTheme as _;

pub struct SideChat {
    state: Entity<WorkspaceState>,
    _subscriptions: Vec<Subscription>,
}

impl SideChat {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        Self {
            state,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for SideChat {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thread_title = self
            .state
            .read(cx)
            .active_chat_entity(cx)
            .map(|chat| chat.read(cx).title.to_string())
            .unwrap_or_else(|| "No thread".into());

        div()
            .w(px(340.))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(cx.theme().sidebar_border)
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .child(
                div()
                    .h(px(58.))
                    .flex()
                    .items_center()
                    .px_4()
                    .border_b_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Side Chat"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .rounded_md()
                            .border_1()
                            .border_color(cx.theme().sidebar_border)
                            .bg(cx.theme().sidebar_accent)
                            .text_color(cx.theme().sidebar_accent_foreground)
                            .p_3()
                            .child(format!("Temporary view of {thread_title}.")),
                    )
                    .child(render_notice(
                        "Side chats remain a UI-only view until promoted through thread/fork.",
                        cx.theme(),
                    )),
            )
    }
}

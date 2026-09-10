mod attachments;
mod composer;
mod composer_view;
mod new_chat;

use crate::app::CodexGui;
use crate::gui::{ChatHistory, ChatHistoryEvent, ChatState, WindowState, WorkspaceState};
use gpui::{
    Context, Entity, IntoElement, MouseButton, ParentElement, Render, Styled, Subscription,
    WeakEntity, Window, WindowControlArea, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{InputEvent, InputState, TextareaState},
    spinner::Spinner,
};

pub struct ChatPanel {
    parent: WeakEntity<CodexGui>,
    state: Entity<WorkspaceState>,
    window_state: Entity<WindowState>,
    history: Entity<ChatHistory>,
    composer_input: Entity<TextareaState>,
    composer_context: ComposerContext,
    composer_chat_subscription: Option<Subscription>,
    project_path_input: Entity<InputState>,
    should_move_window: bool,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, PartialEq)]
enum ComposerContext {
    NewChat,
    Chat(Entity<ChatState>),
}

impl ChatPanel {
    pub fn new(
        parent: WeakEntity<CodexGui>,
        state: Entity<WorkspaceState>,
        window_state: Entity<WindowState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 5)
                .submit_on_enter(true)
                .placeholder("Do anything")
        });
        let history = cx.new(|cx| ChatHistory::new(state.clone(), cx));
        let project_path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .submit_on_enter(true)
                .placeholder("/path/to/project")
        });
        let subscriptions = vec![
            cx.observe_in(&state, window, |view, _, window, cx| {
                view.sync_composer_context(window, cx);
                cx.notify();
            }),
            cx.observe_in(&window_state, window, |view, _, window, cx| {
                view.sync_composer_context(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&composer_input, window, |view, _, event, window, cx| {
                view.save_composer_draft(cx);
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    let answering_input = view
                        .composer_chat()
                        .is_some_and(|chat| chat.read(cx).pending_freeform_input().is_some());
                    if answering_input {
                        view.send_composer_turn(window, cx);
                    } else if view.active_chat_turn_running(cx) && !view.active_chat_editing(cx) {
                        view.steer_composer_turn(window, cx);
                    } else {
                        view.send_composer_turn(window, cx);
                    }
                }
            }),
            cx.subscribe_in(&history, window, |view, _, event, window, cx| match event {
                ChatHistoryEvent::EditUserMessage {
                    chat,
                    turn_id,
                    previous_turn_id,
                    body,
                } => view.begin_editing_message(
                    chat.clone(),
                    turn_id.clone(),
                    previous_turn_id.clone(),
                    body,
                    window,
                    cx,
                ),
                ChatHistoryEvent::ForkTurn { chat, turn_id } => {
                    view.fork_chat_through(chat.clone(), turn_id.clone(), cx)
                }
                ChatHistoryEvent::ResolveApproval {
                    chat,
                    request_id,
                    approved,
                } => {
                    let parent = view.parent.clone();
                    let chat = chat.clone();
                    let request_id = request_id.clone();
                    let approved = *approved;
                    cx.defer(move |cx| {
                        let _ = parent.update(cx, |parent, cx| {
                            parent.resolve_approval(chat, request_id, approved, cx)
                        });
                    });
                }
                ChatHistoryEvent::AnswerInput {
                    chat,
                    request_id,
                    question_id,
                    answer,
                } => {
                    let parent = view.parent.clone();
                    let chat = chat.clone();
                    let request_id = request_id.clone();
                    let question_id = question_id.clone();
                    let answer = answer.clone();
                    cx.defer(move |cx| {
                        let _ = parent.update(cx, |parent, cx| {
                            parent.answer_server_input(chat, request_id, question_id, answer, cx)
                        });
                    });
                }
                ChatHistoryEvent::RejectInput { chat, request_id } => {
                    let parent = view.parent.clone();
                    let chat = chat.clone();
                    let request_id = request_id.clone();
                    cx.defer(move |cx| {
                        let _ = parent.update(cx, |parent, cx| {
                            parent.reject_server_input(chat, request_id, cx)
                        });
                    });
                }
                ChatHistoryEvent::DismissNotice { chat_id, notice_id } => {
                    let parent = view.parent.clone();
                    let chat_id = chat_id.clone();
                    let notice_id = notice_id.clone();
                    cx.defer(move |cx| {
                        let _ = parent.update(cx, |parent, cx| {
                            parent.dismiss_notice(chat_id, notice_id, cx)
                        });
                    });
                }
            }),
            cx.subscribe_in(&project_path_input, window, |view, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    view.add_project_from_input(window, cx);
                }
            }),
        ];

        Self {
            parent,
            state,
            window_state,
            history,
            composer_input,
            composer_context: ComposerContext::NewChat,
            composer_chat_subscription: None,
            project_path_input,
            should_move_window: false,
            _subscriptions: subscriptions,
        }
    }

    fn fork_chat_through(
        &mut self,
        chat: Entity<ChatState>,
        turn_id: String,
        cx: &mut Context<Self>,
    ) {
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.fork_chat_through(chat, turn_id, cx));
        });
    }

    fn toggle_side_chat(&mut self, cx: &mut Context<Self>) {
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.toggle_side_chat(cx));
        });
    }

    fn loading_thread_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .child(Spinner::new().small().color(cx.theme().muted_foreground))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Loading thread…"),
            )
    }
}

impl Render for ChatPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let new_chat_open = self.window_state.read(cx).new_chat_open;
        let active_chat = self.state.read(cx).active_chat_entity(cx);
        let (title, subtitle) = active_chat
            .as_ref()
            .map(|chat| {
                let chat = chat.read(cx);
                (chat.title.to_string(), chat.subtitle.to_string())
            })
            .unwrap_or_else(|| ("No thread selected".into(), "Start a Codex thread".into()));
        let thread_loading = !new_chat_open
            && active_chat
                .as_ref()
                .is_some_and(|chat| chat.read(cx).is_loading);

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .child(
                div()
                    .h(px(58.))
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down_out(cx.listener(|view, _, _, _| {
                        view.should_move_window = false;
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, _| {
                            view.should_move_window = true;
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|view, _, _, _| {
                            view.should_move_window = false;
                        }),
                    )
                    .on_mouse_move(cx.listener(|view, _, window, _| {
                        if view.should_move_window {
                            view.should_move_window = false;
                            window.start_window_move();
                        }
                    }))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .px_5()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(1000.))
                            .mx_auto()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_lg()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .overflow_x_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .overflow_x_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .child(subtitle),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .child(
                                        Button::new("toggle-side-chat")
                                            .small()
                                            .ghost()
                                            .icon(IconName::PanelRightOpen)
                                            .tooltip("Open side chat")
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.toggle_side_chat(cx)
                                            })),
                                    ),
                            ),
                    ),
            )
            .when(new_chat_open, |this| this.child(self.new_chat_page(cx)))
            .when(thread_loading, |this| {
                this.child(self.loading_thread_page(cx))
            })
            .when(!new_chat_open && !thread_loading, |this| {
                this.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.history.clone()),
                )
                .child(self.composer(cx))
            })
    }
}

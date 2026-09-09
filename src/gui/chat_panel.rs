use crate::app::CodexGui;
use crate::gui::{
    ApprovalsReviewer, ChatHistory, ChatHistoryEvent, ChatState, EditingMessage, WindowState,
    WorkspaceState, approvals_reviewer_label, new_client_user_message_id,
};
use gpui::{
    Context, Entity, IntoElement, MouseButton, ParentElement, Render, Styled, Subscription,
    WeakEntity, Window, WindowControlArea, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme as _, IconName, Selectable as _, Side, Sizable as _, box_shadow,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement,
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

    fn desired_composer_context(&self, cx: &mut Context<Self>) -> ComposerContext {
        if self.window_state.read(cx).new_chat_open {
            ComposerContext::NewChat
        } else {
            self.state
                .read(cx)
                .active_chat_entity(cx)
                .map(ComposerContext::Chat)
                .unwrap_or(ComposerContext::NewChat)
        }
    }

    fn save_composer_draft(&self, cx: &mut Context<Self>) {
        let draft = self.composer_input.read(cx).value().to_string();
        match &self.composer_context {
            ComposerContext::NewChat => self.window_state.update(cx, |state, _| {
                state.new_chat_draft = draft;
            }),
            ComposerContext::Chat(chat) => chat.update(cx, |chat, _| {
                chat.draft = draft;
            }),
        }
    }

    fn sync_composer_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.desired_composer_context(cx);
        if next == self.composer_context {
            return;
        }
        self.save_composer_draft(cx);
        let draft = match &next {
            ComposerContext::NewChat => self.window_state.read(cx).new_chat_draft.clone(),
            ComposerContext::Chat(chat) => chat.read(cx).draft.clone(),
        };
        self.composer_chat_subscription = match &next {
            ComposerContext::NewChat => None,
            ComposerContext::Chat(chat) => Some(cx.observe(chat, |_, _, cx| cx.notify())),
        };
        self.composer_context = next;
        self.composer_input
            .update(cx, |input, cx| input.set_value(draft, window, cx));
    }

    fn composer_chat(&self) -> Option<Entity<ChatState>> {
        match &self.composer_context {
            ComposerContext::NewChat => None,
            ComposerContext::Chat(chat) => Some(chat.clone()),
        }
    }

    fn active_chat_editing(&self, cx: &mut Context<Self>) -> bool {
        self.composer_chat()
            .is_some_and(|chat| chat.read(cx).editing_message.is_some())
    }

    fn send_composer_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.user_message_sending(cx) {
            return;
        }
        let target_chat = self.composer_chat();
        let (text, source_bounds) = self.composer_input.update(cx, |input, cx| {
            let text = input.value().trim().to_string();
            let source_bounds = input.text_bounds().unwrap_or_else(|| input.input_bounds());
            if !text.is_empty() {
                input.set_value("", window, cx);
            }
            (text, source_bounds)
        });
        if text.is_empty() {
            return;
        }
        let pending_input = target_chat
            .as_ref()
            .and_then(|chat| chat.read(cx).pending_freeform_input());
        if let Some((request_id, question_id)) = pending_input {
            let parent = self.parent.clone();
            let chat = target_chat.expect("pending input belongs to a chat");
            cx.defer(move |cx| {
                let _ = parent.update(cx, |parent, cx| {
                    parent.answer_server_input(chat, request_id, question_id, text, cx)
                });
            });
            return;
        }
        let editing_message = target_chat.as_ref().and_then(|chat| {
            chat.update(cx, |chat, cx| {
                let editing = chat.editing_message.take();
                if editing.is_some() {
                    cx.notify();
                }
                editing
            })
        });
        let client_user_message_id = new_client_user_message_id();
        if editing_message.is_none() {
            self.history.update(cx, |history, cx| {
                history.begin_send_animation(client_user_message_id.clone(), source_bounds, cx)
            });
        }
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| {
                if let Some(editing_message) = editing_message {
                    let Some(chat) = target_chat else {
                        return;
                    };
                    parent.submit_edited_turn_text(
                        chat,
                        editing_message.turn_id,
                        editing_message.previous_turn_id,
                        client_user_message_id,
                        text,
                        cx,
                    );
                } else {
                    match target_chat {
                        Some(chat) => {
                            parent.submit_turn_text(chat, client_user_message_id, text, cx)
                        }
                        None => parent.submit_new_turn_text(client_user_message_id, text, cx),
                    }
                }
            });
        });
    }

    fn begin_editing_message(
        &mut self,
        chat: Entity<ChatState>,
        turn_id: String,
        previous_turn_id: Option<String>,
        body: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_chat_turn_running(cx) {
            return;
        }
        if self.composer_chat().as_ref() != Some(&chat) {
            return;
        }
        chat.update(cx, |chat, cx| {
            chat.editing_message = Some(EditingMessage {
                turn_id,
                previous_turn_id,
            });
            chat.draft = body.to_string();
            cx.notify();
        });
        self.composer_input.update(cx, |input, cx| {
            input.set_value(body, window, cx);
            input.focus(window, cx);
        });
    }

    fn steer_composer_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.user_message_sending(cx) {
            return;
        }
        let Some(chat) = self.composer_chat() else {
            return;
        };
        let Some(turn_id) = chat.read(cx).active_turn_id().map(str::to_owned) else {
            return;
        };
        let (text, source_bounds) = self.composer_input.update(cx, |input, cx| {
            let text = input.value().trim().to_string();
            let source_bounds = input.text_bounds().unwrap_or_else(|| input.input_bounds());
            if !text.is_empty() {
                input.set_value("", window, cx);
            }
            (text, source_bounds)
        });
        if text.is_empty() {
            return;
        }
        let client_user_message_id = new_client_user_message_id();
        self.history.update(cx, |history, cx| {
            history.begin_send_animation(client_user_message_id.clone(), source_bounds, cx)
        });
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| {
                parent.steer_turn_text(chat, turn_id, client_user_message_id, text, cx)
            });
        });
    }

    fn stop_active_turn(&mut self, cx: &mut Context<Self>) {
        let Some(chat) = self.composer_chat() else {
            return;
        };
        let Some(turn_id) = chat.read(cx).active_turn_id().map(str::to_owned) else {
            return;
        };
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.stop_turn(chat, turn_id, cx));
        });
    }

    fn active_chat_turn_running(&self, cx: &mut Context<Self>) -> bool {
        self.composer_chat()
            .is_some_and(|chat| chat.read(cx).active_turn.is_some())
    }

    fn user_message_sending(&self, cx: &mut Context<Self>) -> bool {
        self.composer_chat()
            .is_some_and(|chat| chat.read(cx).user_message_is_sending())
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

    fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.select_project(index, cx));
        });
    }

    fn select_new_chat_projectless(&mut self, cx: &mut Context<Self>) {
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.select_new_chat_projectless(cx));
        });
    }

    fn add_project_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.project_path_input.update(cx, |input, cx| {
            let path = input.value().trim().to_string();
            if !path.is_empty() {
                input.set_value("", window, cx);
            }
            path
        });
        if path.is_empty() {
            return;
        }
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.add_project(path, cx));
        });
    }

    fn composer_surface(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings_chat = self.composer_chat();
        let (default_settings, models, permission_profiles) = {
            let state = self.state.read(cx);
            (
                state.new_chat_settings.clone(),
                state.available_models.clone(),
                state.permission_profiles.clone(),
            )
        };
        let settings = settings_chat
            .as_ref()
            .map(|chat| chat.read(cx).settings.clone())
            .unwrap_or(default_settings);
        let model_label =
            if let Some(model) = models.iter().find(|model| model.id == settings.model) {
                model.display_name.clone()
            } else {
                settings.model.clone()
            };
        let effort_options = models
            .iter()
            .find(|model| model.id == settings.model)
            .map(|model| model.supported_efforts.clone())
            .filter(|efforts| !efforts.is_empty())
            .unwrap_or_else(|| {
                vec![
                    "none".into(),
                    "minimal".into(),
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "xhigh".into(),
                ]
            });
        let turn_running = self.active_chat_turn_running(cx);
        let answering_input = self
            .composer_chat()
            .is_some_and(|chat| chat.read(cx).pending_freeform_input().is_some());
        let can_stop = turn_running && !answering_input;
        let user_message_sending = self.user_message_sending(cx);

        div()
            .w_full()
            .max_w(px(1000.))
            .rounded_3xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().input_background())
            .shadow(vec![box_shadow(
                0.,
                4.,
                12.,
                2.,
                cx.theme().transparent.alpha(0.1),
            )])
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                Textarea::new(&self.composer_input)
                    .appearance(false)
                    .min_h(px(60.))
                    .w_full(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("composer-model")
                                    .small()
                                    .ghost()
                                    .text_color(cx.theme().muted_foreground)
                                    .icon(IconName::Cpu)
                                    .label(model_label)
                                    .tooltip("Model settings")
                                    .dropdown_menu({
                                        let parent = self.parent.clone();
                                        let settings_chat = settings_chat.clone();
                                        let models = models.clone();
                                        let selected_model = settings.model.clone();
                                        move |menu, _, _| {
                                            let mut menu = menu
                                                .min_w(260.)
                                                .max_h(px(360.))
                                                .scrollable(true)
                                                .check_side(Side::Left);
                                            if models.is_empty() {
                                                return menu.item(
                                                    PopupMenuItem::new("Loading models")
                                                        .disabled(true),
                                                );
                                            }
                                            for model in &models {
                                                let id = model.id.clone();
                                                let label = model.display_name.clone();
                                                let parent = parent.clone();
                                                let settings_chat = settings_chat.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(label)
                                                        .checked(model.id == selected_model)
                                                        .on_click(move |_, _, cx| {
                                                            let id = id.clone();
                                                            let settings_chat =
                                                                settings_chat.clone();
                                                            let _ =
                                                                parent.update(cx, |parent, cx| {
                                                                    parent.set_model(
                                                                        settings_chat,
                                                                        id,
                                                                        cx,
                                                                    )
                                                                });
                                                        }),
                                                );
                                            }
                                            menu
                                        }
                                    }),
                            )
                            .child(
                                Button::new("composer-permissions")
                                    .small()
                                    .ghost()
                                    .text_color(cx.theme().muted_foreground)
                                    .icon(IconName::Check)
                                    .label(approvals_reviewer_label(settings.approvals_reviewer))
                                    .tooltip("Permission settings")
                                    .dropdown_menu({
                                        let parent = self.parent.clone();
                                        let settings_chat = settings_chat.clone();
                                        let settings = settings.clone();
                                        let permission_profiles = permission_profiles.clone();
                                        move |menu, _, _| {
                                            let mut menu = menu
                                                .min_w(240.)
                                                .check_side(Side::Left)
                                                .item(PopupMenuItem::label("Permissions"));
                                            for profile in &permission_profiles {
                                                let id = profile.id.clone();
                                                let parent = parent.clone();
                                                let settings_chat = settings_chat.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(profile.label.clone())
                                                        .checked(
                                                            settings.permission_profile
                                                                == profile.id,
                                                        )
                                                        .on_click(move |_, _, cx| {
                                                            let id = id.clone();
                                                            let settings_chat =
                                                                settings_chat.clone();
                                                            let _ =
                                                                parent.update(cx, |parent, cx| {
                                                                    parent.set_permission_profile(
                                                                        settings_chat,
                                                                        id,
                                                                        cx,
                                                                    )
                                                                });
                                                        }),
                                                );
                                            }
                                            menu = menu
                                                .separator()
                                                .item(PopupMenuItem::label("Approvals"));
                                            for reviewer in [
                                                ApprovalsReviewer::User,
                                                ApprovalsReviewer::AutoReview,
                                            ] {
                                                let parent = parent.clone();
                                                let settings_chat = settings_chat.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(approvals_reviewer_label(
                                                        reviewer,
                                                    ))
                                                    .checked(
                                                        settings.approvals_reviewer == reviewer,
                                                    )
                                                    .on_click(move |_, _, cx| {
                                                        let settings_chat = settings_chat.clone();
                                                        let _ = parent.update(cx, |parent, cx| {
                                                            parent.set_approvals_reviewer(
                                                                settings_chat,
                                                                reviewer,
                                                                cx,
                                                            )
                                                        });
                                                    }),
                                                );
                                            }
                                            menu
                                        }
                                    }),
                            )
                            .child(
                                Button::new("composer-effort")
                                    .small()
                                    .ghost()
                                    .text_color(cx.theme().muted_foreground)
                                    .icon(IconName::LoaderCircle)
                                    .label(format!("Effort {}", title_case(&settings.effort)))
                                    .tooltip("Thinking effort settings")
                                    .dropdown_menu({
                                        let parent = self.parent.clone();
                                        let settings_chat = settings_chat.clone();
                                        let selected_effort = settings.effort.clone();
                                        move |menu, _, _| {
                                            let mut menu = menu.min_w(190.).check_side(Side::Left);
                                            for effort in &effort_options {
                                                let value = effort.clone();
                                                let parent = parent.clone();
                                                let settings_chat = settings_chat.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(title_case(effort))
                                                        .checked(*effort == selected_effort)
                                                        .on_click(move |_, _, cx| {
                                                            let value = value.clone();
                                                            let settings_chat =
                                                                settings_chat.clone();
                                                            let _ =
                                                                parent.update(cx, |parent, cx| {
                                                                    parent.set_effort(
                                                                        settings_chat,
                                                                        value,
                                                                        cx,
                                                                    )
                                                                });
                                                        }),
                                                );
                                            }
                                            menu
                                        }
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(can_stop, |actions| {
                                actions.child(
                                    Button::new("steer-composer-turn")
                                        .small()
                                        .primary()
                                        .loading(user_message_sending)
                                        .rounded(px(999.))
                                        .icon(IconName::ArrowUp)
                                        .tooltip("Steer")
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            view.steer_composer_turn(window, cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("send-or-stop-composer-turn")
                                    .small()
                                    .loading(!can_stop && user_message_sending)
                                    .when(!can_stop, |button| button.primary())
                                    .when(can_stop, |button| button.danger())
                                    .rounded(px(999.))
                                    .icon(if can_stop {
                                        IconName::Close
                                    } else {
                                        IconName::ArrowUp
                                    })
                                    .tooltip(if can_stop { "Stop" } else { "Send" })
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        let answering_input =
                                            view.composer_chat().is_some_and(|chat| {
                                                chat.read(cx).pending_freeform_input().is_some()
                                            });
                                        if view.active_chat_turn_running(cx) && !answering_input {
                                            view.stop_active_turn(cx);
                                        } else {
                                            view.send_composer_turn(window, cx);
                                        }
                                    })),
                            ),
                    ),
            )
    }

    fn composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .px_5()
            .pb_5()
            .flex()
            .justify_center()
            .child(self.composer_surface(cx))
    }

    fn new_chat_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (projects, active_project, active_project_name, projectless) = {
            let state = self.state.read(cx);
            let active_project_name = state
                .active_project()
                .map(|project| project.read(cx).name.to_string())
                .unwrap_or_else(|| "this project".into());
            (
                state.projects.clone(),
                state.active_project,
                active_project_name,
                self.window_state.read(cx).new_chat_projectless || state.projects.is_empty(),
            )
        };
        let heading = if projectless {
            "What should we build?".to_owned()
        } else {
            format!("What should we build in {active_project_name}?")
        };

        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .px_5()
            .pb_20()
            .gap_7()
            .child(
                div()
                    .max_w(px(820.))
                    .w_full()
                    .text_center()
                    .text_3xl()
                    .font_weight(gpui::FontWeight::LIGHT)
                    .child(heading),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(820.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.composer_surface(cx))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .overflow_x_scrollbar()
                            .child(
                                Button::new("new-chat-projectless")
                                    .small()
                                    .ghost()
                                    .selected(projectless)
                                    .label("No project")
                                    .tooltip("Create a chat in Documents/Codex")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.select_new_chat_projectless(cx)
                                    })),
                            )
                            .children(projects.iter().enumerate().map(|(index, project)| {
                                let project = project.read(cx);
                                Button::new(format!("new-chat-project-{index}"))
                                    .small()
                                    .ghost()
                                    .selected(!projectless && index == active_project)
                                    .icon(if !projectless && index == active_project {
                                        IconName::FolderOpen
                                    } else {
                                        IconName::Folder
                                    })
                                    .label(project.name.clone())
                                    .tooltip(project.path.clone())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.select_project(index, cx)
                                    }))
                            })),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().background)
                                    .child(
                                        Input::new(&self.project_path_input)
                                            .appearance(false)
                                            .h(px(34.))
                                            .w_full(),
                                    ),
                            )
                            .child(
                                Button::new("add-project")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Plus)
                                    .label("Add project")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.add_project_from_input(window, cx)
                                    })),
                            ),
                    ),
            )
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

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

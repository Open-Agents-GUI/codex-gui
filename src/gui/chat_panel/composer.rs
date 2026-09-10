use crate::gui::{ChatState, EditingMessage, new_client_user_message_id};
use gpui::{Context, Entity, Window};

use super::{ChatPanel, ComposerContext};

impl ChatPanel {
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

    pub(super) fn save_composer_draft(&self, cx: &mut Context<Self>) {
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

    pub(super) fn sync_composer_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn composer_chat(&self) -> Option<Entity<ChatState>> {
        match &self.composer_context {
            ComposerContext::NewChat => None,
            ComposerContext::Chat(chat) => Some(chat.clone()),
        }
    }

    pub(super) fn active_chat_editing(&self, cx: &mut Context<Self>) -> bool {
        self.composer_chat()
            .is_some_and(|chat| chat.read(cx).editing_message.is_some())
    }

    pub(super) fn send_composer_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn begin_editing_message(
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

    pub(super) fn steer_composer_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn stop_active_turn(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn active_chat_turn_running(&self, cx: &mut Context<Self>) -> bool {
        self.composer_chat()
            .is_some_and(|chat| chat.read(cx).active_turn.is_some())
    }

    pub(super) fn user_message_sending(&self, cx: &mut Context<Self>) -> bool {
        self.composer_chat()
            .is_some_and(|chat| chat.read(cx).user_message_is_sending())
    }
}

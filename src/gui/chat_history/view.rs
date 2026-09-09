use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use codex_app_server_protocol::RequestId;
use gpui::{
    Bounds, Context, Entity, EventEmitter, FollowMode, IntoElement, ParentElement, Pixels, Render,
    Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme as _,
    clipboard::Clipboard,
    text::{MarkdownExtensions, TextView, TextViewState, TextViewStyle},
};

use crate::gui::{ChatState, WorkspaceState, widgets::render_notice};

use super::{
    blocks::BlockId,
    math::MathPlugin,
    motion::{SEND_DESTINATION_TIMEOUT, SendAnimationLaunch},
    projection::build_transcript,
    transcript::{TranscriptBlockStore, TranscriptPlugin, TranscriptSnapshot, node_has_id},
};

pub struct ChatHistory {
    state: Entity<WorkspaceState>,
    active_chat: Option<Entity<ChatState>>,
    _state_subscription: Subscription,
    chat_subscription: Option<Subscription>,
    expanded_turns: HashSet<String>,
    expanded_tool_groups: HashSet<String>,
    transcript: Entity<TextViewState>,
    transcript_extensions: MarkdownExtensions,
    transcript_blocks: TranscriptBlockStore,
    transcript_markdown: String,
    transcript_chat_id: Option<String>,
    block_appeared_at: HashMap<BlockId, Instant>,
    backfill_existing_chat: bool,
    transcript_layout_revision: u64,
    send_animation: Option<SendAnimationLaunch>,
}

#[derive(Clone)]
pub(crate) enum ChatHistoryEvent {
    EditUserMessage {
        chat: Entity<ChatState>,
        turn_id: String,
        previous_turn_id: Option<String>,
        body: String,
    },
    ForkTurn {
        chat: Entity<ChatState>,
        turn_id: String,
    },
    ResolveApproval {
        chat: Entity<ChatState>,
        request_id: RequestId,
        approved: bool,
    },
    AnswerInput {
        chat: Entity<ChatState>,
        request_id: RequestId,
        question_id: String,
        answer: String,
    },
    RejectInput {
        chat: Entity<ChatState>,
        request_id: RequestId,
    },
    DismissNotice {
        chat_id: String,
        notice_id: String,
    },
}

impl ChatHistory {
    pub fn new(state: Entity<WorkspaceState>, cx: &mut Context<Self>) -> Self {
        let active_chat = active_chat_entity(&state, cx);
        let backfill_existing_chat = active_chat
            .as_ref()
            .is_some_and(|chat| chat.read(cx).thread.is_some());
        let chat_subscription = subscribe_to_chat(active_chat.as_ref(), cx);
        let state_subscription = cx.observe(&state, |history, _, cx| {
            history.update_active_chat_subscription(cx);
            cx.notify();
        });
        let transcript = new_transcript(cx);
        let transcript_blocks = Arc::new(RwLock::new(Default::default()));
        let transcript_extensions =
            TranscriptPlugin::new(cx.entity().downgrade(), transcript_blocks.clone())
                .extensions()
                .plugin(MathPlugin::new());

        let mut history = Self {
            state,
            active_chat,
            _state_subscription: state_subscription,
            chat_subscription,
            expanded_turns: HashSet::new(),
            expanded_tool_groups: HashSet::new(),
            transcript,
            transcript_extensions,
            transcript_blocks,
            transcript_markdown: String::new(),
            transcript_chat_id: None,
            block_appeared_at: HashMap::new(),
            backfill_existing_chat,
            transcript_layout_revision: 0,
            send_animation: None,
        };
        history.rebuild_transcript(cx);
        history
    }

    fn update_active_chat_subscription(&mut self, cx: &mut Context<Self>) {
        let active_chat = active_chat_entity(&self.state, cx);
        if self.active_chat == active_chat {
            return;
        }

        let preserve_send_animation = self.send_animation.as_ref().is_some_and(|animation| {
            active_chat.as_ref().is_some_and(|chat| {
                chat.read(cx)
                    .pending_user_message()
                    .is_some_and(|message| message.client_id == animation.client_id)
            })
        });
        if !preserve_send_animation {
            self.send_animation = None;
        }

        self.chat_subscription = subscribe_to_chat(active_chat.as_ref(), cx);
        self.backfill_existing_chat = active_chat
            .as_ref()
            .is_some_and(|chat| chat.read(cx).thread.is_some());
        self.active_chat = active_chat;
        self.expanded_turns.clear();
        self.expanded_tool_groups.clear();
        self.transcript = new_transcript(cx);
        self.transcript_markdown.clear();
        self.transcript_chat_id = None;
        self.block_appeared_at.clear();
        self.transcript_layout_revision = 0;
        self.rebuild_transcript(cx);
    }

    pub(crate) fn begin_send_animation(
        &mut self,
        client_id: String,
        source_bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if cx.reduce_motion() {
            return;
        }

        self.send_animation = Some(SendAnimationLaunch::new(client_id.clone(), source_bounds));

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(SEND_DESTINATION_TIMEOUT)
                .await;
            let _ = this.update(cx, |history, cx| {
                history.expire_waiting_send_animation(&client_id, cx)
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn send_animation_launch(&self, client_id: &str) -> Option<SendAnimationLaunch> {
        self.send_animation
            .as_ref()
            .filter(|animation| animation.client_id == client_id)
            .cloned()
    }

    pub(super) fn block_appeared_at(&self, block_id: &BlockId) -> Option<Instant> {
        self.block_appeared_at.get(block_id).copied()
    }

    fn expire_waiting_send_animation(&mut self, client_id: &str, cx: &mut Context<Self>) {
        let should_expire = self
            .send_animation
            .as_ref()
            .is_some_and(|animation| animation.client_id == client_id && animation.is_waiting());
        if should_expire {
            self.finish_send_animation(client_id, cx);
        }
    }

    pub(super) fn finish_send_animation(&mut self, client_id: &str, cx: &mut Context<Self>) {
        if !self
            .send_animation
            .as_ref()
            .is_some_and(|animation| animation.client_id == client_id)
        {
            return;
        }
        self.send_animation = None;
        cx.notify();
    }

    pub(super) fn toggle_turn(&mut self, turn_id: &str, cx: &mut Context<Self>) {
        if !self.expanded_turns.remove(turn_id) {
            self.expanded_turns.insert(turn_id.to_string());
        }
        self.rebuild_transcript(cx);
        cx.notify();
    }

    pub(super) fn toggle_tools(&mut self, group_id: &str, cx: &mut Context<Self>) {
        if !self.expanded_tool_groups.remove(group_id) {
            self.expanded_tool_groups.insert(group_id.to_string());
        }
        self.rebuild_transcript_remeasuring(Some(BlockId::tool_group(group_id)), cx);
        cx.notify();
    }

    pub(super) fn edit_user_message(
        &mut self,
        turn_id: String,
        previous_turn_id: Option<String>,
        body: String,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.active_chat.clone() else {
            return;
        };
        cx.emit(ChatHistoryEvent::EditUserMessage {
            chat,
            turn_id,
            previous_turn_id,
            body,
        });
    }

    pub(super) fn fork_turn(&mut self, turn_id: String, cx: &mut Context<Self>) {
        let Some(chat) = self.active_chat.clone() else {
            return;
        };
        cx.emit(ChatHistoryEvent::ForkTurn { chat, turn_id });
    }

    pub(super) fn resolve_approval(
        &mut self,
        request_id: RequestId,
        approved: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.active_chat.clone() else {
            return;
        };
        cx.emit(ChatHistoryEvent::ResolveApproval {
            chat,
            request_id,
            approved,
        });
    }

    pub(super) fn answer_input(
        &mut self,
        request_id: RequestId,
        question_id: String,
        answer: String,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.active_chat.clone() else {
            return;
        };
        cx.emit(ChatHistoryEvent::AnswerInput {
            chat,
            request_id,
            question_id,
            answer,
        });
    }

    pub(super) fn reject_input(&mut self, request_id: RequestId, cx: &mut Context<Self>) {
        let Some(chat) = self.active_chat.clone() else {
            return;
        };
        cx.emit(ChatHistoryEvent::RejectInput { chat, request_id });
    }

    pub(super) fn dismiss_notice(&mut self, notice_id: String, cx: &mut Context<Self>) {
        let Some(chat_id) = self
            .active_chat
            .as_ref()
            .map(|chat| chat.read(cx).id.clone())
        else {
            return;
        };
        cx.emit(ChatHistoryEvent::DismissNotice { chat_id, notice_id });
    }

    fn rebuild_transcript(&mut self, cx: &mut Context<Self>) {
        self.rebuild_transcript_remeasuring(None, cx);
    }

    fn rebuild_transcript_remeasuring(
        &mut self,
        changed_block: Option<BlockId>,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.active_chat.as_ref() else {
            self.sync_transcript(
                None,
                TranscriptSnapshot::new(),
                false,
                false,
                changed_block.into_iter().collect(),
                cx,
            );
            return;
        };
        let chat_source = chat.downgrade();
        let (chat_id, snapshot, is_loading, layout_changes) = chat.read_with(cx, |chat, _| {
            (
                chat.id.clone(),
                build_transcript(
                    chat,
                    chat_source,
                    &self.expanded_turns,
                    &self.expanded_tool_groups,
                ),
                chat.is_loading,
                chat.transcript_layout_changes_since(self.transcript_layout_revision),
            )
        });
        self.transcript_layout_revision = layout_changes.revision;
        let mut changed_blocks = layout_changes
            .targets
            .into_iter()
            .filter_map(|target| snapshot.layout_targets.get(&target).cloned())
            .collect::<HashSet<_>>();
        changed_blocks.extend(changed_block);
        self.sync_transcript(
            Some(chat_id),
            snapshot,
            is_loading,
            layout_changes.all,
            changed_blocks,
            cx,
        );
    }

    fn sync_transcript(
        &mut self,
        chat_id: Option<String>,
        snapshot: TranscriptSnapshot,
        chat_is_loading: bool,
        remeasure_all: bool,
        changed_blocks: HashSet<BlockId>,
        cx: &mut Context<Self>,
    ) {
        let same_chat = self.transcript_chat_id == chat_id;
        let now = Instant::now();
        let already_appeared = now.checked_sub(Duration::from_secs(60)).unwrap_or(now);
        let appeared_at = if same_chat && !self.backfill_existing_chat {
            now
        } else {
            already_appeared
        };
        self.block_appeared_at
            .retain(|id, _| snapshot.blocks.contains_key(id));
        for id in snapshot.blocks.keys() {
            self.block_appeared_at
                .entry(id.clone())
                .or_insert(appeared_at);
        }
        if self.backfill_existing_chat && !chat_is_loading {
            self.backfill_existing_chat = false;
        }

        if let Ok(mut blocks) = self.transcript_blocks.write() {
            *blocks = snapshot.blocks;
        }

        let old_markdown = std::mem::replace(&mut self.transcript_markdown, snapshot.markdown);
        self.transcript_chat_id = chat_id;

        if same_chat && old_markdown == self.transcript_markdown {
            // Plugin-backed blocks can change height without changing their stable
            // Markdown marker. Invalidate their cached measurements without resetting
            // ListState, so expanding a tool group keeps the current viewport.
            self.transcript.update(cx, |state, cx| {
                if remeasure_all {
                    state.remeasure_content(cx);
                    return;
                }
                for id in &changed_blocks {
                    state.remeasure_custom_block(|node| node_has_id(node, id), cx);
                }
            });
            return;
        }

        let appended = if same_chat && !old_markdown.is_empty() {
            self.transcript_markdown
                .strip_prefix(&old_markdown)
                .map(str::to_string)
        } else {
            None
        };
        let markdown = self.transcript_markdown.clone();
        self.transcript.update(cx, |state, cx| {
            if let Some(delta) = appended {
                state.push_str(&delta, cx);
            } else if same_chat {
                state.set_text_preserving_layout(&markdown, cx);
                if remeasure_all {
                    state.remeasure_content(cx);
                } else {
                    for id in &changed_blocks {
                        state.remeasure_custom_block(|node| node_has_id(node, id), cx);
                    }
                }
            } else {
                state.set_text(&markdown, cx);
            }
        });
    }
}

impl EventEmitter<ChatHistoryEvent> for ChatHistory {}

fn new_transcript(cx: &mut Context<ChatHistory>) -> Entity<TextViewState> {
    cx.new(|cx| {
        let mut state = TextViewState::markdown("", cx);
        state.set_follow_mode(FollowMode::Tail, cx);
        state.set_append_fade_duration(Some(Duration::from_millis(200)), cx);
        state
    })
}

impl Render for ChatHistory {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.active_chat.is_none() {
            return div()
                .id("message-list")
                .flex()
                .flex_col()
                .size_full()
                .min_w_0()
                .gap_3()
                .overflow_x_hidden()
                .overflow_y_scroll()
                .child(render_notice(
                    "Loading Codex threads from the app server.",
                    cx.theme(),
                ))
                .into_any_element();
        }

        div()
            .id("message-list")
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .child(
                TextView::new(&self.transcript)
                    .markdown_extensions(self.transcript_extensions.clone())
                    .code_block_actions(|code_block, _window, _cx| {
                        Clipboard::new("copy-code-block")
                            .value(code_block.code())
                            .tooltip("Copy code")
                    })
                    .selectable(true)
                    .scrollable(true)
                    .scroll_bottom_padding(gpui::relative(0.3))
                    .content_max_width(px(1000.))
                    .size_full()
                    .min_w_0()
                    .px_5()
                    .text_base()
                    .line_height(px(30.))
                    .style(TextViewStyle {
                        ..Default::default()
                    })
                    .text_color(cx.theme().foreground),
            )
            .into_any_element()
    }
}

fn subscribe_to_chat(
    chat: Option<&Entity<ChatState>>,
    cx: &mut Context<ChatHistory>,
) -> Option<Subscription> {
    chat.map(|chat| {
        cx.observe(chat, |history, _, cx| {
            history.rebuild_transcript(cx);
            cx.notify();
        })
    })
}

fn active_chat_entity(
    state: &Entity<WorkspaceState>,
    cx: &mut Context<ChatHistory>,
) -> Option<Entity<ChatState>> {
    state.read(cx).active_chat_entity(cx)
}

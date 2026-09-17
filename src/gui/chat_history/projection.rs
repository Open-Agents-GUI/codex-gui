use std::{collections::HashSet, sync::Arc, time::Duration};

use codex_app_server_protocol::{ThreadItem, Turn, TurnPlanStepStatus, TurnStatus};
use codex_protocol::models::MessagePhase;
use gpui::WeakEntity;

use crate::gui::{ChatState, PendingUserMessageDelivery, state::TranscriptLayoutTarget};

use super::{
    blocks::{HistoryBlock, UserMessageDelivery, is_tool_item, tool_calls, tools_done},
    transcript::TranscriptSnapshot,
};

pub(super) fn build_transcript(
    chat: &ChatState,
    chat_source: WeakEntity<ChatState>,
    expanded_turns: &HashSet<String>,
    expanded_tool_groups: &HashSet<String>,
    expanded_tool_calls: &HashSet<String>,
) -> TranscriptSnapshot {
    let mut transcript = TranscriptSnapshot::new();

    if let Some(thread) = &chat.thread {
        let mut previous_turn_id = None;
        for turn in &thread.turns {
            append_turn(
                &mut transcript,
                chat,
                &chat_source,
                turn,
                previous_turn_id.as_deref(),
                expanded_turns,
                expanded_tool_groups,
                expanded_tool_calls,
            );
            if !turn
                .items
                .iter()
                .any(|item| matches!(item, ThreadItem::Plan { .. }))
                && let Some(plan) = chat.turn_plans.get(&turn.id)
            {
                let mut body = String::new();
                if let Some(explanation) = &plan.explanation {
                    body.push_str(explanation);
                    body.push_str("\n\n");
                }
                for step in &plan.steps {
                    let marker = match step.status {
                        TurnPlanStepStatus::Pending => "○",
                        TurnPlanStepStatus::InProgress => "◉",
                        TurnPlanStepStatus::Completed => "●",
                    };
                    body.push_str(&format!("{marker} {}\n", step.step));
                }
                transcript.push_block(HistoryBlock::Plan {
                    key: format!("turn-plan-{}", turn.id),
                    body: body.trim_end().to_string().into(),
                    running: plan
                        .steps
                        .iter()
                        .any(|step| matches!(step.status, TurnPlanStepStatus::InProgress)),
                });
                transcript.map_layout_target(
                    TranscriptLayoutTarget::TurnPlan(turn.id.clone()),
                    super::blocks::BlockId::new("plan", &format!("turn-plan-{}", turn.id)),
                );
            }
            previous_turn_id = Some(turn.id.as_str());
        }
    }

    if let Some(message) = chat.pending_user_message() {
        if !message.content.is_empty() {
            let delivery = match &message.delivery {
                PendingUserMessageDelivery::Sending => UserMessageDelivery::Sending,
                PendingUserMessageDelivery::Failed(_) => UserMessageDelivery::Failed,
            };
            transcript.push_block(HistoryBlock::User {
                key: message.client_id.clone(),
                turn_id: None,
                previous_turn_id: None,
                content: message.content.as_slice().into(),
                delivery,
            });
            transcript.map_layout_target(
                TranscriptLayoutTarget::PendingUser(message.client_id.clone()),
                super::blocks::BlockId::new("user", &message.client_id),
            );
        }
    }

    for notice in &chat.notices {
        transcript.push_block(HistoryBlock::Notice {
            key: notice.id.clone(),
            body: notice.body.clone(),
        });
        transcript.map_layout_target(
            TranscriptLayoutTarget::Notice(notice.id.clone()),
            super::blocks::BlockId::new("notice", &notice.id),
        );
    }

    for approval in &chat.pending_approvals {
        transcript.push_block(HistoryBlock::Approval {
            approval: approval.clone(),
        });
        transcript.map_layout_target(
            TranscriptLayoutTarget::Approval(approval.request_id.to_string()),
            super::blocks::BlockId::new("approval", &approval.request_id.to_string()),
        );
    }

    for request in &chat.pending_inputs {
        transcript.push_block(HistoryBlock::InputRequest {
            request: request.clone(),
        });
        transcript.map_layout_target(
            TranscriptLayoutTarget::InputRequest(request.request_id.to_string()),
            super::blocks::BlockId::new("input-request", &request.request_id.to_string()),
        );
    }

    transcript
}

fn append_turn(
    transcript: &mut TranscriptSnapshot,
    chat: &ChatState,
    chat_source: &WeakEntity<ChatState>,
    turn: &Turn,
    previous_turn_id: Option<&str>,
    expanded_turns: &HashSet<String>,
    expanded_tool_groups: &HashSet<String>,
    expanded_tool_calls: &HashSet<String>,
) {
    let turn_is_active = matches!(turn.status, TurnStatus::InProgress);
    let Some(fold) = completed_turn_fold(turn) else {
        append_items(
            transcript,
            chat,
            chat_source,
            &turn.id,
            previous_turn_id,
            &turn.items,
            turn_is_active,
            expanded_tool_groups,
            expanded_tool_calls,
        );
        return;
    };

    append_items(
        transcript,
        chat,
        chat_source,
        &turn.id,
        previous_turn_id,
        &turn.items[..=fold.user_index],
        turn_is_active,
        expanded_tool_groups,
        expanded_tool_calls,
    );

    let expanded = expanded_turns.contains(&turn.id);
    transcript.push_block(HistoryBlock::WorkedSummary {
        turn_id: turn.id.clone(),
        duration: turn_duration(turn),
        expanded,
    });

    if expanded {
        append_items(
            transcript,
            chat,
            chat_source,
            &turn.id,
            previous_turn_id,
            &turn.items[fold.user_index + 1..],
            turn_is_active,
            expanded_tool_groups,
            expanded_tool_calls,
        );
    } else if let Some(final_answer) = turn.items.get(fold.final_index) {
        append_agent(
            transcript,
            chat,
            chat_source,
            &turn.id,
            final_answer,
            &[],
            false,
            expanded_tool_groups,
            expanded_tool_calls,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn append_items(
    transcript: &mut TranscriptSnapshot,
    chat: &ChatState,
    chat_source: &WeakEntity<ChatState>,
    turn_id: &str,
    previous_turn_id: Option<&str>,
    items: &[ThreadItem],
    turn_is_active: bool,
    expanded_tool_groups: &HashSet<String>,
    expanded_tool_calls: &HashSet<String>,
) {
    let mut index = 0;
    while index < items.len() {
        match &items[index] {
            ThreadItem::UserMessage {
                id,
                client_id,
                content,
            } => {
                if !content.is_empty() {
                    transcript.push_block(HistoryBlock::User {
                        key: client_id.clone().unwrap_or_else(|| id.clone()),
                        turn_id: Some(turn_id.to_string()),
                        previous_turn_id: previous_turn_id.map(str::to_string),
                        content: content.as_slice().into(),
                        delivery: UserMessageDelivery::Sent,
                    });
                    transcript.map_layout_target(
                        TranscriptLayoutTarget::Item(id.clone()),
                        super::blocks::BlockId::new("user", client_id.as_deref().unwrap_or(id)),
                    );
                }
                index += 1;
            }
            ThreadItem::AgentMessage { .. } => {
                let tools_end = tool_group_end(items, index + 1);
                let tools = items[index + 1..tools_end]
                    .iter()
                    .filter(|item| is_tool_group_item(item))
                    .collect::<Vec<_>>();
                append_agent(
                    transcript,
                    chat,
                    chat_source,
                    turn_id,
                    &items[index],
                    &tools,
                    tool_group_is_tail(items, index + 1, tools_end, turn_is_active),
                    expanded_tool_groups,
                    expanded_tool_calls,
                );
                index = tools_end;
            }
            ThreadItem::Plan { id, text } => {
                transcript.push_block(HistoryBlock::Plan {
                    key: id.clone(),
                    body: text.clone().into(),
                    running: chat.item_is_streaming(id),
                });
                transcript.map_layout_target(
                    TranscriptLayoutTarget::Item(id.clone()),
                    super::blocks::BlockId::new("plan", id),
                );
                index += 1;
            }
            ThreadItem::HookPrompt { id, fragments } => {
                transcript.push_block(HistoryBlock::HookPrompt {
                    key: id.clone(),
                    body: fragments
                        .iter()
                        .map(|fragment| fragment.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                        .into(),
                });
                transcript.map_layout_target(
                    TranscriptLayoutTarget::Item(id.clone()),
                    super::blocks::BlockId::new("hook-prompt", id),
                );
                index += 1;
            }
            ThreadItem::SubAgentActivity {
                id,
                kind,
                agent_path,
                ..
            } => {
                transcript.push_block(HistoryBlock::Activity {
                    key: id.clone(),
                    title: format!("Sub-agent {kind:?}").into(),
                    body: agent_path.clone().into(),
                    running: false,
                });
                transcript.map_layout_target(
                    TranscriptLayoutTarget::Item(id.clone()),
                    super::blocks::BlockId::new("activity", id),
                );
                index += 1;
            }
            ThreadItem::EnteredReviewMode { id, review } => {
                transcript.push_block(HistoryBlock::Activity {
                    key: id.clone(),
                    title: "Entered review mode".into(),
                    body: review.clone().into(),
                    running: false,
                });
                transcript.map_layout_target(
                    TranscriptLayoutTarget::Item(id.clone()),
                    super::blocks::BlockId::new("activity", id),
                );
                index += 1;
            }
            ThreadItem::ExitedReviewMode { id, review } => {
                transcript.push_block(HistoryBlock::Activity {
                    key: id.clone(),
                    title: "Exited review mode".into(),
                    body: review.clone().into(),
                    running: false,
                });
                transcript.map_layout_target(
                    TranscriptLayoutTarget::Item(id.clone()),
                    super::blocks::BlockId::new("activity", id),
                );
                index += 1;
            }
            ThreadItem::ContextCompaction { id } => {
                transcript.push_block(HistoryBlock::Activity {
                    key: id.clone(),
                    title: "Context compacted".into(),
                    body: "Older conversation context was summarized to continue working.".into(),
                    running: false,
                });
                transcript.map_layout_target(
                    TranscriptLayoutTarget::Item(id.clone()),
                    super::blocks::BlockId::new("activity", id),
                );
                index += 1;
            }
            item if is_tool_group_item(item) => {
                let tools_end = tool_group_end(items, index);
                let tools = items[index..tools_end]
                    .iter()
                    .filter(|item| is_tool_group_item(item))
                    .collect::<Vec<_>>();
                append_tool_group(
                    transcript,
                    chat,
                    chat_source,
                    item.id(),
                    &tools,
                    tool_group_is_tail(items, index, tools_end, turn_is_active),
                    expanded_tool_groups,
                    expanded_tool_calls,
                );
                index = tools_end;
            }
            _ => index += 1,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn append_agent(
    transcript: &mut TranscriptSnapshot,
    chat: &ChatState,
    chat_source: &WeakEntity<ChatState>,
    turn_id: &str,
    item: &ThreadItem,
    tools: &[&ThreadItem],
    tail: bool,
    expanded_tool_groups: &HashSet<String>,
    expanded_tool_calls: &HashSet<String>,
) {
    let ThreadItem::AgentMessage {
        id, text, phase, ..
    } = item
    else {
        return;
    };

    let label = match phase.as_ref() {
        Some(MessagePhase::Commentary) => "",
        _ if chat.item_is_streaming(id) => "Codex is working",
        _ => "Codex",
    };
    if !label.is_empty() {
        transcript.push_block(HistoryBlock::AssistantHeader {
            key: id.clone(),
            label,
        });
        transcript.map_layout_target(
            TranscriptLayoutTarget::Item(id.clone()),
            super::blocks::BlockId::new("assistant-header", id),
        );
    }
    transcript.push_markdown(text);

    if is_final_answer(phase.as_ref()) && !chat.item_is_streaming(id) && !text.trim().is_empty() {
        transcript.push_block(HistoryBlock::AssistantActions {
            key: id.clone(),
            turn_id: turn_id.to_string(),
            body: text.clone().into(),
        });
    }

    if !tools.is_empty() {
        append_tool_group(
            transcript,
            chat,
            chat_source,
            id,
            tools,
            tail,
            expanded_tool_groups,
            expanded_tool_calls,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn append_tool_group(
    transcript: &mut TranscriptSnapshot,
    chat: &ChatState,
    chat_source: &WeakEntity<ChatState>,
    key: &str,
    tools: &[&ThreadItem],
    tail: bool,
    expanded_tool_groups: &HashSet<String>,
    expanded_tool_calls: &HashSet<String>,
) {
    let block_id = super::blocks::BlockId::tool_group(key);
    // Each row owns its own disclosure: reasoning is force-expanded while it
    // streams, everything else opens only when the user has toggled it.
    let row_ids: Arc<[String]> = tools
        .iter()
        .map(|tool| tool.id().to_string())
        .collect::<Vec<_>>()
        .into();
    let expanded_rows: Arc<[bool]> = tools
        .iter()
        .map(|tool| {
            matches!(tool, ThreadItem::Reasoning { id, .. } if chat.item_is_streaming(id))
                || expanded_tool_calls.contains(tool.id())
        })
        .collect::<Vec<_>>()
        .into();
    transcript.push_block(HistoryBlock::ToolGroup {
        key: key.to_string(),
        tools: tool_calls(tools, &chat.tool_progress, chat_source.clone(), |id| {
            chat.item_is_streaming(id)
        }),
        row_ids,
        expanded_rows,
        expanded: expanded_tool_groups.contains(key),
        collapsible: true,
        tail,
    });
    for tool in tools {
        transcript.map_layout_target(
            TranscriptLayoutTarget::Item(tool.id().to_string()),
            block_id.clone(),
        );
    }
}

fn tool_group_end(items: &[ThreadItem], start: usize) -> usize {
    if items
        .get(start)
        .is_none_or(|item| !is_tool_group_item(item))
    {
        return start;
    }

    let mut end = start;
    while let Some(item) = items.get(end) {
        if is_tool_group_item(item) {
            end += 1;
        } else {
            break;
        }
    }
    end
}

fn tool_group_is_tail(
    items: &[ThreadItem],
    start: usize,
    end: usize,
    turn_is_active: bool,
) -> bool {
    turn_is_active && start < end && end == items.len()
}

fn is_tool_group_item(item: &ThreadItem) -> bool {
    is_tool_item(item) || matches!(item, ThreadItem::Reasoning { .. })
}

struct TurnFold {
    user_index: usize,
    final_index: usize,
}

fn completed_turn_fold(turn: &Turn) -> Option<TurnFold> {
    if !matches!(turn.status, TurnStatus::Completed) {
        return None;
    }

    let user_indices = turn
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| matches!(item, ThreadItem::UserMessage { .. }).then_some(index))
        .collect::<Vec<_>>();
    let [user_index] = user_indices.as_slice() else {
        return None;
    };

    let final_index = turn
        .items
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, item)| {
            let ThreadItem::AgentMessage { text, phase, .. } = item else {
                return None;
            };
            (!text.trim().is_empty() && is_final_answer(phase.as_ref())).then_some(index)
        })?;
    if final_index <= *user_index {
        return None;
    }

    let has_progress = turn.items[*user_index + 1..]
        .iter()
        .enumerate()
        .any(|(offset, item)| {
            let index = *user_index + 1 + offset;
            index != final_index
                && (is_tool_item(item)
                    || matches!(item, ThreadItem::AgentMessage { text, .. } if !text.trim().is_empty()))
        });
    if !has_progress {
        return None;
    }

    let tools = turn
        .items
        .iter()
        .filter(|item| is_tool_item(item))
        .collect::<Vec<_>>();
    if !tools.is_empty() && !tools_done(&tools, |_| false) {
        return None;
    }

    Some(TurnFold {
        user_index: *user_index,
        final_index,
    })
}

fn turn_duration(turn: &Turn) -> Duration {
    if let Some(duration_ms) = turn
        .duration_ms
        .and_then(|duration| u64::try_from(duration).ok())
    {
        return Duration::from_millis(duration_ms);
    }

    let seconds = turn
        .started_at
        .zip(turn.completed_at)
        .map(|(started, completed)| completed.saturating_sub(started))
        .and_then(|duration| u64::try_from(duration).ok())
        .unwrap_or_default();
    Duration::from_secs(seconds)
}

fn is_final_answer(phase: Option<&MessagePhase>) -> bool {
    matches!(phase, Some(MessagePhase::FinalAnswer) | None)
}

#[cfg(test)]
mod tests {
    use codex_app_server_protocol::SleepItem;

    use super::*;

    #[test]
    fn tool_group_crosses_completed_empty_reasoning() {
        let items = vec![
            sleep_tool("tool-1"),
            reasoning("reasoning-1", ""),
            sleep_tool("tool-2"),
        ];

        assert_eq!(tool_group_end(&items, 0), items.len());
    }

    #[test]
    fn tool_group_includes_non_empty_reasoning() {
        let items = vec![
            sleep_tool("tool-1"),
            reasoning("reasoning-1", "Still investigating"),
            sleep_tool("tool-2"),
        ];

        assert_eq!(tool_group_end(&items, 0), items.len());
    }

    #[test]
    fn tool_group_includes_streaming_empty_reasoning() {
        let items = vec![
            sleep_tool("tool-1"),
            reasoning("reasoning-1", ""),
            sleep_tool("tool-2"),
        ];

        assert_eq!(tool_group_end(&items, 0), items.len());
    }

    #[test]
    fn last_tool_group_is_tail() {
        let items = vec![sleep_tool("tool-1")];
        let end = tool_group_end(&items, 0);

        assert!(tool_group_is_tail(
            &items, 0, end, /* turn_is_active */ true
        ));
    }

    #[test]
    fn tool_group_before_one_empty_reasoning_is_tail() {
        let items = vec![sleep_tool("tool-1"), reasoning("reasoning-1", "")];
        let end = tool_group_end(&items, 0);

        assert!(tool_group_is_tail(
            &items, 0, end, /* turn_is_active */ true
        ));
    }

    #[test]
    fn tool_group_before_streaming_empty_reasoning_is_tail() {
        let items = vec![sleep_tool("tool-1"), reasoning("reasoning-1", "")];
        let end = tool_group_end(&items, 0);

        assert!(tool_group_is_tail(
            &items, 0, end, /* turn_is_active */ true
        ));
    }

    #[test]
    fn tool_group_ending_in_reasoning_is_tail() {
        let items = vec![
            sleep_tool("tool-1"),
            reasoning("reasoning-1", "Still investigating"),
        ];
        let end = tool_group_end(&items, 0);

        assert!(tool_group_is_tail(
            &items, 0, end, /* turn_is_active */ true
        ));
    }

    #[test]
    fn tool_group_includes_multiple_reasoning_items_at_tail() {
        let items = vec![
            sleep_tool("tool-1"),
            reasoning("reasoning-1", ""),
            reasoning("reasoning-2", ""),
        ];
        let end = tool_group_end(&items, 0);

        assert!(tool_group_is_tail(
            &items, 0, end, /* turn_is_active */ true
        ));
    }

    /// Measure the per-delta cost the transcript pays for one streamed delta.
    ///
    /// `ChatHistory::rebuild_transcript` runs `build_transcript` plus the data
    /// half of `sync_transcript` (block store replacement, `block_appeared_at`
    /// rebuild, Markdown comparison) on the UI thread for *every* app-server
    /// delta, and both are O(history) — a long chat spends more than a frame
    /// budget per delta and the UI stops keeping up. Run with
    /// `cargo test --lib profile_transcript_projection_cost -- --nocapture`.
    #[gpui::test]
    fn profile_transcript_projection_cost(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        use std::collections::HashMap;
        use std::time::Instant;

        use crate::gui::chat_history::blocks::BlockId;

        for turn_count in [40usize, 200, 800, 1600] {
            let chat = cx.new(|_| {
                ChatState::from_thread(
                    transcript_fixture(turn_count),
                    "Thread".into(),
                    "idle".into(),
                    Default::default(),
                )
            });
            let source = chat.downgrade();
            let expanded = HashSet::new();

            let mut store: HashMap<BlockId, HistoryBlock> = HashMap::new();
            let mut appeared: HashMap<BlockId, Instant> = HashMap::new();
            let mut markdown = String::new();
            let now = Instant::now();
            let iterations = 200u64;
            let mut build_total = std::time::Duration::ZERO;
            let mut sync_total = std::time::Duration::ZERO;

            for _ in 0..iterations {
                let started = Instant::now();
                let snapshot = chat.read_with(cx, |chat, _| {
                    build_transcript(chat, source.clone(), &expanded, &expanded, &expanded)
                });
                build_total += started.elapsed();

                let started = Instant::now();
                appeared.retain(|id, _| snapshot.blocks.contains_key(id));
                for id in snapshot.blocks.keys() {
                    appeared.entry(id.clone()).or_insert(now);
                }
                store = snapshot.blocks;
                let previous = std::mem::replace(&mut markdown, snapshot.markdown);
                let unchanged = previous == markdown;
                sync_total += started.elapsed();
                std::hint::black_box((&store, &appeared, unchanged));
            }

            eprintln!(
                "PROFILE transcript turns={turn_count:<5} blocks={:<5} markdown_bytes={:<8} build_us={:<7} sync_us={:<7} per_delta_us={}",
                store.len(),
                markdown.len(),
                build_total.as_micros() / u128::from(iterations),
                sync_total.as_micros() / u128::from(iterations),
                (build_total + sync_total).as_micros() / u128::from(iterations),
            );
        }
    }

    /// A thread of `turn_count` completed turns, each shaped like a real Codex
    /// turn: a user message, an agent message, four command executions, and a
    /// reasoning item.
    fn transcript_fixture(turn_count: usize) -> codex_app_server_protocol::Thread {
        let mut turns = Vec::new();
        for t in 0..turn_count {
            let mut items = vec![
                serde_json::json!({
                    "type": "userMessage",
                    "id": format!("u{t}"),
                    "content": [{"type": "text", "text": "x".repeat(300)}],
                }),
                serde_json::json!({
                    "type": "agentMessage",
                    "id": format!("a{t}"),
                    "text": "y".repeat(4_000),
                }),
            ];
            for k in 0..4 {
                items.push(serde_json::json!({
                    "type": "commandExecution",
                    "id": format!("c{t}-{k}"),
                    "command": format!("rg -n pattern src/turn{t}/call{k}"),
                    "cwd": "/tmp",
                    "source": "agent",
                    "status": "completed",
                    "commandActions": [],
                    "aggregatedOutput": "o".repeat(6_000),
                    "exitCode": 0,
                    "durationMs": 12,
                }));
            }
            items.push(serde_json::json!({
                "type": "reasoning",
                "id": format!("r{t}"),
                "summary": ["z".repeat(3_000)],
                "content": [],
            }));
            turns.push(serde_json::json!({
                "id": format!("turn-{t}"),
                "items": items,
                "itemsView": "full",
                "status": "completed",
                "startedAt": 0,
                "completedAt": 1,
                "durationMs": 1_000,
            }));
        }

        serde_json::from_value(serde_json::json!({
            "id": "thread-1",
            "sessionId": "session-1",
            "preview": "",
            "ephemeral": false,
            "modelProvider": "openai",
            "createdAt": 0,
            "updatedAt": 0,
            "status": {"type": "idle"},
            "cwd": "/tmp",
            "cliVersion": "0.0.0",
            "source": "vscode",
            "turns": turns,
        }))
        .expect("thread fixture")
    }

    fn sleep_tool(id: &str) -> ThreadItem {
        ThreadItem::Sleep(SleepItem {
            id: id.to_string(),
            duration_ms: 1,
        })
    }

    fn reasoning(id: &str, summary: &str) -> ThreadItem {
        ThreadItem::Reasoning {
            id: id.to_string(),
            summary: vec![summary.to_string()],
            content: Vec::new(),
        }
    }
}

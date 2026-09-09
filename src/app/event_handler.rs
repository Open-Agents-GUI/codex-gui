use super::{CodexGui, PendingAgentMessageDeltaLog, thread_mapping::*};
use crate::bridge::BridgeEvent;
use crate::global_state::CodexGlobalState;
use crate::gui::{ChatState, HistoryNotice, ProjectState};
use codex_app_server_protocol::{ServerNotification, Thread, ThreadItem};
use gpui::{AppContext, Context, Entity};
use tracing::debug;

impl CodexGui {
    pub(super) fn apply_bridge_event(&mut self, event: BridgeEvent, cx: &mut Context<Self>) {
        match event {
            BridgeEvent::Notification(notification) => {
                self.apply_server_notification(notification, cx)
            }
            BridgeEvent::ServerRequest(request) => self.apply_server_request(request, cx),
            BridgeEvent::TransportError(message) => self.apply_bridge_error(message, cx),
            BridgeEvent::Lagged { skipped } => {
                self.apply_bridge_error(
                    format!("embedded app-server event consumer dropped {skipped} events"),
                    cx,
                );
            }
        }
    }

    fn apply_server_notification(
        &mut self,
        notification: ServerNotification,
        cx: &mut Context<Self>,
    ) {
        match &notification {
            ServerNotification::AgentMessageDelta(params) => {
                self.accumulate_agent_message_delta_log(&params.delta);
            }
            _ => {
                self.flush_agent_message_delta_log();
                debug!("{:?}", notification);
            }
        }
        match notification {
            ServerNotification::ThreadStarted(params) => {
                self.apply_thread_started(params.thread, cx);
            }
            ServerNotification::ThreadNameUpdated(params) => {
                if let Some(thread_name) = params.thread_name.filter(|name| !name.is_empty()) {
                    self.update_chat_title(&params.thread_id, thread_name, cx);
                }
            }
            ServerNotification::ThreadDeleted(params) => {
                self.remove_thread_from_ui(&params.thread_id, cx);
            }
            ServerNotification::TurnStarted(params) => {
                tracing::info!(
                    thread_id = %params.thread_id,
                    turn_id = %params.turn.id,
                    "turn running"
                );
                self.upsert_thread_turn(&params.thread_id, params.turn.clone(), cx);
                self.update_chat(&params.thread_id, cx, |chat| {
                    chat.start_turn(params.turn.id)
                });
            }
            ServerNotification::ItemStarted(params) => {
                self.start_thread_item(&params.thread_id, &params.turn_id, params.item, cx);
            }
            ServerNotification::AgentMessageDelta(params) => {
                self.append_thread_agent_delta(
                    &params.thread_id,
                    &params.item_id,
                    &params.delta,
                    cx,
                );
            }
            ServerNotification::PlanDelta(params) => {
                self.update_chat(&params.thread_id, cx, |chat| {
                    chat.append_plan_delta(&params.item_id, &params.delta)
                });
            }
            ServerNotification::ReasoningSummaryTextDelta(params) => {
                self.update_chat(&params.thread_id, cx, |chat| {
                    chat.append_reasoning_summary_delta(
                        &params.item_id,
                        params.summary_index,
                        &params.delta,
                    )
                });
            }
            ServerNotification::ReasoningSummaryPartAdded(params) => {
                self.update_chat(&params.thread_id, cx, |chat| {
                    chat.add_reasoning_summary_part(&params.item_id, params.summary_index)
                });
            }
            ServerNotification::ReasoningTextDelta(params) => {
                self.update_chat(&params.thread_id, cx, |chat| {
                    chat.append_reasoning_content_delta(
                        &params.item_id,
                        params.content_index,
                        &params.delta,
                    )
                });
            }
            ServerNotification::CommandExecutionOutputDelta(params) => {
                self.append_thread_command_output_delta(
                    &params.thread_id,
                    &params.item_id,
                    &params.delta,
                    cx,
                );
            }
            ServerNotification::FileChangeOutputDelta(params) => {
                self.append_thread_file_change_output_delta(
                    &params.thread_id,
                    &params.item_id,
                    &params.delta,
                    cx,
                );
            }
            ServerNotification::FileChangePatchUpdated(params) => {
                self.update_thread_file_change_item(
                    &params.thread_id,
                    &params.item_id,
                    params.changes.clone(),
                    cx,
                );
            }
            ServerNotification::McpToolCallProgress(params) => {
                self.update_chat(&params.thread_id, cx, |chat| {
                    chat.append_tool_progress(&params.item_id, params.message)
                });
            }
            ServerNotification::TerminalInteraction(params) => {
                self.update_chat(&params.thread_id, cx, |chat| {
                    chat.append_tool_progress(
                        &params.item_id,
                        format!("Sent terminal input: {}", params.stdin),
                    )
                });
            }
            ServerNotification::ItemCompleted(params) => {
                self.complete_thread_item(&params.thread_id, &params.turn_id, params.item, cx);
            }
            ServerNotification::TurnPlanUpdated(params) => {
                self.update_chat(&params.thread_id, cx, |chat| {
                    chat.update_turn_plan(params.turn_id, params.explanation, params.plan)
                });
            }
            ServerNotification::ThreadStatusChanged(params) => {
                self.update_thread_status(&params.thread_id, params.status.clone(), cx);
                tracing::info!(
                    thread_id = %params.thread_id,
                    status = thread_status_label(&params.status),
                    "thread status changed"
                );
            }
            ServerNotification::TurnCompleted(params) => {
                let turn = params.turn;
                let thread_id = params.thread_id;
                let turn_id = turn.id.clone();
                let turn_error = turn.error.as_ref().map(|error| error.message.clone());
                self.complete_thread_turn(&thread_id, turn, cx);
                self.update_chat(&thread_id, cx, |chat| chat.finish_turn(Some(&turn_id)));
                tracing::info!(thread_id, turn_id, "turn complete");
                if let Some(error) = turn_error {
                    self.apply_thread_error(&thread_id, &turn_id, error, false, cx);
                }
            }
            ServerNotification::Error(params) => {
                self.apply_thread_error(
                    &params.thread_id,
                    &params.turn_id,
                    params.error.message,
                    params.will_retry,
                    cx,
                );
            }
            ServerNotification::Warning(params) => {
                self.apply_targeted_notice(
                    params.thread_id.as_deref(),
                    &format!("warning-{}", uuid::Uuid::new_v4()),
                    params.message,
                    cx,
                );
            }
            ServerNotification::GuardianWarning(_params) => {
                // Normally it's auto approve success
                // We don't want to display this

                // self.apply_targeted_notice(
                //     Some(&params.thread_id),
                //     &format!("guardian-warning-{}", uuid::Uuid::new_v4()),
                //     params.message,
                //     cx,
                // );
            }
            ServerNotification::ConfigWarning(params) => {
                let body = params
                    .details
                    .map(|details| format!("{}\n\n{details}", params.summary))
                    .unwrap_or(params.summary);
                self.apply_targeted_notice(
                    None,
                    &format!("config-warning-{}", uuid::Uuid::new_v4()),
                    body,
                    cx,
                );
            }
            ServerNotification::DeprecationNotice(params) => {
                let body = params
                    .details
                    .map(|details| format!("{}\n\n{details}", params.summary))
                    .unwrap_or(params.summary);
                self.apply_targeted_notice(
                    None,
                    &format!("deprecation-{}", uuid::Uuid::new_v4()),
                    body,
                    cx,
                );
            }
            ServerNotification::ModelRerouted(params) => {
                self.apply_targeted_notice(
                    Some(&params.thread_id),
                    &format!("model-rerouted-{}", params.turn_id),
                    format!(
                        "Model rerouted from {} to {}: {:?}",
                        params.from_model, params.to_model, params.reason
                    ),
                    cx,
                );
            }
            ServerNotification::ModelVerification(params) => {
                self.apply_targeted_notice(
                    Some(&params.thread_id),
                    &format!("model-verification-{}", params.turn_id),
                    format!("Model verification: {:?}", params.verifications),
                    cx,
                );
            }
            ServerNotification::ModelSafetyBufferingUpdated(params) => {
                if params.show_buffering_ui {
                    self.apply_targeted_notice(
                        Some(&params.thread_id),
                        &format!("model-safety-{}", params.turn_id),
                        format!(
                            "Model safety buffering is active for {}. {}",
                            params.model,
                            params.reasons.join("; ")
                        ),
                        cx,
                    );
                } else {
                    self.remove_targeted_notice(
                        &params.thread_id,
                        &format!("model-safety-{}", params.turn_id),
                        cx,
                    );
                }
            }
            ServerNotification::ItemGuardianApprovalReviewStarted(params) => {
                if let Some(item_id) = params.target_item_id {
                    self.update_chat(&params.thread_id, cx, |chat| {
                        chat.append_tool_progress(
                            &item_id,
                            "Automatic approval review started".into(),
                        )
                    });
                } else {
                    self.apply_targeted_notice(
                        Some(&params.thread_id),
                        &format!("guardian-review-{}", params.review_id),
                        format!("Automatic approval review started: {:?}", params.action),
                        cx,
                    );
                }
            }
            ServerNotification::ItemGuardianApprovalReviewCompleted(params) => {
                let mut message = format!(
                    "Automatic approval review completed: {:?}",
                    params.review.status
                );
                if let Some(risk) = params.review.risk_level {
                    message.push_str(&format!(" (risk: {risk:?})"));
                }
                if let Some(rationale) = params.review.rationale {
                    message.push_str(&format!(" — {rationale}"));
                }
                if let Some(item_id) = params.target_item_id {
                    self.update_chat(&params.thread_id, cx, |chat| {
                        chat.append_tool_progress(&item_id, message)
                    });
                } else {
                    self.apply_targeted_notice(
                        Some(&params.thread_id),
                        &format!("guardian-review-{}", params.review_id),
                        message,
                        cx,
                    );
                }
            }
            ServerNotification::HookStarted(params) => {
                self.apply_targeted_notice(
                    Some(&params.thread_id),
                    &format!("hook-{}", params.run.id),
                    format!("Hook started: {:?}", params.run.event_name),
                    cx,
                );
            }
            ServerNotification::HookCompleted(params) => {
                self.apply_targeted_notice(
                    Some(&params.thread_id),
                    &format!("hook-{}", params.run.id),
                    format!(
                        "Hook completed: {:?} — {:?}",
                        params.run.event_name, params.run.status
                    ),
                    cx,
                );
            }
            ServerNotification::ThreadArchived(params) => {
                self.remove_thread_from_ui(&params.thread_id, cx);
            }
            ServerNotification::ThreadClosed(params) => {
                self.finish_thread_turn(&params.thread_id, None, cx);
            }
            ServerNotification::ServerRequestResolved(params) => {
                self.remove_pending_approval(&params.thread_id, &params.request_id, cx);
                self.remove_pending_input(&params.thread_id, &params.request_id, cx);
            }
            ServerNotification::McpServerOauthLoginCompleted(params) => {
                let body = if params.success {
                    format!("Signed in to MCP server {}.", params.name)
                } else {
                    format!(
                        "MCP server {} sign-in failed: {}",
                        params.name,
                        params.error.unwrap_or_else(|| "unknown error".into())
                    )
                };
                self.apply_targeted_notice(
                    params.thread_id.as_deref(),
                    &format!("mcp-oauth-{}", params.name),
                    body,
                    cx,
                );
            }
            ServerNotification::McpServerStatusUpdated(_params) => {
                // We don't want to display MCP startup failures

                // if params.error.is_some() || params.failure_reason.is_some() {
                //     self.apply_targeted_notice(
                //         params.thread_id.as_deref(),
                //         &format!("mcp-status-{}", params.name),
                //         format!(
                //             "MCP server {}: {:?}. {}",
                //             params.name,
                //             params.status,
                //             params.error.unwrap_or_default()
                //         ),
                //         cx,
                //     );
                // }
            }
            ServerNotification::WindowsWorldWritableWarning(params) => {
                let mut body = format!(
                    "Windows sandbox warning: world-writable paths detected: {}",
                    params.sample_paths.join(", ")
                );
                if params.extra_count > 0 {
                    body.push_str(&format!(" and {} more", params.extra_count));
                }
                if params.failed_scan {
                    body.push_str(". The scan did not complete.");
                }
                self.apply_targeted_notice(None, "windows-sandbox-warning", body, cx);
            }

            // These notifications support features that this client does not expose yet, or are
            // internal bookkeeping with no user-visible transcript representation.
            ServerNotification::ThreadUnarchived(_)
            | ServerNotification::SkillsChanged(_)
            | ServerNotification::ThreadGoalUpdated(_)
            | ServerNotification::ThreadGoalCleared(_)
            | ServerNotification::EnvironmentConnected(_)
            | ServerNotification::EnvironmentDisconnected(_)
            | ServerNotification::ThreadSettingsUpdated(_)
            | ServerNotification::ThreadTokenUsageUpdated(_)
            | ServerNotification::TurnDiffUpdated(_)
            | ServerNotification::RawResponseItemCompleted(_)
            | ServerNotification::RawResponseCompleted(_)
            | ServerNotification::CommandExecOutputDelta(_)
            | ServerNotification::ProcessOutputDelta(_)
            | ServerNotification::ProcessExited(_)
            | ServerNotification::AccountUpdated(_)
            | ServerNotification::AccountRateLimitsUpdated(_)
            | ServerNotification::AppListUpdated(_)
            | ServerNotification::RemoteControlStatusChanged(_)
            | ServerNotification::ExternalAgentConfigImportProgress(_)
            | ServerNotification::ExternalAgentConfigImportCompleted(_)
            | ServerNotification::FsChanged(_)
            | ServerNotification::ContextCompacted(_)
            | ServerNotification::TurnModerationMetadata(_)
            | ServerNotification::FuzzyFileSearchSessionUpdated(_)
            | ServerNotification::FuzzyFileSearchSessionCompleted(_)
            | ServerNotification::ThreadRealtimeStarted(_)
            | ServerNotification::ThreadRealtimeItemAdded(_)
            | ServerNotification::ThreadRealtimeTranscriptDelta(_)
            | ServerNotification::ThreadRealtimeTranscriptDone(_)
            | ServerNotification::ThreadRealtimeOutputAudioDelta(_)
            | ServerNotification::ThreadRealtimeSdp(_)
            | ServerNotification::ThreadRealtimeError(_)
            | ServerNotification::ThreadRealtimeClosed(_)
            | ServerNotification::WindowsSandboxSetupCompleted(_)
            | ServerNotification::AccountLoginCompleted(_) => {}
        }
    }

    fn accumulate_agent_message_delta_log(&mut self, delta: &str) {
        if let Some(pending) = self.pending_agent_message_delta_log.as_mut() {
            pending.delta.push_str(delta);
            pending.count += 1;
            return;
        }

        self.pending_agent_message_delta_log = Some(PendingAgentMessageDeltaLog {
            delta: delta.to_owned(),
            count: 1,
        });
    }

    fn flush_agent_message_delta_log(&mut self) {
        let Some(pending) = self.pending_agent_message_delta_log.take() else {
            return;
        };

        debug!(
            delta_count = pending.count,
            delta = ?pending.delta,
            "AgentMessageDelta"
        );
    }

    pub(super) fn update_chat(
        &self,
        thread_id: &str,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut ChatState),
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            tracing::warn!(thread_id, "notification targeted an unloaded thread");
            return;
        };
        chat.update(cx, |chat, cx| {
            update(chat);
            cx.notify();
        });
    }

    fn apply_targeted_notice(
        &self,
        thread_id: Option<&str>,
        notice_id: &str,
        body: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(thread_id) = thread_id {
            self.append_notice(thread_id, format!("{notice_id}-{thread_id}"), body, cx);
        } else if let Some(chat) = self.active_chat_entity(cx) {
            let thread_id = chat.read(cx).id.clone();
            self.append_notice(&thread_id, notice_id.to_string(), body, cx);
        } else {
            tracing::warn!(notice_id, %body, "notification had no chat to display in");
        }
    }

    fn remove_targeted_notice(&self, thread_id: &str, notice_id: &str, cx: &mut Context<Self>) {
        let id = format!("{notice_id}-{thread_id}");
        self.update_chat(thread_id, cx, |chat| chat.remove_notice(&id));
    }

    pub(super) fn apply_thread_error(
        &self,
        thread_id: &str,
        turn_id: &str,
        message: String,
        will_retry: bool,
        cx: &mut Context<Self>,
    ) {
        tracing::error!(thread_id, turn_id, will_retry, error = %message, "turn error");
        if !will_retry {
            self.finish_thread_turn(thread_id, Some(turn_id), cx);
        }
        let body = if will_retry {
            format!("{message}\n\nCodex will retry automatically.")
        } else {
            message
        };
        self.append_notice(thread_id, format!("turn-error-{turn_id}"), body, cx);
    }

    fn finish_thread_turn(&self, thread_id: &str, turn_id: Option<&str>, cx: &mut Context<Self>) {
        self.update_chat(thread_id, cx, |chat| chat.finish_turn(turn_id));
    }

    pub(super) fn remove_pending_approval(
        &self,
        thread_id: &str,
        request_id: &codex_app_server_protocol::RequestId,
        cx: &mut Context<Self>,
    ) {
        self.update_chat(thread_id, cx, |chat| chat.remove_approval(request_id));
    }

    pub(super) fn remove_pending_input(
        &self,
        thread_id: &str,
        request_id: &codex_app_server_protocol::RequestId,
        cx: &mut Context<Self>,
    ) {
        self.update_chat(thread_id, cx, |chat| chat.remove_input_request(request_id));
    }

    pub(super) fn apply_thread_started(&mut self, thread: Thread, cx: &mut Context<Self>) {
        let thread_id = thread.id.clone();
        let updated_at = thread.updated_at;
        let cwd = thread.cwd.to_string_lossy().into_owned();
        let known_projectless = CodexGlobalState::load()
            .map(|state| state.projectless_thread_ids().contains(&thread_id))
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to classify started thread from global state");
                false
            });
        let settings = self.state.read(cx).new_chat_settings.clone();
        let selected_chat = self.active_chat_entity(cx);
        let existing = self.find_chat_entity(&thread_id, cx);
        let chat = if let Some(chat) = existing {
            let title = thread_title(thread.name.as_deref(), &thread.preview);
            let subtitle = format!(
                "{} - {}",
                thread_status_label(&thread.status),
                thread.cwd.display()
            );
            chat.update(cx, |chat, cx| {
                chat.adopt_thread(thread, title.into(), subtitle.into());
                cx.notify();
            });
            chat.clone()
        } else {
            chat_entity_from_thread(thread, settings, cx)
        };

        if known_projectless {
            let projects = self.state.read(cx).projects.clone();
            for project in projects {
                project.update(cx, |project, cx| {
                    project.chats.retain(|candidate| candidate != &chat);
                    cx.notify();
                });
            }
            if let Err(error) = CodexGlobalState::add_projectless_thread(&thread_id) {
                chat.update(cx, |chat, cx| {
                    chat.upsert_notice(
                        "global-state-write-error".into(),
                        format!("Failed to persist this project-less chat: {error}"),
                    );
                    cx.notify();
                });
            }
            self.state.update(cx, |state, cx| {
                if !state
                    .projectless_chats
                    .iter()
                    .any(|candidate| candidate == &chat)
                {
                    state.projectless_chats.insert(0, chat.clone());
                }
                if let Some(selected_chat) = selected_chat.as_ref() {
                    state.select_chat_entity(selected_chat, cx);
                }
                cx.notify();
            });
        } else {
            if let Some(project) = self.ensure_project_for_cwd(&cwd, cx) {
                project.update(cx, |project, cx| {
                    project.mark_thread_updated_at(updated_at);
                    if !project.chats.iter().any(|candidate| candidate == &chat) {
                        project.upsert_chat(chat, &thread_id, cx);
                    }
                    cx.notify();
                });
            }
            self.state.update(cx, |state, cx| {
                state.sort_projects_by_recent_activity(cx);
                if let Some(selected_chat) = selected_chat.as_ref() {
                    state.select_chat_entity(selected_chat, cx);
                }
                cx.notify();
            });
        }
        tracing::info!(thread_id, "thread ready");
    }

    pub(super) fn apply_pending_thread_started(
        &mut self,
        pending_chat: &Entity<ChatState>,
        project: Option<&Entity<ProjectState>>,
        projectless: bool,
        thread: Thread,
        cx: &mut Context<Self>,
    ) {
        let thread_id = thread.id.clone();
        let updated_at = thread.updated_at;
        let selected_chat = self.active_chat_entity(cx);
        let title = thread_title(thread.name.as_deref(), &thread.preview);
        let subtitle = format!(
            "{} - {}",
            thread_status_label(&thread.status),
            thread.cwd.display()
        );
        pending_chat.update(cx, |chat, cx| {
            chat.adopt_thread(thread, title.into(), subtitle.into());
            cx.notify();
        });

        let projects = self.state.read(cx).projects.clone();
        for candidate_project in projects {
            candidate_project.update(cx, |state, cx| {
                state
                    .chats
                    .retain(|chat| chat == pending_chat || chat.read(cx).id.as_str() != thread_id);
                cx.notify();
            });
        }
        self.state.update(cx, |state, cx| {
            state
                .projectless_chats
                .retain(|chat| chat == pending_chat || chat.read(cx).id.as_str() != thread_id);
            cx.notify();
        });

        if projectless {
            if let Err(error) = CodexGlobalState::add_projectless_thread(&thread_id) {
                pending_chat.update(cx, |chat, cx| {
                    chat.upsert_notice(
                        "global-state-write-error".into(),
                        format!("Failed to persist this project-less chat: {error}"),
                    );
                    cx.notify();
                });
            }
            self.state.update(cx, |state, cx| {
                if !state
                    .projectless_chats
                    .iter()
                    .any(|chat| chat == pending_chat)
                {
                    state.projectless_chats.insert(0, pending_chat.clone());
                }
                if let Some(selected_chat) = selected_chat.as_ref() {
                    state.select_chat_entity(selected_chat, cx);
                }
                cx.notify();
            });
        } else if let Some(project) = project {
            project.update(cx, |state, cx| {
                if !state.chats.iter().any(|chat| chat == pending_chat) {
                    state.chats.insert(0, pending_chat.clone());
                }
                state.mark_thread_updated_at(updated_at);
                cx.notify();
            });
            self.state.update(cx, |state, cx| {
                state.sort_projects_by_recent_activity(cx);
                if let Some(selected_chat) = selected_chat.as_ref() {
                    state.select_chat_entity(selected_chat, cx);
                }
                cx.notify();
            });
        }

        tracing::info!(thread_id, "pending thread ready");
        if let Some((client_user_message_id, text)) =
            pending_chat.read(cx).pending_user_message_request()
        {
            let settings = pending_chat.read(cx).settings.clone();
            let bridge = self.bridge.clone();
            cx.spawn(async move |this, cx| {
                let result = bridge
                    .send_turn(
                        thread_id.clone(),
                        client_user_message_id.clone(),
                        text,
                        settings,
                    )
                    .await
                    .map(|_| ());
                let _ = this.update(cx, |view, cx| {
                    view.apply_user_submission_result(
                        &thread_id,
                        &client_user_message_id,
                        true,
                        result,
                        cx,
                    )
                });
            })
            .detach();
        }
    }

    pub(super) fn apply_forked_thread(
        &mut self,
        source_chat: &Entity<ChatState>,
        thread: Thread,
        cx: &mut Context<Self>,
    ) {
        let thread_id = thread.id.clone();
        let cwd = thread.cwd.to_string_lossy().into_owned();
        let settings = source_chat.read(cx).settings.clone();
        let selected_chat = self.active_chat_entity(cx);
        let should_select = selected_chat.as_ref() == Some(source_chat);
        let fork_chat = if let Some(existing) = self.find_chat_entity(&thread_id, cx) {
            let title = thread_title(thread.name.as_deref(), &thread.preview);
            let subtitle = format!(
                "{} - {}",
                thread_status_label(&thread.status),
                thread.cwd.display()
            );
            existing.update(cx, |chat, cx| {
                chat.settings = settings;
                chat.adopt_thread(thread, title.into(), subtitle.into());
                cx.notify();
            });
            existing
        } else {
            chat_entity_from_thread(thread, settings, cx)
        };
        let source_is_projectless = self
            .state
            .read(cx)
            .projectless_chats
            .iter()
            .any(|chat| chat == source_chat);
        if source_is_projectless {
            let _ = CodexGlobalState::add_projectless_thread(&thread_id);
            let projects = self.state.read(cx).projects.clone();
            for project in projects {
                project.update(cx, |project, cx| {
                    project.chats.retain(|chat| chat != &fork_chat);
                    cx.notify();
                });
            }
            self.state.update(cx, |state, cx| {
                if !state
                    .projectless_chats
                    .iter()
                    .any(|chat| chat == &fork_chat)
                {
                    state.projectless_chats.insert(0, fork_chat.clone());
                }
                if should_select {
                    state.select_chat_entity(&fork_chat, cx);
                } else if let Some(selected_chat) = selected_chat.as_ref() {
                    state.select_chat_entity(selected_chat, cx);
                }
                cx.notify();
            });
        } else if let Some(project) = self.ensure_project_for_cwd(&cwd, cx) {
            project.update(cx, |project, cx| {
                if !project.chats.iter().any(|chat| chat == &fork_chat) {
                    project.chats.insert(0, fork_chat.clone());
                }
                cx.notify();
            });
            self.state.update(cx, |state, cx| {
                if should_select {
                    state.select_chat_entity(&fork_chat, cx);
                } else if let Some(selected_chat) = selected_chat.as_ref() {
                    state.select_chat_entity(selected_chat, cx);
                }
                cx.notify();
            });
        }
    }

    pub(super) fn replace_thread_in_ui(
        &mut self,
        source_chat: &Entity<ChatState>,
        replacement: Thread,
        settings: crate::gui::ChatSettings,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ChatState>> {
        let source_thread_id = source_chat.read(cx).id.clone();
        let replacement_thread_id = replacement.id.clone();
        let updated_at = replacement.updated_at;
        let replacement_chat = chat_entity_from_thread(replacement, settings, cx);
        let was_selected = self.active_chat_entity(cx).as_ref() == Some(source_chat);

        let projectless_index = self
            .state
            .read(cx)
            .projectless_chats
            .iter()
            .position(|chat| chat == source_chat);
        if projectless_index.is_some() {
            self.state.update(cx, |state, cx| {
                state
                    .projectless_chats
                    .retain(|chat| chat.read(cx).id != replacement_thread_id);
                if let Some(source_index) = state
                    .projectless_chats
                    .iter()
                    .position(|chat| chat == source_chat)
                {
                    state.projectless_chats[source_index] = replacement_chat.clone();
                    if was_selected {
                        state.select_projectless_chat(source_index);
                    }
                }
                cx.notify();
            });
        } else {
            let location = self.state.read(cx).projects.iter().enumerate().find_map(
                |(project_index, project)| {
                    project
                        .read(cx)
                        .chats
                        .iter()
                        .position(|chat| chat == source_chat)
                        .map(|chat_index| (project_index, chat_index, project.clone()))
                },
            );
            let Some((project_index, _chat_index, project)) = location else {
                tracing::warn!(
                    source_thread_id,
                    replacement_thread_id,
                    "source thread missing while applying edited replacement"
                );
                return None;
            };
            project.update(cx, |project, cx| {
                project
                    .chats
                    .retain(|chat| chat.read(cx).id != replacement_thread_id);
                if let Some(source_index) =
                    project.chats.iter().position(|chat| chat == source_chat)
                {
                    project.chats[source_index] = replacement_chat.clone();
                }
                project.mark_thread_updated_at(updated_at);
                cx.notify();
            });
            if was_selected {
                self.state.update(cx, |state, cx| {
                    state.active_project = project_index;
                    if let Some(chat_index) = project
                        .read(cx)
                        .chats
                        .iter()
                        .position(|chat| chat == &replacement_chat)
                    {
                        state.select_chat(chat_index);
                    }
                    cx.notify();
                });
            }
        }

        tracing::info!(
            source_thread_id,
            thread_id = replacement_thread_id,
            "thread replaced in UI"
        );
        Some(replacement_chat)
    }

    pub(super) fn remove_thread_from_ui(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        let active_thread_id = self
            .active_chat_entity(cx)
            .map(|chat| chat.read(cx).id.clone());
        let projects = self.state.read(cx).projects.clone();
        for project in projects {
            project.update(cx, |project, cx| {
                let old_len = project.chats.len();
                project.chats.retain(|chat| chat.read(cx).id != thread_id);
                if project.chats.len() != old_len {
                    cx.notify();
                }
            });
        }

        self.state.update(cx, |state, cx| {
            state
                .projectless_chats
                .retain(|chat| chat.read(cx).id != thread_id);
            if let Some(active_thread_id) = active_thread_id {
                if let Some(index) = state
                    .projectless_chats
                    .iter()
                    .position(|chat| chat.read(cx).id == active_thread_id)
                {
                    state.select_projectless_chat(index);
                } else if let Some((project_index, chat_index)) = state
                    .projects
                    .iter()
                    .enumerate()
                    .find_map(|(project_index, project)| {
                        project
                            .read(cx)
                            .chats
                            .iter()
                            .position(|chat| chat.read(cx).id == active_thread_id)
                            .map(|chat_index| (project_index, chat_index))
                    })
                {
                    state.active_project = project_index;
                    state.select_chat(chat_index);
                } else {
                    state.select_first_chat();
                }
            }
            cx.notify();
        });
        tracing::info!(thread_id, "removed deleted thread from UI");
    }

    pub(super) fn apply_thread_resumed(
        &mut self,
        target: &Entity<ChatState>,
        thread: Thread,
        cx: &mut Context<Self>,
    ) {
        let thread_id = thread.id.clone();
        if target.read(cx).id != thread_id {
            tracing::warn!(
                expected = target.read(cx).id,
                thread_id,
                "resume returned wrong thread"
            );
            return;
        }
        let title = thread_title(thread.name.as_deref(), &thread.preview);
        let subtitle = format!(
            "{} - {}",
            thread_status_label(&thread.status),
            thread.cwd.display()
        );
        target.update(cx, |chat, cx| {
            chat.adopt_thread(thread, title.into(), subtitle.into());
            cx.notify();
        });
        tracing::info!(thread_id, "thread loaded");
    }

    pub(super) fn apply_bridge_error(&mut self, message: String, cx: &mut Context<Self>) {
        tracing::error!(error = %message, "codex app-server error");
        if let Some(chat) = self.active_chat_entity(cx) {
            let thread_id = chat.read(cx).id.clone();
            self.append_notice(&thread_id, format!("bridge-error-{thread_id}"), message, cx);
        } else if let Some(project) = self.active_project_entity(cx) {
            let chat = cx.new(|_| {
                ChatState::new(
                    "bridge-error".into(),
                    "Bridge error".into(),
                    message.clone().into(),
                    vec![HistoryNotice {
                        id: "bridge-error".into(),
                        body: message.into(),
                    }],
                )
            });
            project.update(cx, |project, cx| {
                project.append_chat(chat);
                cx.notify();
            });
        }
    }

    pub(super) fn active_project_entity(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ProjectState>> {
        self.state.read(cx).active_project()
    }

    pub(super) fn active_chat_entity(&self, cx: &mut Context<Self>) -> Option<Entity<ChatState>> {
        self.state.read(cx).active_chat_entity(cx)
    }

    fn ensure_project_for_cwd(
        &self,
        cwd: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ProjectState>> {
        let existing = self
            .state
            .read(cx)
            .projects
            .iter()
            .position(|project| project.read(cx).path.as_ref() == cwd);
        if let Some(index) = existing {
            return self.state.read(cx).projects.get(index).cloned();
        }

        let name = project_name_from_path(cwd);
        let empty_chat = empty_chat_entity(cx);
        let project = cx.new(|_| ProjectState::new(name.into(), cwd.into(), vec![empty_chat]));
        self.state.update(cx, |state, cx| {
            state.projects.push(project.clone());
            state.sort_projects_by_recent_activity(cx);
            cx.notify();
        });
        Some(project)
    }

    pub(super) fn find_chat_entity(
        &self,
        thread_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ChatState>> {
        let state = self.state.read(cx);
        if let Some(chat) = state
            .projectless_chats
            .iter()
            .find(|chat| chat.read(cx).id == thread_id)
        {
            return Some(chat.clone());
        }
        for project in &state.projects {
            let chats = project.read(cx).chats.clone();
            for chat in chats {
                let is_match = chat.read(cx).id == thread_id;
                if is_match {
                    return Some(chat);
                }
            }
        }
        None
    }

    pub(super) fn append_notice(
        &self,
        thread_id: &str,
        notice_id: String,
        body: String,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.upsert_notice(notice_id, body);
            cx.notify();
        });
    }

    fn update_chat_title(&self, thread_id: &str, title: String, cx: &mut Context<Self>) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.set_title(title);
            cx.notify();
        });
    }

    fn upsert_thread_turn(
        &self,
        thread_id: &str,
        turn: codex_app_server_protocol::Turn,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.upsert_turn(turn);
            cx.notify();
        });
    }

    fn complete_thread_turn(
        &self,
        thread_id: &str,
        turn: codex_app_server_protocol::Turn,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.complete_turn(turn);
            cx.notify();
        });
    }

    fn start_thread_item(
        &self,
        thread_id: &str,
        turn_id: &str,
        item: ThreadItem,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.start_item(turn_id, item);
            cx.notify();
        });
    }

    fn complete_thread_item(
        &self,
        thread_id: &str,
        turn_id: &str,
        item: ThreadItem,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.complete_item(turn_id, item);
            cx.notify();
        });
    }

    fn append_thread_agent_delta(
        &self,
        thread_id: &str,
        item_id: &str,
        delta: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.append_agent_text_delta(item_id, delta);
            cx.notify();
        });
    }

    fn append_thread_command_output_delta(
        &self,
        thread_id: &str,
        item_id: &str,
        delta: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.append_command_output_delta(item_id, delta);
            cx.notify();
        });
    }

    fn append_thread_file_change_output_delta(
        &self,
        thread_id: &str,
        item_id: &str,
        delta: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.append_file_change_output_delta(item_id, delta);
            cx.notify();
        });
    }

    fn update_thread_file_change_item(
        &self,
        thread_id: &str,
        item_id: &str,
        changes: Vec<codex_app_server_protocol::FileUpdateChange>,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.update_file_change_item(item_id, changes);
            cx.notify();
        });
    }

    fn update_thread_status(
        &self,
        thread_id: &str,
        status: codex_app_server_protocol::ThreadStatus,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.find_chat_entity(thread_id, cx) else {
            return;
        };
        chat.update(cx, |chat, cx| {
            chat.set_thread_status(status);
            cx.notify();
        });
    }
}

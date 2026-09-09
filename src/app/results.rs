//! Read RPC responses and update view state accordingly

use super::{
    CodexGui,
    thread_mapping::{chat_entity_from_thread, empty_chat_entity},
};
use crate::{
    bridge::{BridgeError, PersistedChatSettings},
    global_state::CodexGlobalState,
    gui::{ChatState, ModelOption, PermissionProfileOption, ProjectState},
};
use codex_app_server_protocol::Thread;
use gpui::{AppContext, Context, Entity};
use std::collections::{HashMap, HashSet};

impl CodexGui {
    pub(super) fn apply_initialize_result(
        &mut self,
        result: Result<(), BridgeError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => {
                tracing::info!("started embedded codex app-server");
                self.load_startup_data(cx);
            }
            Err(err) => self.apply_bridge_error(err.to_string(), cx),
        }
    }

    pub(super) fn apply_threads_result(
        &mut self,
        result: Result<Vec<Thread>, BridgeError>,
        cx: &mut Context<Self>,
    ) {
        let threads = match result {
            Ok(result) => result,
            Err(err) => {
                self.apply_bridge_error(err.to_string(), cx);
                self.state.update(cx, |state, cx| {
                    state.threads_loaded = true;
                    cx.notify();
                });
                return;
            }
        };

        let thread_count = threads.len();
        let projectless_ids = projectless_thread_ids();
        let default_settings = self.state.read(cx).new_chat_settings.clone();
        let mut project_paths = Vec::new();
        let mut threads_by_project = HashMap::<String, Vec<Thread>>::new();
        let mut projectless_threads = Vec::new();
        let mut default_thread: Option<(i64, String)> = None;
        for thread in threads {
            let is_newest = default_thread
                .as_ref()
                .is_none_or(|(updated_at, _)| thread.updated_at > *updated_at);
            if is_newest {
                default_thread = Some((thread.updated_at, thread.id.clone()));
            }
            if projectless_ids.contains(&thread.id) {
                projectless_threads.push(thread);
                continue;
            }
            let cwd = thread.cwd.to_string_lossy().into_owned();
            if !threads_by_project.contains_key(&cwd) {
                project_paths.push(cwd.clone());
            }
            threads_by_project.entry(cwd).or_default().push(thread);
        }

        let existing_projects = self.state.read(cx).projects.clone();
        let existing_paths = existing_projects
            .iter()
            .map(|project| project.read(cx).path.to_string())
            .collect::<HashSet<_>>();

        for project in existing_projects {
            let cwd = project.read(cx).path.to_string();
            let (chats, latest_thread_updated_at) = loaded_chats(
                threads_by_project.remove(&cwd).unwrap_or_default(),
                default_settings.clone(),
                cx,
            );
            project.update(cx, |project, cx| {
                project.replace_loaded_chats(chats, latest_thread_updated_at);
                cx.notify();
            });
        }

        let mut discovered_projects = Vec::new();
        for cwd in project_paths {
            if existing_paths.contains(&cwd) {
                continue;
            }
            let Some(threads) = threads_by_project.remove(&cwd) else {
                continue;
            };
            let (chats, latest_thread_updated_at) =
                loaded_chats(threads, default_settings.clone(), cx);
            let name = super::thread_mapping::project_name_from_path(&cwd);
            discovered_projects.push(cx.new(|_| {
                let mut project = ProjectState::new(name.into(), cwd.into(), chats);
                project.latest_thread_updated_at = latest_thread_updated_at;
                project
            }));
        }

        let projectless_chats = if projectless_threads.is_empty() {
            Vec::new()
        } else {
            loaded_chats(projectless_threads, default_settings, cx).0
        };

        self.state.update(cx, |state, cx| {
            state.projects.extend(discovered_projects);
            state.projectless_chats = projectless_chats;
            state.sort_projects_by_recent_activity(cx);
            state.select_first_chat();
            state.threads_loaded = true;
            cx.notify();
        });

        let project_count = self.state.read(cx).projects.len();
        let projectless_count = self.state.read(cx).projectless_chats.len();
        tracing::info!(
            thread_count,
            project_count,
            projectless_count,
            "loaded threads from app server"
        );
        let can_resume_default = !self.window_state.read(cx).new_chat_open;
        if can_resume_default && let Some((_, thread_id)) = default_thread {
            if projectless_ids.contains(&thread_id) {
                self.state.update(cx, |state, cx| {
                    state.active_projectless_chat = state
                        .projectless_chats
                        .iter()
                        .position(|chat| chat.read(cx).id == thread_id);
                    cx.notify();
                });
            }
            let Some(chat) = self.find_chat_entity(&thread_id, cx) else {
                return;
            };
            tracing::info!(thread_id, "loading thread");
            chat.update(cx, |chat, cx| {
                chat.is_loading = true;
                cx.notify();
            });
            let bridge = self.bridge.clone();
            cx.spawn(async move |this, cx| {
                let result = bridge.resume_thread(thread_id.clone()).await;
                let _ = this.update(cx, |view, cx| {
                    view.apply_thread_resumed_result(&chat, &thread_id, result, cx)
                });
            })
            .detach();
        }
    }

    pub(super) fn apply_models_result(
        &mut self,
        result: Result<Vec<ModelOption>, BridgeError>,
        settings_result: Result<PersistedChatSettings, BridgeError>,
        cx: &mut Context<Self>,
    ) {
        let persisted_settings = match settings_result {
            Ok(settings) => Some(settings),
            Err(error) => {
                self.apply_bridge_error(error.to_string(), cx);
                None
            }
        };
        match result {
            Ok(models) => {
                self.state.update(cx, |state, cx| {
                    state.set_available_models(models);
                    if let Some(settings) = persisted_settings {
                        if let Some(reviewer) = settings.approvals_reviewer {
                            state.set_approvals_reviewer(reviewer);
                        }
                        if let Some(model) = settings.model.filter(|model| {
                            state
                                .available_models
                                .iter()
                                .any(|option| option.id == *model)
                        }) {
                            state.set_model(model);
                        }
                        if let Some(effort) = settings.effort
                            && let Some(effort) = resolve_reasoning_effort(
                                &effort,
                                &state.new_chat_settings.model,
                                &state.available_models,
                            )
                        {
                            state.set_effort(effort);
                        }
                    }
                    cx.notify();
                });
            }
            Err(err) => self.apply_bridge_error(err.to_string(), cx),
        }
    }

    pub(super) fn apply_permission_profiles_result(
        &mut self,
        result: Result<Vec<PermissionProfileOption>, BridgeError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(profiles) => {
                self.state.update(cx, |state, cx| {
                    state.set_permission_profiles(profiles);
                    cx.notify();
                });
            }
            Err(err) => self.apply_bridge_error(err.to_string(), cx),
        }
    }

    pub(super) fn apply_pending_thread_started_result(
        &mut self,
        chat: &Entity<ChatState>,
        project: Option<&Entity<ProjectState>>,
        projectless: bool,
        result: Result<Thread, BridgeError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(thread) => self.apply_pending_thread_started(chat, project, projectless, thread, cx),
            Err(err) => {
                let message = err.to_string();
                chat.update(cx, |chat, cx| {
                    chat.is_creating = false;
                    if let Some((client_id, _)) = chat.pending_user_message_request() {
                        chat.fail_user_message(&client_id, message.clone());
                    }
                    chat.upsert_notice("thread-start-error".into(), message.clone());
                    cx.notify();
                });
                tracing::error!(error = %message, "failed to start thread");
            }
        }
    }

    pub(super) fn apply_forked_thread_result(
        &mut self,
        source_chat: &Entity<ChatState>,
        source_thread_id: &str,
        result: Result<Thread, BridgeError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(thread) => self.apply_forked_thread(source_chat, thread, cx),
            Err(error) => {
                self.apply_thread_error(source_thread_id, "fork", error.to_string(), false, cx)
            }
        }
    }

    pub(super) fn apply_user_submission_result(
        &mut self,
        thread_id: &str,
        client_user_message_id: &str,
        finish_turn_on_error: bool,
        result: Result<(), BridgeError>,
        cx: &mut Context<Self>,
    ) {
        let Err(err) = result else {
            return;
        };
        let message = err.to_string();
        tracing::error!(
            thread_id,
            client_user_message_id,
            error = %message,
            "user message submission failed"
        );
        if let Some(chat) = self.find_chat_entity(thread_id, cx) {
            chat.update(cx, |chat, cx| {
                if finish_turn_on_error {
                    chat.finish_turn(None);
                }
                chat.fail_user_message(client_user_message_id, message.clone());
                chat.upsert_notice(format!("send-error-{client_user_message_id}"), message);
                cx.notify();
            });
        }
    }

    pub(super) fn apply_thread_resumed_result(
        &mut self,
        chat: &Entity<ChatState>,
        requested_thread_id: &str,
        result: Result<Thread, BridgeError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(thread) => self.apply_thread_resumed(chat, thread, cx),
            Err(err) => {
                let message = err.to_string();
                chat.update(cx, |chat, cx| {
                    chat.upsert_notice(
                        format!("resume-error-{requested_thread_id}"),
                        message.clone(),
                    );
                    cx.notify();
                });
            }
        }
        chat.update(cx, |chat, cx| {
            chat.is_loading = false;
            cx.notify();
        });
    }

    pub(super) fn apply_unit_result(
        &mut self,
        result: Result<(), BridgeError>,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = result {
            self.apply_bridge_error(err.to_string(), cx);
        }
    }

    pub(super) fn apply_thread_unit_result(
        &mut self,
        thread_id: &str,
        operation: &str,
        result: Result<(), BridgeError>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = result {
            self.append_notice(
                thread_id,
                format!("{operation}-error-{thread_id}"),
                error.to_string(),
                cx,
            );
        }
    }
}

fn loaded_chats(
    threads: Vec<Thread>,
    settings: crate::gui::ChatSettings,
    cx: &mut Context<CodexGui>,
) -> (Vec<Entity<ChatState>>, Option<i64>) {
    let latest_thread_updated_at = threads.iter().map(|thread| thread.updated_at).max();
    let chats = if threads.is_empty() {
        vec![empty_chat_entity(cx)]
    } else {
        threads
            .into_iter()
            .map(|thread| chat_entity_from_thread(thread, settings.clone(), cx))
            .collect()
    };
    (chats, latest_thread_updated_at)
}

/// Thread IDs that the Codex desktop marks as project-less, so they are shown
/// outside of any project group in the sidebar.
fn projectless_thread_ids() -> HashSet<String> {
    CodexGlobalState::load()
        .map(|state| state.projectless_thread_ids())
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to load project-less thread ids");
            HashSet::new()
        })
}

fn resolve_reasoning_effort(
    effort: &str,
    model_id: &str,
    models: &[ModelOption],
) -> Option<String> {
    models
        .iter()
        .find(|model| model.id == model_id)?
        .supported_efforts
        .iter()
        .any(|supported| supported == effort)
        .then(|| effort.to_owned())
}

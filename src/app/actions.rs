//! Handle user intents

use super::{
    CodexGui,
    thread_mapping::{empty_chat_entity, project_name_from_path},
};
use crate::gui::{ApprovalsReviewer, ChatState, ProjectState, single_line_title};
use anyhow::{Context as _, Result, anyhow};
use chrono::Local;
use codex_app_server_protocol::RequestId;
use gpui::{AppContext, Context};
use std::{fs, io::ErrorKind, path::PathBuf};
use uuid::Uuid;

impl CodexGui {
    pub(crate) fn dismiss_notice(
        &mut self,
        chat_id: String,
        notice_id: String,
        cx: &mut Context<Self>,
    ) {
        self.update_chat(&chat_id, cx, |chat| chat.remove_notice(&notice_id));
    }

    pub(crate) fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.select_project(index);
            cx.notify();
        });
        self.window_state.update(cx, |state, cx| {
            state.select_new_chat_project();
            cx.notify();
        });
    }

    pub(crate) fn open_new_chat(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let projectless = state.active_projectless_chat.is_some() || state.projects.is_empty();
        self.window_state.update(cx, |state, cx| {
            state.open_new_chat(projectless);
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn select_new_chat_projectless(&mut self, cx: &mut Context<Self>) {
        self.window_state.update(cx, |state, cx| {
            state.select_new_chat_projectless();
            cx.notify();
        });
    }

    pub(crate) fn add_project(&mut self, path: String, cx: &mut Context<Self>) {
        let path = path.trim();
        if path.is_empty() {
            return;
        }

        if let Some(index) = self
            .state
            .update(cx, |state, cx| state.project_index_by_path(path, cx))
        {
            self.select_project(index, cx);
            self.open_new_chat(cx);
            return;
        }

        let name = project_name_from_path(path);
        let empty_chat = empty_chat_entity(cx);
        let project = cx.new(|_| ProjectState::new(name.into(), path.into(), vec![empty_chat]));
        let index = self.state.update(cx, |state, cx| {
            let index = state.add_project(project);
            cx.notify();
            index
        });
        self.open_new_chat(cx);
        self.select_project(index, cx);
    }

    pub(crate) fn select_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        let chat = self
            .state
            .read(cx)
            .active_project()
            .and_then(|project| project.read(cx).chats.get(index).cloned());

        self.state.update(cx, |state, cx| {
            state.select_chat(index);
            cx.notify();
        });
        self.window_state.update(cx, |state, cx| {
            state.close_new_chat();
            cx.notify();
        });

        if let Some(chat) = chat.filter(|chat| {
            let chat = chat.read(cx);
            chat.thread.is_some() && !chat.is_loading
        }) {
            let thread_id = chat.read(cx).id.clone();
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

    pub(crate) fn select_projectless_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        let chat = self.state.read(cx).projectless_chats.get(index).cloned();

        self.state.update(cx, |state, cx| {
            state.select_projectless_chat(index);
            cx.notify();
        });
        self.window_state.update(cx, |state, cx| {
            state.close_new_chat();
            cx.notify();
        });

        if let Some(chat) = chat.filter(|chat| {
            let chat = chat.read(cx);
            chat.thread.is_some() && !chat.is_loading
        }) {
            let thread_id = chat.read(cx).id.clone();
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

    pub(crate) fn fork_chat_through(
        &mut self,
        chat: gpui::Entity<ChatState>,
        turn_id: String,
        cx: &mut Context<Self>,
    ) {
        let thread_id = chat.read(cx).id.clone();
        if chat.read(cx).thread.is_none() {
            return;
        }
        tracing::info!(thread_id, turn_id, "forking thread through turn");
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge
                .fork_thread(thread_id.clone(), Some(turn_id), None)
                .await;
            let _ = this.update(cx, |view, cx| {
                view.apply_forked_thread_result(&chat, &thread_id, result, cx)
            });
        })
        .detach();
    }

    pub(crate) fn submit_edited_turn_text(
        &mut self,
        source_chat: gpui::Entity<ChatState>,
        turn_id: String,
        previous_turn_id: Option<String>,
        client_user_message_id: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if source_chat.read(cx).active_turn.is_some() {
            return;
        }
        let source_thread_id = source_chat.read(cx).id.clone();
        if source_chat.read(cx).thread.is_none() {
            return;
        }

        tracing::info!(
            thread_id = source_thread_id,
            last_turn_id = previous_turn_id,
            "replacing thread for edited message"
        );
        let settings = source_chat.read(cx).settings.clone();
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let before_turn_id = previous_turn_id.is_none().then_some(turn_id);
            let forked_thread = match bridge
                .fork_thread(source_thread_id.clone(), previous_turn_id, before_turn_id)
                .await
            {
                Ok(thread) => thread,
                Err(err) => {
                    let _ = this.update(cx, |view, cx| {
                        view.apply_thread_error(
                            &source_thread_id,
                            "edit-fork",
                            err.to_string(),
                            false,
                            cx,
                        )
                    });
                    return;
                }
            };
            let replacement_thread_id = forked_thread.id.clone();
            let replacement_chat = this
                .update(cx, |view, cx| {
                    view.replace_thread_in_ui(&source_chat, forked_thread, settings.clone(), cx)
                })
                .ok()
                .flatten();
            let Some(replacement_chat) = replacement_chat else {
                return;
            };

            let pending_started = this
                .update(cx, |_view, cx| {
                    replacement_chat.update(cx, |chat, cx| {
                        let started =
                            chat.begin_user_message(client_user_message_id.clone(), text.clone());
                        if started {
                            cx.notify();
                        }
                        started
                    })
                })
                .unwrap_or(false);
            if !pending_started {
                return;
            }

            match bridge.delete_thread(source_thread_id.clone()).await {
                Ok(()) => {
                    let _ = this.update(cx, |view, cx| {
                        view.remove_thread_from_ui(&source_thread_id, cx)
                    });
                }
                Err(err) => {
                    let _ = this.update(cx, |view, cx| {
                        view.append_notice(
                            &replacement_thread_id,
                            format!("delete-error-{source_thread_id}"),
                            err.to_string(),
                            cx,
                        )
                    });
                }
            }

            let result = bridge
                .send_turn(
                    replacement_thread_id.clone(),
                    client_user_message_id.clone(),
                    text,
                    settings,
                )
                .await
                .map(|_| ());
            let _ = this.update(cx, |view, cx| {
                view.apply_user_submission_result(
                    &replacement_thread_id,
                    &client_user_message_id,
                    true,
                    result,
                    cx,
                )
            });
        })
        .detach();
    }

    /// Starts an empty thread for the active project.
    ///
    /// Composer submission normally goes through `submit_turn_text` so the first
    /// prompt can be sent after the asynchronous thread creation completes.
    fn start_new_thread(
        &mut self,
        chat: gpui::Entity<ChatState>,
        project: Option<gpui::Entity<ProjectState>>,
        projectless: bool,
        cwd: String,
        cx: &mut Context<Self>,
    ) {
        let settings = chat.read(cx).settings.clone();
        tracing::info!(cwd, "starting thread");
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge.start_thread(cwd, settings).await;
            let _ = this.update(cx, |view, cx| {
                view.apply_pending_thread_started_result(
                    &chat,
                    project.as_ref(),
                    projectless,
                    result,
                    cx,
                )
            });
        })
        .detach();
    }

    pub(crate) fn submit_new_turn_text(
        &mut self,
        client_user_message_id: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let project = self.active_project_entity(cx);
        let projectless = self.window_state.read(cx).new_chat_projectless || project.is_none();
        let cwd = if projectless {
            match new_projectless_chat_directory() {
                Ok(cwd) => cwd,
                Err(error) => {
                    self.apply_bridge_error(
                        format!("failed to create project-less chat directory: {error}"),
                        cx,
                    );
                    return;
                }
            }
        } else {
            project
                .as_ref()
                .expect("project checked above")
                .read(cx)
                .path
                .to_string()
        };
        let title = single_line_title(&text);
        let pending_chat_id = format!("pending-{client_user_message_id}");
        let settings = self.state.read(cx).new_chat_settings.clone();
        let pending_chat = cx.new(|_| {
            let mut chat = ChatState::new(
                pending_chat_id,
                title.into(),
                format!("starting - {cwd}").into(),
                Vec::new(),
            );
            chat.settings = settings;
            chat.is_creating = true;
            chat.begin_user_message(client_user_message_id, text);
            chat
        });
        if projectless {
            self.state.update(cx, |state, cx| {
                state
                    .projectless_chats
                    .retain(|chat| chat.read(cx).id.as_str() != "empty");
                state.projectless_chats.insert(0, pending_chat.clone());
                state.select_projectless_chat(0);
                cx.notify();
            });
        } else if let Some(project) = project.as_ref() {
            project.update(cx, |project, cx| {
                project
                    .chats
                    .retain(|chat| chat.read(cx).id.as_str() != "empty");
                project.chats.insert(0, pending_chat.clone());
                cx.notify();
            });
            self.state.update(cx, |state, cx| {
                state.select_chat(0);
                cx.notify();
            });
        }
        self.window_state.update(cx, |state, cx| {
            state.new_chat_draft.clear();
            state.close_new_chat();
            cx.notify();
        });
        self.start_new_thread(pending_chat, project, projectless, cwd, cx);
    }

    pub(crate) fn submit_turn_text(
        &mut self,
        chat: gpui::Entity<ChatState>,
        client_user_message_id: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if chat.read(cx).thread.is_none() {
            return;
        }
        if chat.read(cx).user_message_is_sending() {
            return;
        }
        let thread_id = chat.read(cx).id.clone();
        let pending_started = chat.update(cx, |chat, cx| {
            let started = chat.begin_user_message(client_user_message_id.clone(), text.clone());
            if started {
                cx.notify();
            }
            started
        });
        if !pending_started {
            return;
        }
        let settings = chat.read(cx).settings.clone();
        tracing::info!(thread_id, "starting turn");
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

    pub(crate) fn steer_turn_text(
        &mut self,
        chat: gpui::Entity<ChatState>,
        turn_id: String,
        client_user_message_id: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        if chat.read(cx).active_turn_id() != Some(turn_id.as_str()) {
            return;
        }
        if chat.read(cx).user_message_is_sending() {
            return;
        }
        let active_thread_id = chat.read(cx).id.clone();
        let pending_started = chat.update(cx, |chat, cx| {
            let started = chat.begin_user_message(client_user_message_id.clone(), text.clone());
            if started {
                cx.notify();
            }
            started
        });
        if !pending_started {
            return;
        }
        tracing::info!(thread_id = active_thread_id, turn_id, "steering turn");
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge
                .steer_turn(
                    active_thread_id.clone(),
                    turn_id,
                    client_user_message_id.clone(),
                    text,
                )
                .await
                .map(|_| ());
            let _ = this.update(cx, |view, cx| {
                view.apply_user_submission_result(
                    &active_thread_id,
                    &client_user_message_id,
                    false,
                    result,
                    cx,
                )
            });
        })
        .detach();
    }

    pub(crate) fn stop_turn(
        &mut self,
        chat: gpui::Entity<ChatState>,
        turn_id: String,
        cx: &mut Context<Self>,
    ) {
        if chat.read(cx).active_turn_id() != Some(turn_id.as_str()) {
            return;
        }
        let thread_id = chat.read(cx).id.clone();
        tracing::info!(thread_id, turn_id, "stopping turn");
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge.interrupt_turn(thread_id.clone(), turn_id).await;
            let _ = this.update(cx, |view, cx| {
                view.apply_thread_unit_result(&thread_id, "interrupt", result, cx)
            });
        })
        .detach();
    }

    pub(crate) fn set_model(
        &mut self,
        chat: Option<gpui::Entity<ChatState>>,
        model: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(chat) = chat {
            let default_effort = self
                .state
                .read(cx)
                .available_models
                .iter()
                .find(|option| option.id == model)
                .map(|option| option.default_effort.clone());
            chat.update(cx, |chat, cx| {
                chat.settings.model = model;
                if let Some(effort) = default_effort {
                    chat.settings.effort = effort;
                }
                cx.notify();
            });
            self.sync_thread_settings(chat, cx);
        } else {
            self.state.update(cx, |state, cx| {
                state.set_model(model);
                cx.notify();
            });
            self.persist_new_chat_settings(cx);
        }
    }

    pub(crate) fn set_effort(
        &mut self,
        chat: Option<gpui::Entity<ChatState>>,
        effort: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(chat) = chat {
            chat.update(cx, |chat, cx| {
                chat.settings.effort = effort;
                cx.notify();
            });
            self.sync_thread_settings(chat, cx);
        } else {
            self.state.update(cx, |state, cx| {
                state.set_effort(effort);
                cx.notify();
            });
            self.persist_new_chat_settings(cx);
        }
    }

    pub(crate) fn set_permission_profile(
        &mut self,
        chat: Option<gpui::Entity<ChatState>>,
        permission_profile: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(chat) = chat {
            chat.update(cx, |chat, cx| {
                chat.settings.permission_profile = permission_profile;
                cx.notify();
            });
            self.sync_thread_settings(chat, cx);
        } else {
            self.state.update(cx, |state, cx| {
                state.set_permission_profile(permission_profile);
                cx.notify();
            });
            self.persist_new_chat_settings(cx);
        }
    }

    pub(crate) fn set_approvals_reviewer(
        &mut self,
        chat: Option<gpui::Entity<ChatState>>,
        approvals_reviewer: ApprovalsReviewer,
        cx: &mut Context<Self>,
    ) {
        if let Some(chat) = chat {
            chat.update(cx, |chat, cx| {
                chat.settings.approvals_reviewer = approvals_reviewer;
                cx.notify();
            });
            self.sync_thread_settings(chat, cx);
        } else {
            self.state.update(cx, |state, cx| {
                state.set_approvals_reviewer(approvals_reviewer);
                cx.notify();
            });
            self.persist_new_chat_settings(cx);
        }
    }

    pub(crate) fn toggle_side_chat(&mut self, cx: &mut Context<Self>) {
        self.window_state.update(cx, |state, cx| {
            state.toggle_side_chat();
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn resolve_approval(
        &mut self,
        chat: gpui::Entity<ChatState>,
        request_id: RequestId,
        approved: bool,
        cx: &mut Context<Self>,
    ) {
        let thread_id = chat.read(cx).id.clone();
        let response = chat
            .read(cx)
            .pending_approvals
            .iter()
            .find(|approval| approval.request_id == request_id)
            .map(|approval| approval.response(approved));
        let Some(response) = response else {
            return;
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                self.apply_bridge_error(format!("failed to encode approval response: {error}"), cx);
                return;
            }
        };

        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge
                .resolve_server_request(request_id.clone(), response)
                .await;
            let _ = this.update(cx, |view, cx| match result {
                Ok(()) => view.remove_pending_approval(&thread_id, &request_id, cx),
                Err(error) => view.apply_thread_error(
                    &thread_id,
                    "approval",
                    format!("failed to resolve approval: {error}"),
                    false,
                    cx,
                ),
            });
        })
        .detach();
    }

    pub(crate) fn answer_server_input(
        &mut self,
        chat: gpui::Entity<ChatState>,
        request_id: RequestId,
        question_id: String,
        answer: String,
        cx: &mut Context<Self>,
    ) {
        let thread_id = chat.read(cx).id.clone();
        let response = chat.update(cx, |chat, cx| {
            let response = chat.answer_input_request(&request_id, question_id, answer);
            cx.notify();
            response
        });
        let Some(response) = response else {
            return;
        };
        let response = match serde_json::to_value(response) {
            Ok(response) => response,
            Err(error) => {
                self.apply_thread_error(
                    &thread_id,
                    "user-input",
                    format!("failed to encode input response: {error}"),
                    false,
                    cx,
                );
                return;
            }
        };
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge
                .resolve_server_request(request_id.clone(), response)
                .await;
            let _ = this.update(cx, |view, cx| match result {
                Ok(()) => view.remove_pending_input(&thread_id, &request_id, cx),
                Err(error) => view.apply_thread_error(
                    &thread_id,
                    "user-input",
                    format!("failed to answer input request: {error}"),
                    false,
                    cx,
                ),
            });
        })
        .detach();
    }

    pub(crate) fn reject_server_input(
        &mut self,
        chat: gpui::Entity<ChatState>,
        request_id: RequestId,
        cx: &mut Context<Self>,
    ) {
        let thread_id = chat.read(cx).id.clone();
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge
                .reject_server_request(request_id.clone(), "user declined input request".into())
                .await;
            let _ = this.update(cx, |view, cx| match result {
                Ok(()) => view.remove_pending_input(&thread_id, &request_id, cx),
                Err(error) => view.apply_thread_error(
                    &thread_id,
                    "user-input",
                    format!("failed to reject input request: {error}"),
                    false,
                    cx,
                ),
            });
        })
        .detach();
    }

    fn sync_thread_settings(&mut self, chat: gpui::Entity<ChatState>, cx: &mut Context<Self>) {
        let Some(thread_id) = chat
            .read(cx)
            .thread
            .as_ref()
            .map(|thread| thread.id.clone())
        else {
            return;
        };
        let settings = chat.read(cx).settings.clone();
        tracing::info!(thread_id, "updating thread settings");
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge
                .update_thread_settings(thread_id.clone(), settings)
                .await;
            let _ = this.update(cx, |view, cx| {
                view.apply_thread_unit_result(&thread_id, "settings", result, cx)
            });
        })
        .detach();
    }

    fn persist_new_chat_settings(&mut self, cx: &mut Context<Self>) {
        let settings = self.state.read(cx).new_chat_settings.clone();
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge.write_chat_settings(settings).await;
            let _ = this.update(cx, |view, cx| view.apply_unit_result(result, cx));
        })
        .detach();
    }
}

fn new_projectless_chat_directory() -> Result<String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("failed to locate the home directory"))?;
    let documents = home.join("Documents");
    let date = Local::now().format("%Y-%m-%d").to_string();
    let parent = documents.join("Codex").join(date);
    fs::create_dir_all(&parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    for _ in 0..10 {
        let random = Uuid::new_v4().simple().to_string();
        let path: PathBuf = parent.join(format!("chat-{}", &random[..8]));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path.to_string_lossy().into_owned()),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to create {}", path.display()));
            }
        }
    }

    Err(anyhow!(
        "failed to allocate a unique project-less chat directory under {}",
        parent.display()
    ))
}

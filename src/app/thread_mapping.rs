use super::CodexGui;
use crate::gui::{ChatSettings, ChatState, HistoryNotice, single_line_title};
use codex_app_server_protocol::{Thread, ThreadStatus};
use gpui::{AppContext, Context, Entity};
use std::path::Path;

pub(super) fn chat_entity_from_thread(
    thread: Thread,
    settings: ChatSettings,
    cx: &mut Context<CodexGui>,
) -> Entity<ChatState> {
    let title = thread_title(thread.name.as_deref(), &thread.preview);
    let subtitle = format!(
        "{} - {}",
        thread_status_label(&thread.status),
        thread.cwd.display()
    );
    cx.new(|_| ChatState::from_thread(thread, title.into(), subtitle.into(), settings))
}

pub(super) fn thread_title(name: Option<&str>, preview: &str) -> String {
    let title = name
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            let preview = preview.trim();
            (!preview.is_empty()).then_some(preview)
        })
        .unwrap_or("Untitled Codex thread");
    single_line_title(title)
}

pub(super) fn empty_chat_entity(cx: &mut Context<CodexGui>) -> Entity<ChatState> {
    cx.new(|_| {
        ChatState::new(
            "empty".into(),
            "No Codex threads".into(),
            "Click New to start one in this workspace".into(),
            vec![HistoryNotice {
                id: "empty-thread-list".into(),
                body: "No persisted Codex threads were returned for this workspace.".into(),
            }],
        )
    })
}

pub(super) fn project_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

pub(super) fn thread_status_label(status: &ThreadStatus) -> &'static str {
    match status {
        ThreadStatus::NotLoaded => "not loaded",
        ThreadStatus::Idle => "idle",
        ThreadStatus::SystemError => "system error",
        ThreadStatus::Active { .. } => "active",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_title_prefers_name() {
        assert_eq!(
            thread_title(Some("Saved title"), "First prompt"),
            "Saved title"
        );
    }

    #[test]
    fn thread_title_falls_back_to_preview() {
        assert_eq!(thread_title(None, "  First prompt  "), "First prompt");
    }

    #[test]
    fn thread_title_removes_all_line_breaks() {
        assert_eq!(
            thread_title(None, " First line\nSecond line\r\nThird line\rFourth line "),
            "First line Second line Third line Fourth line"
        );
    }

    #[test]
    fn thread_title_uses_default_when_name_and_preview_are_empty() {
        assert_eq!(thread_title(Some("   "), " "), "Untitled Codex thread");
    }
}

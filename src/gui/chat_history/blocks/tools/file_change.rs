use std::ops::Range;

use codex_app_server_protocol::{FileUpdateChange, PatchChangeKind, ThreadItem};
use gpui::{
    AnyElement, App, Context, Entity, FollowMode, IntoElement, ListAlignment, ListState,
    ParentElement, Render, SharedString, Styled, WeakEntity, Window, div, list, px,
};
use gpui_component::{
    IconName,
    scroll::{ScrollableElement as _, ScrollableMask},
};

use crate::gui::ChatState;

use super::simple::{ToolFrame, ToolRow, ToolStatus};

const DIFF_MAX_HEIGHT: f32 = 144.;
const DIFF_LINE_HEIGHT: f32 = 18.;

#[derive(Clone)]
pub(in crate::gui::chat_history) struct FileChangeTool {
    item_id: String,
    chat: WeakEntity<ChatState>,
    title: SharedString,
    change_labels: Vec<SharedString>,
    progress: Vec<SharedString>,
    show_detail: bool,
    status: ToolStatus,
    additions: usize,
    deletions: usize,
}

impl FileChangeTool {
    pub(super) fn new(
        item: &ThreadItem,
        status: ToolStatus,
        progress: Option<&[SharedString]>,
        chat: WeakEntity<ChatState>,
    ) -> Option<Self> {
        let ThreadItem::FileChange { id, changes, .. } = item else {
            return None;
        };
        let progress = progress.unwrap_or_default().to_vec();
        if changes.is_empty() {
            return Some(Self {
                item_id: id.clone(),
                chat,
                title: "Preparing file edits".into(),
                change_labels: Vec::new(),
                show_detail: !progress.is_empty(),
                progress,
                status,
                additions: 0,
                deletions: 0,
            });
        }

        let (additions, deletions) = changes.iter().fold((0, 0), |mut total, change| {
            let stats = diff_stats(&change.diff);
            total.0 += stats.0;
            total.1 += stats.1;
            total
        });
        let action = change_action(&changes[0].kind, status);
        let all_same_action = changes
            .iter()
            .all(|change| change_action(&change.kind, status) == action);
        let title = if let [change] = changes.as_slice() {
            let path = match &change.kind {
                PatchChangeKind::Update {
                    move_path: Some(move_path),
                } => format!("{} → {}", change.path, move_path.display()),
                _ => change.path.clone(),
            };
            format!("{action} {path}")
        } else if all_same_action {
            format!("{action} {} files", changes.len())
        } else if matches!(status, ToolStatus::Running) {
            format!("Changing {} files", changes.len())
        } else {
            format!("Changed {} files", changes.len())
        };
        Some(Self {
            item_id: id.clone(),
            chat,
            title: title.into(),
            change_labels: changes
                .iter()
                .map(|change| change.path.clone().into())
                .collect(),
            progress,
            show_detail: true,
            status,
            additions,
            deletions,
        })
    }

    pub(super) fn status(&self) -> ToolStatus {
        self.status
    }
}

impl FileChangeTool {
    pub(super) fn render_row(
        self,
        row: ToolRow,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let frame = ToolFrame::new(IconName::File, self.title, None, self.status)
            .diff(self.additions, self.deletions)
            .row(row);
        if !self.show_detail {
            return frame.into_any_element();
        }

        let state_key = format!("file-change-output-{}", self.item_id);
        let chat = self.chat.clone();
        let item_id = self.item_id.clone();
        let state: Entity<FileChangeOutputState> =
            window.use_keyed_state(state_key, cx, move |_, _| {
                FileChangeOutputState::new(chat, item_id)
            });
        state.update(cx, |state, _| {
            state.configure(self.change_labels, self.progress, self.status)
        });

        frame.custom_detail(state).into_any_element()
    }
}

struct FileChangeOutputState {
    chat: WeakEntity<ChatState>,
    item_id: String,
    change_labels: Vec<SharedString>,
    progress: Vec<SharedString>,
    diff_line_starts: Vec<Vec<usize>>,
    indexed_diff_lens: Vec<usize>,
    reset_diff_indexes: bool,
    status: Option<ToolStatus>,
    list_state: ListState,
}

impl FileChangeOutputState {
    fn new(chat: WeakEntity<ChatState>, item_id: String) -> Self {
        let list_state = ListState::new(0, ListAlignment::Top, px(36.));
        list_state.set_follow_mode(FollowMode::Tail);
        Self {
            chat,
            item_id,
            change_labels: Vec::new(),
            progress: Vec::new(),
            diff_line_starts: Vec::new(),
            indexed_diff_lens: Vec::new(),
            reset_diff_indexes: false,
            status: None,
            list_state,
        }
    }

    fn configure(
        &mut self,
        change_labels: Vec<SharedString>,
        progress: Vec<SharedString>,
        status: ToolStatus,
    ) {
        if self.status.is_some_and(|old| old != status) {
            // ItemCompleted may replace the streamed item with a retained diff.
            self.reset_diff_indexes = true;
        }

        if self.change_labels.len() != change_labels.len() {
            self.change_labels = change_labels;
            self.progress = progress;
            self.reset_diff_indexes = true;
            self.status = Some(status);
            return;
        }

        for (index, (old, new)) in self.change_labels.iter().zip(&change_labels).enumerate() {
            if old != new {
                let row = self.change_row_start(index);
                self.list_state.remeasure_items(row..row + 1);
            }
        }
        self.change_labels = change_labels;

        if self.progress != progress {
            let start = self.progress_row_start();
            let old_count = self.progress_row_count();
            self.progress = progress;
            self.list_state
                .splice(start..start + old_count, self.progress_row_count());
        }
        self.status = Some(status);
    }

    fn sync_diff_indexes(&mut self, changes: &[FileUpdateChange]) {
        let must_reset = self.reset_diff_indexes
            || changes.len() != self.diff_line_starts.len()
            || changes.iter().enumerate().any(|(index, change)| {
                self.indexed_diff_lens.get(index).is_none_or(|indexed_len| {
                    change.diff.len() < *indexed_len || !change.diff.is_char_boundary(*indexed_len)
                })
            });
        if must_reset {
            self.replace_diff_indexes(changes);
            self.reset_diff_indexes = false;
            return;
        }

        for (change_index, change) in changes.iter().enumerate() {
            let indexed_len = self.indexed_diff_lens[change_index];
            if change.diff.len() == indexed_len {
                continue;
            }

            let row_start = self.change_row_start(change_index) + 1;
            let old_line_count = self.diff_line_starts[change_index].len();
            for (offset, byte) in change.diff.as_bytes()[indexed_len..].iter().enumerate() {
                if *byte == b'\n' {
                    self.diff_line_starts[change_index].push(indexed_len + offset + 1);
                }
            }
            self.indexed_diff_lens[change_index] = change.diff.len();

            let added = self.diff_line_starts[change_index].len() - old_line_count;
            if added > 0 {
                let insertion = row_start + old_line_count;
                self.list_state.splice(insertion..insertion, added);
            }
            if old_line_count > 0 {
                let last_old_line = row_start + old_line_count - 1;
                self.list_state
                    .remeasure_items(last_old_line..last_old_line + 1);
            }
        }
    }

    fn replace_diff_indexes(&mut self, changes: &[FileUpdateChange]) {
        let old_count = self.list_state.item_count();
        self.diff_line_starts = changes
            .iter()
            .map(|change| line_starts(&change.diff))
            .collect();
        self.indexed_diff_lens = changes.iter().map(|change| change.diff.len()).collect();
        self.list_state.splice(0..old_count, self.row_count());
    }

    fn change_row_start(&self, change_index: usize) -> usize {
        self.diff_line_starts
            .iter()
            .take(change_index)
            .map(|lines| 1 + lines.len() + 1)
            .sum()
    }

    fn progress_row_start(&self) -> usize {
        self.change_labels.len()
            + self.diff_line_starts.iter().map(Vec::len).sum::<usize>()
            + self.change_labels.len().saturating_sub(1)
    }

    fn progress_row_count(&self) -> usize {
        if self.progress.is_empty() {
            0
        } else {
            self.progress.len() + usize::from(!self.change_labels.is_empty())
        }
    }

    fn row_count(&self) -> usize {
        self.progress_row_start() + self.progress_row_count()
    }

    fn row(&self, index: usize) -> Option<FileChangeOutputRow> {
        let mut index = index;
        for (change_index, label) in self.change_labels.iter().enumerate() {
            if index == 0 {
                return Some(FileChangeOutputRow::Owned(label.clone()));
            }
            index -= 1;

            let lines = self.diff_line_starts.get(change_index)?;
            if let Some(start) = lines.get(index).copied() {
                let end = lines
                    .get(index + 1)
                    .copied()
                    .map(|next| next.saturating_sub(1))
                    .unwrap_or(self.indexed_diff_lens[change_index]);
                return Some(FileChangeOutputRow::Diff {
                    change_index,
                    range: start..end,
                });
            }
            index = index.checked_sub(lines.len())?;

            if change_index + 1 < self.change_labels.len() {
                if index == 0 {
                    return Some(FileChangeOutputRow::Owned(SharedString::default()));
                }
                index -= 1;
            }
        }

        if !self.progress.is_empty() && !self.change_labels.is_empty() {
            if index == 0 {
                return Some(FileChangeOutputRow::Owned(SharedString::default()));
            }
            index -= 1;
        }
        self.progress
            .get(index)
            .cloned()
            .map(FileChangeOutputRow::Owned)
    }
}

enum FileChangeOutputRow {
    Owned(SharedString),
    Diff {
        change_index: usize,
        range: Range<usize>,
    },
}

impl Render for FileChangeOutputState {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = self.chat.clone();
        let item_id = self.item_id.clone();
        let _ = chat.read_with(cx, |chat, _| {
            self.sync_diff_indexes(chat.file_changes(&item_id).unwrap_or_default());
        });

        let row_count = self.row_count();
        let height = px((row_count.max(1) as f32 * DIFF_LINE_HEIGHT).min(DIFF_MAX_HEIGHT));
        let state = cx.entity().downgrade();
        let list_state = self.list_state.clone();

        div()
            .relative()
            .w_full()
            .h(height)
            .min_w_0()
            .child(
                list(list_state.clone(), move |index, _, cx| {
                    let row = state
                        .read_with(cx, |state, _| state.row(index))
                        .ok()
                        .flatten();
                    let text = match row {
                        Some(FileChangeOutputRow::Owned(text)) => text,
                        Some(FileChangeOutputRow::Diff {
                            change_index,
                            range,
                        }) => state
                            .read_with(cx, |state, _| {
                                let chat = state.chat.clone();
                                let item_id = state.item_id.clone();
                                chat.read_with(cx, |chat, _| {
                                    chat.file_changes(&item_id)
                                        .and_then(|changes| changes.get(change_index))
                                        .and_then(|change| change.diff.get(range))
                                        .unwrap_or_default()
                                        .trim_end_matches('\r')
                                        .to_string()
                                        .into()
                                })
                                .unwrap_or_else(|_| SharedString::default())
                            })
                            .unwrap_or_else(|_| SharedString::default()),
                        None => SharedString::default(),
                    };
                    div()
                        .w_full()
                        .min_w_0()
                        .min_h(px(DIFF_LINE_HEIGHT))
                        .whitespace_normal()
                        .child(text)
                        .into_any_element()
                })
                .size_full(),
            )
            .vertical_scrollbar(&list_state)
            // A distinct id per row keeps rows from sharing a gesture axis lock.
            .child(
                ScrollableMask::new(gpui::Axis::Vertical, &list_state)
                    .id(format!("file-change-mask-{}", self.item_id)),
            )
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        text.as_bytes()
            .iter()
            .enumerate()
            .filter_map(|(index, byte)| (*byte == b'\n').then_some(index + 1)),
    );
    starts
}

fn change_action(kind: &PatchChangeKind, status: ToolStatus) -> &'static str {
    let running = matches!(status, ToolStatus::Running);
    match kind {
        PatchChangeKind::Add if running => "Adding",
        PatchChangeKind::Add => "Added",
        PatchChangeKind::Delete if running => "Deleting",
        PatchChangeKind::Delete => "Deleted",
        PatchChangeKind::Update {
            move_path: None, ..
        } if running => "Editing",
        PatchChangeKind::Update {
            move_path: None, ..
        } => "Edited",
        PatchChangeKind::Update {
            move_path: Some(_), ..
        } if running => "Moving",
        PatchChangeKind::Update {
            move_path: Some(_), ..
        } => "Moved",
    }
}

fn diff_stats(diff: &str) -> (usize, usize) {
    let mut additions = 0;
    let mut deletions = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    (additions, deletions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_diff_lines_by_byte_offset() {
        assert_eq!(line_starts("one\ntwo\n"), vec![0, 4, 8]);
    }
}

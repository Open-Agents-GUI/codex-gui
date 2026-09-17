use std::ops::Range;

use codex_app_server_protocol::ThreadItem;
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

const OUTPUT_MAX_HEIGHT: f32 = 144.;
const OUTPUT_LINE_HEIGHT: f32 = 18.;

#[derive(Clone)]
pub(in crate::gui::chat_history) struct CommandTool {
    item_id: String,
    chat: WeakEntity<ChatState>,
    title: SharedString,
    before_output: Vec<SharedString>,
    after_output: Vec<SharedString>,
    status: ToolStatus,
}

impl CommandTool {
    pub(super) fn new(
        item: &ThreadItem,
        status: ToolStatus,
        progress: Option<&[SharedString]>,
        chat: WeakEntity<ChatState>,
    ) -> Option<Self> {
        let ThreadItem::CommandExecution {
            id,
            command,
            cwd,
            exit_code,
            duration_ms,
            ..
        } = item
        else {
            return None;
        };
        let action = if matches!(status, ToolStatus::Running) {
            "Running"
        } else {
            "Ran"
        };
        let mut after_output = Vec::new();
        if let Some(exit_code) = exit_code {
            after_output.push(format!("exit code: {exit_code}").into());
        }
        if let Some(duration_ms) = duration_ms {
            after_output.push(format!("duration: {duration_ms} ms").into());
        }
        if let Some(progress) = progress.filter(|progress| !progress.is_empty()) {
            if !after_output.is_empty() {
                after_output.push("".into());
            }
            after_output.extend(progress.iter().cloned());
        }
        Some(Self {
            item_id: id.clone(),
            chat,
            title: format!("{action} {}", single_line(command)).into(),
            before_output: vec![format!("cwd: {cwd}").into()],
            after_output,
            status,
        })
    }

    pub(super) fn status(&self) -> ToolStatus {
        self.status
    }
}

impl CommandTool {
    pub(super) fn render_row(
        self,
        row: ToolRow,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let state_key = format!("command-output-{}", self.item_id);
        let chat = self.chat.clone();
        let item_id = self.item_id.clone();
        let state: Entity<CommandOutputState> =
            window.use_keyed_state(state_key, cx, move |_, _| {
                CommandOutputState::new(chat, item_id)
            });
        state.update(cx, |state, _| {
            state.configure(self.before_output, self.after_output, self.status)
        });

        ToolFrame::new(IconName::SquareTerminal, self.title, None, self.status)
            .custom_detail(state)
            .row(row)
            .into_any_element()
    }
}

struct CommandOutputState {
    chat: WeakEntity<ChatState>,
    item_id: String,
    before_output: Vec<SharedString>,
    after_output: Vec<SharedString>,
    output_line_starts: Vec<usize>,
    indexed_output_len: usize,
    output_present: bool,
    reset_output_index: bool,
    status: Option<ToolStatus>,
    list_state: ListState,
}

impl CommandOutputState {
    fn new(chat: WeakEntity<ChatState>, item_id: String) -> Self {
        let list_state = ListState::new(0, ListAlignment::Top, px(36.));
        list_state.set_follow_mode(FollowMode::Tail);
        Self {
            chat,
            item_id,
            before_output: Vec::new(),
            after_output: Vec::new(),
            output_line_starts: Vec::new(),
            indexed_output_len: 0,
            output_present: false,
            reset_output_index: false,
            status: None,
            list_state,
        }
    }

    fn configure(
        &mut self,
        before_output: Vec<SharedString>,
        after_output: Vec<SharedString>,
        status: ToolStatus,
    ) {
        if self.status.is_some_and(|old| old != status) {
            // ItemCompleted replaces the streaming protocol item. Re-index once
            // rather than assuming the final retained output is a strict append.
            self.reset_output_index = true;
        }

        if self.before_output != before_output {
            let old_count = self.before_output.len();
            let new_count = before_output.len();
            self.before_output = before_output;
            self.list_state.splice(0..old_count, new_count);
        }
        if self.after_output != after_output {
            let start = self.before_output.len() + self.output_line_starts.len();
            let old_count = self.after_output.len();
            let new_count = after_output.len();
            self.after_output = after_output;
            self.list_state.splice(start..start + old_count, new_count);
        }
        self.status = Some(status);
    }

    fn sync_output_index(&mut self, output: Option<&str>) {
        let must_reset = self.reset_output_index
            || output.is_some() != self.output_present
            || output.is_some_and(|output| {
                output.len() < self.indexed_output_len
                    || !output.is_char_boundary(self.indexed_output_len)
            });
        if must_reset {
            self.replace_output_index(output);
            self.reset_output_index = false;
            return;
        }

        let Some(output) = output else {
            return;
        };
        let output_grew = output.len() > self.indexed_output_len;
        let old_line_count = self.output_line_starts.len();
        for (offset, byte) in output.as_bytes()[self.indexed_output_len..]
            .iter()
            .enumerate()
        {
            if *byte == b'\n' {
                self.output_line_starts
                    .push(self.indexed_output_len + offset + 1);
            }
        }
        self.indexed_output_len = output.len();
        let added = self.output_line_starts.len() - old_line_count;
        if added > 0 {
            let insertion = self.before_output.len() + old_line_count;
            self.list_state.splice(insertion..insertion, added);
        }
        if output_grew && old_line_count > 0 {
            let last_old_line = self.before_output.len() + old_line_count - 1;
            self.list_state
                .remeasure_items(last_old_line..last_old_line + 1);
        }
    }

    fn replace_output_index(&mut self, output: Option<&str>) {
        let old_line_count = self.output_line_starts.len();
        self.output_line_starts.clear();
        self.output_present = output.is_some();
        self.indexed_output_len = output.map_or(0, str::len);
        if let Some(output) = output {
            self.output_line_starts.push(0);
            self.output_line_starts.extend(
                output
                    .as_bytes()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, byte)| (*byte == b'\n').then_some(index + 1)),
            );
        }
        let start = self.before_output.len();
        self.list_state
            .splice(start..start + old_line_count, self.output_line_starts.len());
    }

    fn row_count(&self) -> usize {
        self.before_output.len() + self.output_line_starts.len() + self.after_output.len()
    }

    fn row(&self, index: usize) -> Option<CommandOutputRow> {
        if let Some(line) = self.before_output.get(index) {
            return Some(CommandOutputRow::Owned(line.clone()));
        }
        let output_index = index.checked_sub(self.before_output.len())?;
        if let Some(start) = self.output_line_starts.get(output_index).copied() {
            let end = self
                .output_line_starts
                .get(output_index + 1)
                .copied()
                .map(|next| next.saturating_sub(1))
                .unwrap_or(self.indexed_output_len);
            return Some(CommandOutputRow::Output(start..end));
        }
        let after_index = output_index.checked_sub(self.output_line_starts.len())?;
        self.after_output
            .get(after_index)
            .cloned()
            .map(CommandOutputRow::Owned)
    }
}

enum CommandOutputRow {
    Owned(SharedString),
    Output(Range<usize>),
}

impl Render for CommandOutputState {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = self.chat.clone();
        let item_id = self.item_id.clone();
        let _ = chat.read_with(cx, |chat, _| {
            self.sync_output_index(chat.command_output(&item_id));
        });

        let row_count = self.row_count();
        let height = px((row_count.max(1) as f32 * OUTPUT_LINE_HEIGHT).min(OUTPUT_MAX_HEIGHT));
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
                        Some(CommandOutputRow::Owned(text)) => text,
                        Some(CommandOutputRow::Output(range)) => state
                            .read_with(cx, |state, _| {
                                let chat = state.chat.clone();
                                let item_id = state.item_id.clone();
                                chat.read_with(cx, |chat, _| {
                                    chat.command_output(&item_id)
                                        .and_then(|output| output.get(range))
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
                        .min_h(px(OUTPUT_LINE_HEIGHT))
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
                    .id(format!("command-output-mask-{}", self.item_id)),
            )
    }
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_index_extends_incrementally() {
        let mut state = CommandOutputState::new(WeakEntity::new_invalid(), "command".into());
        state.sync_output_index(Some("one\ntwo"));
        assert_eq!(state.output_line_starts, vec![0, 4]);

        state.sync_output_index(Some("one\ntwo\nthree\n"));
        assert_eq!(state.output_line_starts, vec![0, 4, 8, 14]);
    }
}

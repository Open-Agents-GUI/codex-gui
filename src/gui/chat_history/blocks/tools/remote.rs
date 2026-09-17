use codex_app_server_protocol::ThreadItem;
use gpui::SharedString;
use gpui_component::IconName;
use serde_json::Value;

use super::simple::{SimpleTool, ToolStatus, append_progress, format_json};

#[derive(Clone)]
pub(in crate::gui::chat_history) struct McpTool {
    title: SharedString,
    detail: Option<SharedString>,
    status: ToolStatus,
}

impl McpTool {
    pub(super) fn new(
        item: &ThreadItem,
        status: ToolStatus,
        progress: Option<&[SharedString]>,
    ) -> Option<Self> {
        let ThreadItem::McpToolCall {
            server,
            tool,
            arguments,
            result,
            error,
            duration_ms,
            ..
        } = item
        else {
            return None;
        };
        let action = if matches!(status, ToolStatus::Running) {
            "Calling"
        } else {
            "Called"
        };
        let mut details = vec![format_json(arguments)];
        if let Some(result) = result {
            details.push(format!("result:\n{}", format_json(result.as_ref())));
        }
        if let Some(error) = error {
            details.push(format!("error:\n{}", format_json(error)));
        }
        if let Some(duration_ms) = duration_ms {
            details.push(format!("duration: {duration_ms} ms"));
        }
        Some(Self {
            title: format!("{action} {server}.{tool}").into(),
            detail: append_progress(Some(details.join("\n\n")), progress),
            status,
        })
    }
}

impl SimpleTool for McpTool {
    fn icon(&self) -> IconName {
        IconName::Globe
    }

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn detail(&self) -> Option<SharedString> {
        self.detail.clone()
    }

    fn status(&self) -> ToolStatus {
        self.status
    }
}

#[derive(Clone)]
pub(in crate::gui::chat_history) struct DynamicTool {
    title: SharedString,
    detail: Option<SharedString>,
    status: ToolStatus,
}

impl DynamicTool {
    pub(super) fn new(
        item: &ThreadItem,
        status: ToolStatus,
        progress: Option<&[SharedString]>,
    ) -> Option<Self> {
        let ThreadItem::DynamicToolCall {
            namespace,
            tool,
            arguments,
            content_items,
            duration_ms,
            ..
        } = item
        else {
            return None;
        };
        let name = namespace
            .as_ref()
            .map(|namespace| format!("{namespace}.{tool}"))
            .unwrap_or_else(|| tool.clone());
        let action = if matches!(status, ToolStatus::Running) {
            "Calling"
        } else {
            "Called"
        };
        // Dynamic tools carry their registered tool name and raw arguments, so a
        // known harness tool can expose what it actually touched (file, query,
        // URL, command) instead of a bare `Called read`. Namespaced dynamic
        // tools keep the generic label because their names live in a different
        // vocabulary.
        let title = namespace
            .is_none()
            .then(|| dynamic_tool_title(tool, arguments, status))
            .flatten()
            .unwrap_or_else(|| format!("{action} {name}"));
        let mut details = vec![format_json(arguments)];
        if let Some(content_items) = content_items {
            details.push(format!("output:\n{}", format_json(content_items)));
        }
        if let Some(duration_ms) = duration_ms {
            details.push(format!("duration: {duration_ms} ms"));
        }
        Some(Self {
            title: title.into(),
            detail: append_progress(Some(details.join("\n\n")), progress),
            status,
        })
    }
}

/// Read a string field from a tool call's raw JSON arguments.
fn argument_str<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

/// Collapse a multi-line argument into a single line for a card title.
fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build a descriptive title for a known dsh tool from its registered name and
/// arguments, mirroring the harness `presentCall` labels. Returns `None` for an
/// unknown tool or when the arguments are missing/reshaped, so the caller falls
/// back to the generic tool name.
fn dynamic_tool_title(tool: &str, arguments: &Value, status: ToolStatus) -> Option<String> {
    let verb = |running: &'static str, done: &'static str| {
        if matches!(status, ToolStatus::Running) {
            running
        } else {
            done
        }
    };
    let title = match tool {
        "read" => {
            let path = argument_str(arguments, "file_path")?;
            format!("{} {path}{}", verb("Reading", "Read"), read_window(arguments))
        }
        "write" => format!(
            "{} {}",
            verb("Writing", "Wrote"),
            argument_str(arguments, "file_path")?
        ),
        "edit" => format!(
            "{} {}",
            verb("Editing", "Edited"),
            argument_str(arguments, "file_path")?
        ),
        "grep" => {
            let pattern = argument_str(arguments, "pattern")?;
            let scope = argument_str(arguments, "path")
                .map(|path| format!(" in {path}"))
                .unwrap_or_default();
            let filter = argument_str(arguments, "include")
                .map(|include| format!(" ({include})"))
                .unwrap_or_default();
            format!(
                "{} {pattern}{scope}{filter}",
                verb("Searching", "Searched")
            )
        }
        "glob" => {
            let pattern = argument_str(arguments, "pattern")?;
            let scope = argument_str(arguments, "path")
                .map(|path| format!(" in {path}"))
                .unwrap_or_default();
            format!("{} {pattern}{scope}", verb("Finding", "Found"))
        }
        "web_search" => {
            let queries = arguments.get("queries")?.as_array()?;
            let queries = queries
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            if queries.is_empty() {
                return None;
            }
            format!("{} {queries}", verb("Searching", "Searched"))
        }
        "web_fetch" => format!(
            "{} {}",
            verb("Fetching", "Fetched"),
            argument_str(arguments, "url")?
        ),
        "bash" => format!(
            "{} {}",
            verb("Running", "Ran"),
            single_line(argument_str(arguments, "command")?)
        ),
        "terminal_open" => {
            let name = argument_str(arguments, "name")
                .or_else(|| argument_str(arguments, "type"))
                .unwrap_or("terminal");
            format!("{} terminal {name}", verb("Opening", "Opened"))
        }
        "terminal_send" => {
            let text = argument_str(arguments, "text")
                .filter(|text| !text.is_empty())
                .unwrap_or("(send input)");
            format!("{} {}", verb("Sending", "Sent"), single_line(text))
        }
        _ => return None,
    };
    Some(title)
}

/// The `(offset - limit)` suffix the read tool appends to its title, mirroring
/// the harness presenter. Omitted when neither argument is present.
fn read_window(arguments: &Value) -> String {
    let offset = arguments.get("offset").and_then(Value::as_u64);
    let limit = arguments.get("limit").and_then(Value::as_u64);
    match (offset, limit) {
        (offset, Some(limit)) if limit > 0 => {
            let start = offset.unwrap_or(1);
            format!(" ({start} - {})", start + limit - 1)
        }
        (Some(offset), _) => format!(" (from line {offset})"),
        _ => String::new(),
    }
}

impl SimpleTool for DynamicTool {
    fn icon(&self) -> IconName {
        IconName::Asterisk
    }

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn detail(&self) -> Option<SharedString> {
        self.detail.clone()
    }

    fn status(&self) -> ToolStatus {
        self.status
    }
}

use codex_app_server_protocol::ThreadItem;
use gpui::SharedString;
use gpui_component::IconName;

use super::simple::{SimpleTool, ToolStatus, append_progress};

#[derive(Clone)]
pub(in crate::gui::chat_history) struct ImageViewTool {
    path: SharedString,
    status: ToolStatus,
}

impl ImageViewTool {
    pub(super) fn new(item: &ThreadItem, status: ToolStatus) -> Option<Self> {
        let ThreadItem::ImageView { path, .. } = item else {
            return None;
        };
        Some(Self {
            path: path.to_string().into(),
            status,
        })
    }
}

impl SimpleTool for ImageViewTool {
    fn icon(&self) -> IconName {
        IconName::Eye
    }

    fn title(&self) -> SharedString {
        format!("Viewed {}", self.path).into()
    }

    fn detail(&self) -> Option<SharedString> {
        None
    }

    fn status(&self) -> ToolStatus {
        self.status
    }
}

#[derive(Clone)]
pub(in crate::gui::chat_history) struct ImageGenerationTool {
    title: SharedString,
    detail: Option<SharedString>,
    status: ToolStatus,
}

impl ImageGenerationTool {
    pub(super) fn new(
        item: &ThreadItem,
        status: ToolStatus,
        progress: Option<&[SharedString]>,
    ) -> Option<Self> {
        let ThreadItem::ImageGeneration(item) = item else {
            return None;
        };
        let title = match status {
            ToolStatus::Running => "Generating image",
            ToolStatus::Succeeded => "Generated image",
            ToolStatus::Failed => "Image generation failed",
        };
        let mut details = Vec::new();
        if let Some(prompt) = &item.revised_prompt {
            details.push(prompt.clone());
        }
        if let Some(path) = &item.saved_path {
            details.push(format!("saved to: {}", path.display()));
        }
        if item.transparent_background == Some(true) {
            details.push("transparent background".into());
        }
        Some(Self {
            title: title.into(),
            detail: append_progress((!details.is_empty()).then(|| details.join("\n")), progress),
            status,
        })
    }

    pub(super) fn status(&self) -> ToolStatus {
        self.status
    }
}

impl SimpleTool for ImageGenerationTool {
    fn icon(&self) -> IconName {
        IconName::Palette
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

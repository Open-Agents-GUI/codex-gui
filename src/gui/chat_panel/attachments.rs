use crate::gui::ComposerAttachment;
use gpui::{ClipboardEntry, Context, Image, ImageFormat, Window};
use gpui_component::input::Paste;
use std::{fs, path::Path};

use super::ChatPanel;

impl ChatPanel {
    pub(super) fn paste_into_composer(
        &mut self,
        _: &Paste,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(clipboard) = cx.read_from_clipboard() else {
            return;
        };
        if matches!(
            clipboard.entries().first(),
            Some(ClipboardEntry::String(_)) | None
        ) {
            return;
        }
        let image_is_primary =
            matches!(clipboard.entries().first(), Some(ClipboardEntry::Image(_)));

        let mut attachments = Vec::new();
        for entry in clipboard.into_entries() {
            match entry {
                ClipboardEntry::Image(image) => match pasted_image_attachment(image) {
                    Ok(attachment) => attachments.push(attachment),
                    Err(error) => tracing::error!(%error, "failed to attach pasted image"),
                },
                ClipboardEntry::ExternalPaths(paths) => {
                    attachments.extend(
                        paths
                            .paths()
                            .iter()
                            .filter_map(|path| image_attachment_from_path(path)),
                    );
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if attachments.is_empty() {
            if image_is_primary {
                cx.stop_propagation();
            }
            return;
        }

        cx.stop_propagation();
        self.add_composer_attachments(attachments, cx);
    }
}

fn pasted_image_attachment(image: Image) -> std::io::Result<ComposerAttachment> {
    let attachment_dir = std::env::temp_dir().join("codex-gui").join("attachments");
    fs::create_dir_all(&attachment_dir)?;
    let file_name = format!(
        "pasted-image-{}.{}",
        uuid::Uuid::new_v4().simple(),
        image.format.extension()
    );
    let path = attachment_dir.join(&file_name);
    fs::write(&path, &image.bytes)?;
    Ok(ComposerAttachment::image(file_name.into(), path, image))
}

fn image_attachment_from_path(path: &Path) -> Option<ComposerAttachment> {
    let format = image_format_for_path(path)?;
    let bytes = fs::read(path).ok()?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Image")
        .to_owned();
    Some(ComposerAttachment::image(
        name.into(),
        path.to_path_buf(),
        Image::from_bytes(format, bytes),
    ))
}

fn image_format_for_path(path: &Path) -> Option<ImageFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::Webp),
        "gif" => Some(ImageFormat::Gif),
        "svg" => Some(ImageFormat::Svg),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "ico" => Some(ImageFormat::Ico),
        "pbm" | "pgm" | "ppm" | "pnm" => Some(ImageFormat::Pnm),
        _ => None,
    }
}

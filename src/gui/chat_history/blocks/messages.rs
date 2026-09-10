use std::{path::Path, sync::Arc};

use codex_app_server_protocol::UserInput;
use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, IntoElement, ObjectFit, ParentElement,
    SharedString, Styled, Window, div, img, prelude::*, px,
};
use gpui_component::{
    IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    spinner::Spinner,
    theme::Theme,
};

use super::{
    super::motion::{AnimatedUserMessage, SendAnimationLaunch, UserMessageTarget},
    UserMessageDelivery,
};

pub(super) fn render_assistant_header(author: &'static str, theme: &Theme) -> gpui::Div {
    div()
        .w_full()
        .min_w_0()
        .pt_2()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(author)
}

/// Flatten a user message's protocol content into the text shown in the bubble.
///
/// Attachments that render as their own element (local images) contribute no
/// text. Inputs this client cannot render fall back to compact markers so the
/// message remains legible.
fn user_message_body(content: &[UserInput]) -> SharedString {
    let mut parts = Vec::new();
    for input in content {
        match input {
            UserInput::Text { text, .. } => parts.push(text.clone()),
            UserInput::LocalImage { .. } => {}
            UserInput::Image { .. } => parts.push("[Image]".to_string()),
            UserInput::Audio { url } => parts.push(format!("[Audio: {url}]")),
            UserInput::LocalAudio { path } => parts.push(format!("[Audio: {}]", path.display())),
            UserInput::Skill { name, path } => {
                parts.push(format!("[Skill: {name} ({})]", path.display()))
            }
            UserInput::Mention { name, path } => parts.push(format!("[Mention: {name} ({path})]")),
        }
    }
    parts.join("\n").into()
}

fn local_image_paths(content: &[UserInput]) -> Vec<&Path> {
    content
        .iter()
        .filter_map(|input| match input {
            UserInput::LocalImage { path, .. } => Some(path.as_path()),
            _ => None,
        })
        .collect()
}

pub(super) fn render_user(
    key: &str,
    content: Arc<[UserInput]>,
    delivery: UserMessageDelivery,
    actions_available: bool,
    animation: Option<SendAnimationLaunch>,
    theme: &Theme,
    on_animation_complete: impl FnOnce(&mut App) + 'static,
    on_edit: impl Fn(String, &ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let body = user_message_body(&content);
    let images = local_image_paths(&content);
    let copy_body = body.clone();
    let edit_body = body.clone();
    let has_body = !body.is_empty();
    // An image-only message has no bubble to fly, so skip the send animation
    // for it and reveal the images directly.
    let animation = animation.filter(|_| has_body);
    let animation_target = animation.as_ref().map(|_| UserMessageTarget::default());
    let animation_overlay = animation
        .as_ref()
        .map(|_| render_user_bubble(body.clone(), theme).into_any_element());
    let animating = animation_target.is_some();
    let delivery = if animating {
        UserMessageDelivery::Sending
    } else {
        delivery
    };
    let bubble = has_body.then(|| match &animation_target {
        Some(target) => target
            .observe(render_user_bubble(body.clone(), theme).opacity(0.))
            .into_any_element(),
        None => render_user_bubble(body.clone(), theme).into_any_element(),
    });
    let images_row = (!images.is_empty()).then(|| render_user_images(&images, theme));

    let row = div()
        .w_full()
        .min_w_0()
        .overflow_x_hidden()
        .py_2()
        .flex()
        .justify_end()
        .child(
            div()
                .w_full()
                .max_w(px(620.))
                .min_w_0()
                .flex()
                .flex_col()
                .items_end()
                .when_some(images_row, |column, images| column.child(images))
                .when_some(bubble, |column, bubble| column.child(bubble))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .pt_1()
                        .pr_1()
                        .when(matches!(delivery, UserMessageDelivery::Sending), |footer| {
                            footer
                                .child(Spinner::new().xsmall().color(theme.muted_foreground))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child("Sending…"),
                                )
                        })
                        .when(matches!(delivery, UserMessageDelivery::Failed), |footer| {
                            footer.child(div().text_xs().text_color(theme.danger).child("Not sent"))
                        })
                        .when(actions_available, |footer| {
                            footer
                                .child(
                                    Button::new(format!("copy-user-message-{key}"))
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Copy)
                                        .tooltip("Copy message")
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy_body.to_string(),
                                            ));
                                        }),
                                )
                                .child(
                                    Button::new(format!("edit-user-message-{key}"))
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Replace)
                                        .tooltip("Edit message in a fork")
                                        .on_click(move |event, window, cx| {
                                            on_edit(edit_body.to_string(), event, window, cx)
                                        }),
                                )
                        }),
                ),
        );

    if let Some(animation) = animation {
        AnimatedUserMessage::new(
            format!("animated-user-message-{key}"),
            animation,
            animation_target.expect("animation target must exist"),
            row.into_any_element(),
            animation_overlay.expect("animation overlay must exist"),
            on_animation_complete,
        )
        .into_any_element()
    } else {
        row.into_any_element()
    }
}

pub(super) fn render_assistant_actions(
    key: &str,
    body: SharedString,
    on_fork: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let copy_body = body.clone();
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_1()
        .pt_1()
        .child(
            Button::new(format!("copy-assistant-message-{key}"))
                .xsmall()
                .ghost()
                .icon(IconName::Copy)
                .tooltip("Copy response")
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_body.to_string()));
                }),
        )
        .child(
            Button::new(format!("fork-assistant-message-{key}"))
                .xsmall()
                .ghost()
                .icon(IconName::Network)
                .tooltip("Fork chat from this response")
                .on_click(on_fork),
        )
        .into_any_element()
}

fn render_user_images(images: &[&Path], theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .justify_end()
        .gap_2()
        .max_w(px(620.))
        .min_w_0()
        .pb_2()
        .children(images.iter().map(|path| render_user_image(path, theme)))
        .into_any_element()
}

fn render_user_image(path: &Path, theme: &Theme) -> AnyElement {
    let border = theme.border;
    let muted_foreground = theme.muted_foreground;
    let label: SharedString = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Image")
        .to_owned()
        .into();

    div()
        .min_w_0()
        .overflow_hidden()
        .rounded_xl()
        .border_1()
        .border_color(border)
        .bg(theme.muted)
        .child(
            img(path.to_path_buf())
                .max_w(px(560.))
                .max_h(px(240.))
                .object_fit(ObjectFit::Contain)
                .with_fallback(move || {
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(muted_foreground)
                        .child(label.clone())
                        .into_any_element()
                }),
        )
        .into_any_element()
}

fn render_user_bubble(body: SharedString, theme: &Theme) -> gpui::Div {
    div()
        .min_w_0()
        .max_w(px(620.))
        .overflow_x_hidden()
        .rounded_3xl()
        .bg(theme.secondary)
        .px_3()
        .py_2()
        .text_base()
        .line_height(px(25.))
        .text_color(theme.secondary_foreground)
        .whitespace_normal()
        .child(body)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn text(value: &str) -> UserInput {
        UserInput::Text {
            text: value.to_string(),
            text_elements: Vec::new(),
        }
    }

    fn local_image(path: &str) -> UserInput {
        UserInput::LocalImage {
            path: PathBuf::from(path),
            detail: None,
        }
    }

    #[test]
    fn body_keeps_text_and_omits_rendered_images() {
        let content = vec![text("Describe this"), local_image("/tmp/a.png")];

        assert_eq!(user_message_body(&content), "Describe this");
    }

    #[test]
    fn image_only_message_has_no_body_but_reports_paths() {
        let content = vec![local_image("/tmp/a.png"), local_image("/tmp/b.png")];

        assert!(user_message_body(&content).is_empty());
        assert_eq!(
            local_image_paths(&content),
            vec![Path::new("/tmp/a.png"), Path::new("/tmp/b.png")]
        );
    }

    #[test]
    fn inline_data_url_keeps_a_compact_marker() {
        let content = vec![UserInput::Image {
            url: "data:image/png;base64,AAAA".to_string(),
            detail: None,
        }];

        assert_eq!(user_message_body(&content), "[Image]");
        assert!(local_image_paths(&content).is_empty());
    }
}

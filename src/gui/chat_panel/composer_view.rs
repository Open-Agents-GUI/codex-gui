use crate::gui::{ApprovalsReviewer, ComposerAttachment, approvals_reviewer_label};
use gpui::{Context, IntoElement, ObjectFit, ParentElement, Styled, div, img, prelude::*, px};
use gpui_component::{
    ActiveTheme as _, IconName, Side, Sizable as _, box_shadow,
    button::{Button, ButtonVariants as _},
    input::Textarea,
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement,
};

use super::ChatPanel;

impl ChatPanel {
    fn composer_attachment_strip(
        &self,
        attachments: Vec<ComposerAttachment>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .w_full()
            .min_w_0()
            .h(px(82.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .overflow_x_scrollbar()
            .children(attachments.into_iter().map(|attachment| {
                let attachment_id = attachment.id.clone();
                let attachment_name = attachment.name.clone();
                div()
                    .relative()
                    .size(px(72.))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .rounded_xl()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .child(
                        img(attachment.preview())
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(
                        div().absolute().top_1().right_1().child(
                            Button::new(format!("remove-composer-attachment-{attachment_id}"))
                                .xsmall()
                                .ghost()
                                .rounded(px(999.))
                                .icon(IconName::Close)
                                .tooltip(format!("Remove {attachment_name}"))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.remove_composer_attachment(&attachment_id, cx);
                                })),
                        ),
                    )
            }))
    }

    pub(super) fn composer_surface(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings_chat = self.composer_chat();
        let attachments = self.composer_attachments(cx);
        let (default_settings, models, permission_profiles) = {
            let state = self.state.read(cx);
            (
                state.new_chat_settings.clone(),
                state.available_models.clone(),
                state.permission_profiles.clone(),
            )
        };
        let settings = settings_chat
            .as_ref()
            .map(|chat| chat.read(cx).settings.clone())
            .unwrap_or(default_settings);
        let model_label =
            if let Some(model) = models.iter().find(|model| model.id == settings.model) {
                model.display_name.clone()
            } else {
                settings.model.clone()
            };
        let effort_options = models
            .iter()
            .find(|model| model.id == settings.model)
            .map(|model| model.supported_efforts.clone())
            .filter(|efforts| !efforts.is_empty())
            .unwrap_or_else(|| {
                vec![
                    "none".into(),
                    "minimal".into(),
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "xhigh".into(),
                ]
            });
        let turn_running = self.active_chat_turn_running(cx);
        let answering_input = self
            .composer_chat()
            .is_some_and(|chat| chat.read(cx).pending_freeform_input().is_some());
        let can_stop = turn_running && !answering_input;
        let user_message_sending = self.user_message_sending(cx);

        div()
            .capture_action(cx.listener(Self::paste_into_composer))
            .w_full()
            .max_w(px(1000.))
            .rounded_3xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().input_background())
            .shadow(vec![box_shadow(
                0.,
                4.,
                12.,
                2.,
                cx.theme().transparent.alpha(0.1),
            )])
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .when(!attachments.is_empty(), |surface| {
                surface.child(self.composer_attachment_strip(attachments, cx))
            })
            .child(
                Textarea::new(&self.composer_input)
                    .appearance(false)
                    .min_h(px(60.))
                    .w_full(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("composer-model")
                                    .small()
                                    .ghost()
                                    .text_color(cx.theme().muted_foreground)
                                    .icon(IconName::Cpu)
                                    .label(model_label)
                                    .tooltip("Model settings")
                                    .dropdown_menu({
                                        let parent = self.parent.clone();
                                        let settings_chat = settings_chat.clone();
                                        let models = models.clone();
                                        let selected_model = settings.model.clone();
                                        move |menu, _, _| {
                                            let mut menu = menu
                                                .min_w(260.)
                                                .max_h(px(360.))
                                                .scrollable(true)
                                                .check_side(Side::Left);
                                            if models.is_empty() {
                                                return menu.item(
                                                    PopupMenuItem::new("Loading models")
                                                        .disabled(true),
                                                );
                                            }
                                            for model in &models {
                                                let id = model.id.clone();
                                                let label = model.display_name.clone();
                                                let parent = parent.clone();
                                                let settings_chat = settings_chat.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(label)
                                                        .checked(model.id == selected_model)
                                                        .on_click(move |_, _, cx| {
                                                            let id = id.clone();
                                                            let settings_chat =
                                                                settings_chat.clone();
                                                            let _ =
                                                                parent.update(cx, |parent, cx| {
                                                                    parent.set_model(
                                                                        settings_chat,
                                                                        id,
                                                                        cx,
                                                                    )
                                                                });
                                                        }),
                                                );
                                            }
                                            menu
                                        }
                                    }),
                            )
                            .child(
                                Button::new("composer-permissions")
                                    .small()
                                    .ghost()
                                    .text_color(cx.theme().muted_foreground)
                                    .icon(IconName::Check)
                                    .label(approvals_reviewer_label(settings.approvals_reviewer))
                                    .tooltip("Permission settings")
                                    .dropdown_menu({
                                        let parent = self.parent.clone();
                                        let settings_chat = settings_chat.clone();
                                        let settings = settings.clone();
                                        let permission_profiles = permission_profiles.clone();
                                        move |menu, _, _| {
                                            let mut menu = menu
                                                .min_w(240.)
                                                .check_side(Side::Left)
                                                .item(PopupMenuItem::label("Permissions"));
                                            for profile in &permission_profiles {
                                                let id = profile.id.clone();
                                                let parent = parent.clone();
                                                let settings_chat = settings_chat.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(profile.label.clone())
                                                        .checked(
                                                            settings.permission_profile
                                                                == profile.id,
                                                        )
                                                        .on_click(move |_, _, cx| {
                                                            let id = id.clone();
                                                            let settings_chat =
                                                                settings_chat.clone();
                                                            let _ =
                                                                parent.update(cx, |parent, cx| {
                                                                    parent.set_permission_profile(
                                                                        settings_chat,
                                                                        id,
                                                                        cx,
                                                                    )
                                                                });
                                                        }),
                                                );
                                            }
                                            menu = menu
                                                .separator()
                                                .item(PopupMenuItem::label("Approvals"));
                                            for reviewer in [
                                                ApprovalsReviewer::User,
                                                ApprovalsReviewer::AutoReview,
                                            ] {
                                                let parent = parent.clone();
                                                let settings_chat = settings_chat.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(approvals_reviewer_label(
                                                        reviewer,
                                                    ))
                                                    .checked(
                                                        settings.approvals_reviewer == reviewer,
                                                    )
                                                    .on_click(move |_, _, cx| {
                                                        let settings_chat = settings_chat.clone();
                                                        let _ = parent.update(cx, |parent, cx| {
                                                            parent.set_approvals_reviewer(
                                                                settings_chat,
                                                                reviewer,
                                                                cx,
                                                            )
                                                        });
                                                    }),
                                                );
                                            }
                                            menu
                                        }
                                    }),
                            )
                            .child(
                                Button::new("composer-effort")
                                    .small()
                                    .ghost()
                                    .text_color(cx.theme().muted_foreground)
                                    .icon(IconName::LoaderCircle)
                                    .label(format!("Effort {}", title_case(&settings.effort)))
                                    .tooltip("Thinking effort settings")
                                    .dropdown_menu({
                                        let parent = self.parent.clone();
                                        let settings_chat = settings_chat.clone();
                                        let selected_effort = settings.effort.clone();
                                        move |menu, _, _| {
                                            let mut menu = menu.min_w(190.).check_side(Side::Left);
                                            for effort in &effort_options {
                                                let value = effort.clone();
                                                let parent = parent.clone();
                                                let settings_chat = settings_chat.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(title_case(effort))
                                                        .checked(*effort == selected_effort)
                                                        .on_click(move |_, _, cx| {
                                                            let value = value.clone();
                                                            let settings_chat =
                                                                settings_chat.clone();
                                                            let _ =
                                                                parent.update(cx, |parent, cx| {
                                                                    parent.set_effort(
                                                                        settings_chat,
                                                                        value,
                                                                        cx,
                                                                    )
                                                                });
                                                        }),
                                                );
                                            }
                                            menu
                                        }
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(can_stop, |actions| {
                                actions.child(
                                    Button::new("steer-composer-turn")
                                        .small()
                                        .primary()
                                        .loading(user_message_sending)
                                        .rounded(px(999.))
                                        .icon(IconName::ArrowUp)
                                        .tooltip("Steer")
                                        .on_click(cx.listener(|view, _, window, cx| {
                                            view.steer_composer_turn(window, cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("send-or-stop-composer-turn")
                                    .small()
                                    .loading(!can_stop && user_message_sending)
                                    .when(!can_stop, |button| button.primary())
                                    .when(can_stop, |button| button.danger())
                                    .rounded(px(999.))
                                    .icon(if can_stop {
                                        IconName::Close
                                    } else {
                                        IconName::ArrowUp
                                    })
                                    .tooltip(if can_stop { "Stop" } else { "Send" })
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        let answering_input =
                                            view.composer_chat().is_some_and(|chat| {
                                                chat.read(cx).pending_freeform_input().is_some()
                                            });
                                        if view.active_chat_turn_running(cx) && !answering_input {
                                            view.stop_active_turn(cx);
                                        } else {
                                            view.send_composer_turn(window, cx);
                                        }
                                    })),
                            ),
                    ),
            )
    }

    pub(super) fn composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .px_5()
            .pb_5()
            .flex()
            .justify_center()
            .child(self.composer_surface(cx))
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

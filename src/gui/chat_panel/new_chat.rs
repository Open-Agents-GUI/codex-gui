use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::Input,
    scroll::ScrollableElement,
};

use super::ChatPanel;

impl ChatPanel {
    fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.select_project(index, cx));
        });
    }

    fn select_new_chat_projectless(&mut self, cx: &mut Context<Self>) {
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.select_new_chat_projectless(cx));
        });
    }

    pub(super) fn add_project_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.project_path_input.update(cx, |input, cx| {
            let path = input.value().trim().to_string();
            if !path.is_empty() {
                input.set_value("", window, cx);
            }
            path
        });
        if path.is_empty() {
            return;
        }
        let parent = self.parent.clone();
        cx.defer(move |cx| {
            let _ = parent.update(cx, |parent, cx| parent.add_project(path, cx));
        });
    }

    pub(super) fn new_chat_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (projects, active_project, active_project_name, projectless) = {
            let state = self.state.read(cx);
            let active_project_name = state
                .active_project()
                .map(|project| project.read(cx).name.to_string())
                .unwrap_or_else(|| "this project".into());
            (
                state.projects.clone(),
                state.active_project,
                active_project_name,
                self.window_state.read(cx).new_chat_projectless || state.projects.is_empty(),
            )
        };
        let heading = if projectless {
            "What should we build?".to_owned()
        } else {
            format!("What should we build in {active_project_name}?")
        };

        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .px_5()
            .pb_20()
            .gap_7()
            .child(
                div()
                    .max_w(px(820.))
                    .w_full()
                    .text_center()
                    .text_3xl()
                    .font_weight(gpui::FontWeight::LIGHT)
                    .child(heading),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(820.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.composer_surface(cx))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .overflow_x_scrollbar()
                            .child(
                                Button::new("new-chat-projectless")
                                    .small()
                                    .ghost()
                                    .selected(projectless)
                                    .label("No project")
                                    .tooltip("Create a chat in Documents/Codex")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.select_new_chat_projectless(cx)
                                    })),
                            )
                            .children(projects.iter().enumerate().map(|(index, project)| {
                                let project = project.read(cx);
                                Button::new(format!("new-chat-project-{index}"))
                                    .small()
                                    .ghost()
                                    .selected(!projectless && index == active_project)
                                    .icon(if !projectless && index == active_project {
                                        IconName::FolderOpen
                                    } else {
                                        IconName::Folder
                                    })
                                    .label(project.name.clone())
                                    .tooltip(project.path.clone())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.select_project(index, cx)
                                    }))
                            })),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(cx.theme().background)
                                    .child(
                                        Input::new(&self.project_path_input)
                                            .appearance(false)
                                            .h(px(34.))
                                            .w_full(),
                                    ),
                            )
                            .child(
                                Button::new("add-project")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Plus)
                                    .label("Add project")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.add_project_from_input(window, cx)
                                    })),
                            ),
                    ),
            )
    }
}

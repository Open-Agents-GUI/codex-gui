use super::{
    PAGE_SIZE, SIDEBAR_LIST_OVERDRAW, SIDEBAR_ROW_HEIGHT, Sidebar, SidebarRowDisplayStatus,
};
use crate::app::CodexGui;
use crate::gui::WorkspaceState;
use gpui::{Context, Entity, ListAlignment, ListState, WeakEntity};
use std::collections::{HashMap, HashSet};

impl Sidebar {
    pub fn new(
        parent: WeakEntity<CodexGui>,
        state: Entity<WorkspaceState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![cx.observe(&state, |view, _, cx| {
            view.sync_rows_from_state(cx);
            cx.notify();
        })];
        let mut sidebar = Self {
            parent,
            state,
            should_move_window: false,
            collapsed_projects: HashSet::new(),
            visible_project_count: PAGE_SIZE,
            visible_projectless_count: PAGE_SIZE,
            project_chat_visible_counts: HashMap::new(),
            display_status: SidebarRowDisplayStatus::new(0, 0, 0, 0, 0, 0, None, None, 0, 0, true),
            list_state: ListState::new(0, ListAlignment::Top, SIDEBAR_LIST_OVERDRAW)
                .with_uniform_item_height(SIDEBAR_ROW_HEIGHT),
            list_active_project: None,
            project_fold_animation: None,
            departing_project_fold_animation: None,
            project_fold_generation: 0,
            pending_project_expansion: None,
            observed_active_project: None,
            _active_project_subscription: None,
            _subscriptions: subscriptions,
        };
        sidebar.display_status = sidebar.rows_from_state(cx);
        sidebar.insert_rows(0, sidebar.display_status.len());
        sidebar.update_active_project_subscription(cx);
        sidebar
    }
}

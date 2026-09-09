use crate::app::CodexGui;
use crate::gui::{ProjectState, WorkspaceState};
use gpui::{Entity, ListState, Subscription, WeakEntity, px};
use std::collections::{HashMap, HashSet};

mod actions;
mod lifecycle;
mod motion;
mod reconcile;
mod rows;
mod view;

use motion::ProjectFoldAnimation;
use rows::{PaginateKind, SidebarRowDisplayStatus};

const SIDEBAR_ROW_HEIGHT: gpui::Pixels = px(34.);
/// Vertical breathing room inside each row slot. Spacing between list rows must
/// come from padding, because the list measures border-box heights only and
/// margins on rows are ignored.
const SIDEBAR_ROW_GAP: gpui::Pixels = px(2.);
const SIDEBAR_LIST_OVERDRAW: gpui::Pixels = px(170.);
const PAGE_SIZE: usize = 10;

pub struct Sidebar {
    parent: WeakEntity<CodexGui>,
    state: Entity<WorkspaceState>,
    should_move_window: bool,
    collapsed_projects: HashSet<String>,
    visible_project_count: usize,
    visible_projectless_count: usize,
    project_chat_visible_counts: HashMap<String, usize>,
    /// Which projects are expanded? How many chats to show? etc.
    display_status: SidebarRowDisplayStatus,
    list_state: ListState,
    list_active_project: Option<usize>,
    project_fold_animation: Option<ProjectFoldAnimation>,
    departing_project_fold_animation: Option<ProjectFoldAnimation>,
    project_fold_generation: u64,
    pending_project_expansion: Option<String>,
    observed_active_project: Option<Entity<ProjectState>>,
    _active_project_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

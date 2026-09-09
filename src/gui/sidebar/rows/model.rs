use std::ops::Range;

/// A row produced lazily from the sidebar's canonical and view state.
#[derive(Clone, Debug)]
pub enum SidebarRow<Project, Chat> {
    /// "Projects" or "Chats" label.
    SectionHeader { label: &'static str },
    /// Project folder that contains chats in it.
    Project {
        project_index: usize,
        project: Project,
        selected: bool,
        expanded: bool,
    },
    /// A thread with a user-specified project dir.
    Chat {
        project_index: usize,
        chat_index: usize,
        chat: Chat,
        selected: bool,
    },
    /// A project-less chat.
    ProjectlessChat {
        chat_index: usize,
        chat: Chat,
        selected: bool,
    },
    /// Show more/show all button.
    ShowMore { kind: PaginateKind },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaginateKind {
    Projects,
    ProjectlessChats,
    ProjectChats { path: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpandedProject {
    project_index: usize,
    path: String,
    active_chat: usize,
    chat_count: usize,
    visible_chat_count: usize,
}

impl ExpandedProject {
    fn has_pager(&self) -> bool {
        self.chat_count > self.visible_chat_count
    }

    fn child_count(&self) -> usize {
        self.visible_chat_count + usize::from(self.has_pager())
    }
}

/// Lightweight, lazily indexed projection of `WorkspaceState + Sidebar` view state.
///
/// This stores only counts and offsets. It does not materialize one value per
/// sidebar row; [`Self::row_at`] creates a row only when GPUI asks to render it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRowDisplayStatus {
    project_count: usize,
    visible_project_count: usize,
    projectless_chat_count: usize,
    visible_projectless_chat_count: usize,
    active_project: usize,
    active_projectless_chat: Option<usize>,
    expanded_project: Option<ExpandedProject>,
    /// The previously active project, retained only while its children animate
    /// out during a project switch.
    departing_project: Option<ExpandedProject>,
}

impl SidebarRowDisplayStatus {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_count: usize,
        visible_project_count: usize,
        projectless_chat_count: usize,
        visible_projectless_chat_count: usize,
        active_project: usize,
        active_chat: usize,
        active_projectless_chat: Option<usize>,
        active_project_path: Option<String>,
        active_project_chat_count: usize,
        visible_project_chat_count: usize,
        active_project_collapsed: bool,
    ) -> Self {
        let visible_project_count = visible_project_count.min(project_count);
        let visible_projectless_chat_count =
            visible_projectless_chat_count.min(projectless_chat_count);
        let expanded_project = if active_project < visible_project_count
            && active_projectless_chat.is_none()
            && !active_project_collapsed
        {
            active_project_path.map(|path| ExpandedProject {
                project_index: active_project,
                path,
                active_chat,
                chat_count: active_project_chat_count,
                visible_chat_count: visible_project_chat_count.min(active_project_chat_count),
            })
        } else {
            None
        };

        Self {
            project_count,
            visible_project_count,
            projectless_chat_count,
            visible_projectless_chat_count,
            active_project,
            active_projectless_chat,
            expanded_project,
            departing_project: None,
        }
    }

    pub fn len(&self) -> usize {
        self.project_section_len() + self.projectless_section_len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn active_project(&self) -> usize {
        self.active_project
    }

    pub fn active_project_row(&self) -> Option<usize> {
        (self.project_count > 0 && self.active_project < self.visible_project_count).then(|| {
            let preceding_child_count = [&self.expanded_project, &self.departing_project]
                .into_iter()
                .filter_map(Option::as_ref)
                .filter(|expanded| expanded.project_index < self.active_project)
                .map(ExpandedProject::child_count)
                .sum::<usize>();
            1 + self.active_project + preceding_child_count
        })
    }

    pub fn expanded_project_path(&self) -> Option<&str> {
        self.expanded_project
            .as_ref()
            .map(|expanded| expanded.path.as_str())
    }

    pub fn departing_project_path(&self) -> Option<&str> {
        self.departing_project
            .as_ref()
            .map(|expanded| expanded.path.as_str())
    }

    pub fn departing_project_index(&self) -> Option<usize> {
        self.departing_project
            .as_ref()
            .map(|expanded| expanded.project_index)
    }

    pub fn retain_departing_project_from(&mut self, previous: &Self, path: &str) -> bool {
        let Some(departing) = previous
            .departing_project
            .as_ref()
            .or(previous.expanded_project.as_ref())
            .filter(|expanded| expanded.path == path)
        else {
            return false;
        };
        if self
            .expanded_project
            .as_ref()
            .is_none_or(|arriving| arriving.path == departing.path)
        {
            return false;
        }
        self.departing_project = Some(departing.clone());
        true
    }

    pub fn project_section_len(&self) -> usize {
        if self.project_count == 0 {
            return 0;
        }
        1 + self.project_body_len() + usize::from(self.has_project_pager())
    }

    pub fn project_base_len(&self) -> usize {
        self.project_section_len() - self.expanded_child_count()
    }

    pub fn projectless_section_len(&self) -> usize {
        if self.projectless_chat_count == 0 {
            return 0;
        }
        1 + self.visible_projectless_chat_count + usize::from(self.has_projectless_pager())
    }

    pub fn expanded_children_range(&self) -> Option<Range<usize>> {
        let expanded = self.expanded_project.as_ref()?;
        Some(self.children_range_for(expanded))
    }

    pub fn departing_children_range(&self) -> Option<Range<usize>> {
        let departing = self.departing_project.as_ref()?;
        Some(self.children_range_for(departing))
    }

    pub fn has_same_expanded_structure(&self, other: &Self) -> bool {
        fn same(left: &Option<ExpandedProject>, right: &Option<ExpandedProject>) -> bool {
            match (left, right) {
                (None, None) => true,
                (Some(left), Some(right)) => {
                    left.project_index == right.project_index
                        && left.path == right.path
                        && left.visible_chat_count == right.visible_chat_count
                        && left.has_pager() == right.has_pager()
                }
                _ => false,
            }
        }

        same(&self.expanded_project, &other.expanded_project)
            && same(&self.departing_project, &other.departing_project)
    }

    pub fn pagination_pager_index(&self, kind: &PaginateKind) -> Option<usize> {
        match kind {
            PaginateKind::Projects => self
                .has_project_pager()
                .then_some(1 + self.project_body_len()),
            PaginateKind::ProjectlessChats => self
                .has_projectless_pager()
                .then_some(self.project_section_len() + 1 + self.visible_projectless_chat_count),
            PaginateKind::ProjectChats { path } => {
                let expanded = self.expanded_project.as_ref()?;
                (expanded.path == *path && expanded.has_pager())
                    .then(|| self.children_range_for(expanded).start + expanded.visible_chat_count)
            }
        }
    }

    pub fn pagination_body_len(&self, kind: &PaginateKind) -> Option<usize> {
        match kind {
            PaginateKind::Projects => Some(self.project_body_len()),
            PaginateKind::ProjectlessChats => Some(self.visible_projectless_chat_count),
            PaginateKind::ProjectChats { path } => self
                .expanded_project
                .as_ref()
                .filter(|expanded| expanded.path == *path)
                .map(|expanded| expanded.visible_chat_count),
        }
    }

    pub fn row_at<Project: Clone, Chat: Clone>(
        &self,
        index: usize,
        projects: &[Project],
        projectless_chats: &[Chat],
        active_project_chats: &[Chat],
        departing_project_chats: &[Chat],
    ) -> Option<SidebarRow<Project, Chat>> {
        if index >= self.len() {
            return None;
        }

        if index < self.project_section_len() {
            return self.project_row(
                index,
                projects,
                active_project_chats,
                departing_project_chats,
            );
        }

        let section_index = index - self.project_section_len();
        if section_index == 0 {
            return Some(SidebarRow::SectionHeader { label: "Chats" });
        }
        let chat_index = section_index - 1;
        if chat_index < self.visible_projectless_chat_count {
            return projectless_chats.get(chat_index).cloned().map(|chat| {
                SidebarRow::ProjectlessChat {
                    chat_index,
                    chat,
                    selected: self.active_projectless_chat == Some(chat_index),
                }
            });
        }
        self.has_projectless_pager()
            .then_some(SidebarRow::ShowMore {
                kind: PaginateKind::ProjectlessChats,
            })
    }

    fn project_row<Project: Clone, Chat: Clone>(
        &self,
        index: usize,
        projects: &[Project],
        active_project_chats: &[Chat],
        departing_project_chats: &[Chat],
    ) -> Option<SidebarRow<Project, Chat>> {
        if index == 0 {
            return Some(SidebarRow::SectionHeader { label: "Projects" });
        }

        for (expanded, chats) in [
            (self.expanded_project.as_ref(), active_project_chats),
            (self.departing_project.as_ref(), departing_project_chats),
        ] {
            if let Some(expanded) = expanded {
                let child_range = self.children_range_for(expanded);
                if child_range.contains(&index) {
                    let child_index = index - child_range.start;
                    if child_index < expanded.visible_chat_count {
                        return chats
                            .get(child_index)
                            .cloned()
                            .map(|chat| SidebarRow::Chat {
                                project_index: expanded.project_index,
                                chat_index: child_index,
                                chat,
                                selected: expanded.project_index == self.active_project
                                    && child_index == expanded.active_chat,
                            });
                    }
                    return expanded.has_pager().then(|| SidebarRow::ShowMore {
                        kind: PaginateKind::ProjectChats {
                            path: expanded.path.clone(),
                        },
                    });
                }
            }
        }

        let preceding_child_count = [&self.expanded_project, &self.departing_project]
            .into_iter()
            .filter_map(Option::as_ref)
            .filter(|expanded| index >= self.children_range_for(expanded).end)
            .map(ExpandedProject::child_count)
            .sum::<usize>();
        let project_index = index.checked_sub(1 + preceding_child_count)?;
        if project_index < self.visible_project_count {
            return projects
                .get(project_index)
                .cloned()
                .map(|project| SidebarRow::Project {
                    project_index,
                    project,
                    selected: project_index == self.active_project
                        && self.active_projectless_chat.is_none(),
                    expanded: self
                        .expanded_project
                        .as_ref()
                        .is_some_and(|expanded| expanded.project_index == project_index)
                        || self
                            .departing_project
                            .as_ref()
                            .is_some_and(|expanded| expanded.project_index == project_index),
                });
        }

        self.has_project_pager().then_some(SidebarRow::ShowMore {
            kind: PaginateKind::Projects,
        })
    }

    fn project_body_len(&self) -> usize {
        self.visible_project_count + self.expanded_child_count()
    }

    fn expanded_child_count(&self) -> usize {
        [&self.expanded_project, &self.departing_project]
            .into_iter()
            .filter_map(Option::as_ref)
            .map(ExpandedProject::child_count)
            .sum()
    }

    fn children_range_for(&self, expanded: &ExpandedProject) -> Range<usize> {
        let preceding_child_count = [&self.expanded_project, &self.departing_project]
            .into_iter()
            .filter_map(Option::as_ref)
            .filter(|other| other.project_index < expanded.project_index)
            .map(ExpandedProject::child_count)
            .sum::<usize>();
        let start = 1 + expanded.project_index + preceding_child_count + 1;
        start..start + expanded.child_count()
    }

    fn has_project_pager(&self) -> bool {
        self.project_count > self.visible_project_count
    }

    fn has_projectless_pager(&self) -> bool {
        self.projectless_chat_count > self.visible_projectless_chat_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_rows<Project: Clone, Chat: Clone>(
        rows: &SidebarRowDisplayStatus,
        projects: &[Project],
        projectless: &[Chat],
        active_chats: &[Chat],
    ) -> Vec<SidebarRow<Project, Chat>> {
        (0..rows.len())
            .map(|index| {
                rows.row_at(index, projects, projectless, active_chats, &[])
                    .expect("every projected index should resolve")
            })
            .collect()
    }

    #[test]
    fn indexes_only_the_active_projects_visible_threads() {
        let projects = vec!["one", "two", "three"];
        let chats = (0..8).collect::<Vec<_>>();
        let rows = SidebarRowDisplayStatus::new(
            projects.len(),
            projects.len(),
            0,
            0,
            1,
            2,
            None,
            Some("two".into()),
            chats.len(),
            5,
            false,
        );
        let rows = collect_rows(&rows, &projects, &[] as &[i32], &chats);

        assert_eq!(rows.len(), 10);
        assert!(matches!(rows[0], SidebarRow::SectionHeader { .. }));
        assert!(matches!(
            rows[2],
            SidebarRow::Project {
                project_index: 1,
                expanded: true,
                ..
            }
        ));
        assert!(matches!(
            rows[5],
            SidebarRow::Chat {
                chat_index: 2,
                selected: true,
                ..
            }
        ));
        assert!(matches!(rows[8], SidebarRow::ShowMore { .. }));
        assert!(matches!(
            rows[9],
            SidebarRow::Project {
                project_index: 2,
                expanded: false,
                ..
            }
        ));
    }

    #[test]
    fn indexes_projectless_chats_in_their_own_section() {
        let projects = vec!["one"];
        let projectless = vec![10, 11, 12];
        let rows = SidebarRowDisplayStatus::new(
            projects.len(),
            projects.len(),
            projectless.len(),
            projectless.len(),
            0,
            0,
            Some(1),
            Some("one".into()),
            0,
            0,
            false,
        );
        let rows = collect_rows(&rows, &projects, &projectless, &[] as &[i32]);

        assert_eq!(rows.len(), 5);
        assert!(matches!(rows[0], SidebarRow::SectionHeader { .. }));
        assert!(matches!(
            rows[1],
            SidebarRow::Project {
                selected: false,
                expanded: false,
                ..
            }
        ));
        assert!(matches!(rows[2], SidebarRow::SectionHeader { .. }));
        assert!(matches!(
            rows[3],
            SidebarRow::ProjectlessChat {
                chat_index: 0,
                selected: false,
                ..
            }
        ));
        assert!(matches!(
            rows[4],
            SidebarRow::ProjectlessChat {
                chat_index: 2,
                selected: true,
                ..
            }
        ));
    }

    #[test]
    fn reports_pager_positions_and_body_lengths() {
        let rows = SidebarRowDisplayStatus::new(
            15,
            10,
            15,
            10,
            0,
            0,
            Some(0),
            Some("zero".into()),
            0,
            0,
            false,
        );

        assert_eq!(rows.len(), 24);
        assert_eq!(
            rows.pagination_pager_index(&PaginateKind::Projects),
            Some(11)
        );
        assert_eq!(
            rows.pagination_pager_index(&PaginateKind::ProjectlessChats),
            Some(23)
        );
        assert_eq!(rows.pagination_body_len(&PaginateKind::Projects), Some(10));
        assert_eq!(
            rows.pagination_body_len(&PaginateKind::ProjectlessChats),
            Some(10)
        );
    }

    #[test]
    fn collapse_removes_only_the_expanded_children_range() {
        let expanded =
            SidebarRowDisplayStatus::new(2, 2, 0, 0, 0, 0, None, Some("one".into()), 8, 8, false);
        let collapsed =
            SidebarRowDisplayStatus::new(2, 2, 0, 0, 0, 0, None, Some("one".into()), 8, 8, true);

        assert_eq!(expanded.len(), 11);
        assert_eq!(expanded.expanded_children_range(), Some(2..10));
        assert_eq!(collapsed.len(), 3);
        assert_eq!(collapsed.expanded_children_range(), None);
    }

    #[test]
    fn temporarily_indexes_departing_and_arriving_project_children() {
        let projects = vec!["one", "two", "three"];
        let departing_chats = vec![10, 11];
        let arriving_chats = vec![20, 21, 22];
        let previous = SidebarRowDisplayStatus::new(
            3,
            3,
            0,
            0,
            0,
            0,
            None,
            Some("one".into()),
            departing_chats.len(),
            departing_chats.len(),
            false,
        );
        let mut transitioning = SidebarRowDisplayStatus::new(
            3,
            3,
            0,
            0,
            1,
            0,
            None,
            Some("two".into()),
            arriving_chats.len(),
            arriving_chats.len(),
            false,
        );

        assert!(transitioning.retain_departing_project_from(&previous, "one"));
        assert_eq!(transitioning.departing_children_range(), Some(2..4));
        assert_eq!(transitioning.active_project_row(), Some(4));
        assert_eq!(transitioning.expanded_children_range(), Some(5..8));

        let rows = (0..transitioning.len())
            .map(|index| {
                transitioning
                    .row_at(
                        index,
                        &projects,
                        &[] as &[i32],
                        &arriving_chats,
                        &departing_chats,
                    )
                    .expect("every transition row should resolve")
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            rows[2],
            SidebarRow::Chat {
                project_index: 0,
                chat_index: 0,
                ..
            }
        ));
        assert!(matches!(
            rows[4],
            SidebarRow::Project {
                project_index: 1,
                expanded: true,
                ..
            }
        ));
        assert!(matches!(
            rows[5],
            SidebarRow::Chat {
                project_index: 1,
                chat_index: 0,
                ..
            }
        ));
    }
}

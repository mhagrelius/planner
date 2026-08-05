//! Projects, sections and labels.
//!
//! Sections are stored *inside* their project rather than in a flat list with
//! a project ID. A section has no meaning apart from its project, no view
//! shows sections across projects, and deleting a project must take its
//! sections with it — nesting makes all three true by construction instead of
//! by remembering.
//!
//! Labels are the opposite: they are deliberately global and flat, because
//! their whole purpose is to cut across the project tree.

use serde::{Deserialize, Serialize};

use super::color::Color;
use super::id::{FilterId, LabelId, ProjectId, SectionId};

/// How a project's tasks are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ViewStyle {
    /// One flat list, sections as headers.
    #[default]
    List,
    /// One column per section.
    Board,
}

/// How a project's tasks are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SortBy {
    /// Whatever order the user dragged them into.
    #[default]
    Manual,
    DueDate,
    Priority,
    Name,
    AddedAt,
}

/// A named group of tasks within a project. A board column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: SectionId,
    pub name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
    #[serde(default)]
    pub order: i32,
}

impl Section {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: SectionId::new(),
            name: name.into(),
            collapsed: false,
            order: 0,
        }
    }
}

/// A project. Projects nest; the Inbox is one with a reserved ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    #[serde(default)]
    pub color: Color,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,

    #[serde(default, skip_serializing_if = "is_false")]
    pub is_favorite: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_archived: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,

    #[serde(default)]
    pub view_style: ViewStyle,
    #[serde(default)]
    pub sort_by: SortBy,
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_completed: bool,
    #[serde(default)]
    pub order: i32,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Project {
    pub fn new(name: impl Into<String>, color: Color) -> Self {
        Self {
            id: ProjectId::new(),
            name: name.into(),
            color,
            description: String::new(),
            parent_id: None,
            sections: Vec::new(),
            is_favorite: false,
            is_archived: false,
            collapsed: false,
            view_style: ViewStyle::default(),
            sort_by: SortBy::default(),
            show_completed: false,
            order: 0,
        }
    }

    /// The Inbox: where a task with no stated project goes.
    ///
    /// Constructed rather than stored on first run, so a store whose Inbox has
    /// somehow been deleted still has one. It is not renameable or deletable
    /// in the UI, but it does take a colour like any other project.
    pub fn inbox() -> Self {
        Self {
            id: ProjectId::inbox(),
            order: -1, // always at the top of the sidebar
            ..Self::new("Inbox", Color::Slate)
        }
    }

    /// Whether this is the Inbox.
    pub fn is_inbox(&self) -> bool {
        self.id.is_inbox()
    }

    /// Find one of this project's sections.
    pub fn section(&self, id: &SectionId) -> Option<&Section> {
        self.sections.iter().find(|section| &section.id == id)
    }

    pub fn section_mut(&mut self, id: &SectionId) -> Option<&mut Section> {
        self.sections.iter_mut().find(|section| &section.id == id)
    }

    /// Add a section at the end.
    pub fn add_section(&mut self, mut section: Section) -> SectionId {
        section.order = self
            .sections
            .iter()
            .map(|existing| existing.order)
            .max()
            .map_or(0, |max| max + 1);
        let id = section.id.clone();
        self.sections.push(section);
        id
    }

    /// Remove a section, returning it. The caller is responsible for the tasks
    /// that pointed at it — the project has no way to reach them.
    pub fn remove_section(&mut self, id: &SectionId) -> Option<Section> {
        let index = self.sections.iter().position(|section| &section.id == id)?;
        Some(self.sections.remove(index))
    }

    /// Put a removed section back where it was.
    ///
    /// Unlike [`add_section`](Self::add_section) this keeps the section's own
    /// `order`, so an undone deletion lands in the place it was deleted from
    /// rather than at the end.
    pub fn restore_section(&mut self, section: Section) {
        self.sections.push(section);
    }

    /// Sections in display order.
    pub fn sections_ordered(&self) -> Vec<&Section> {
        let mut sections: Vec<&Section> = self.sections.iter().collect();
        sections.sort_by_key(|section| section.order);
        sections
    }
}

/// A tag that cuts across projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub id: LabelId,
    /// Unique, case-insensitively — `@Work` and `@work` are the same label,
    /// because quick-add cannot ask which one you meant.
    pub name: String,
    #[serde(default)]
    pub color: Color,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_favorite: bool,
    #[serde(default)]
    pub order: i32,
}

/// A query the user saved and named.
///
/// The whole reason the filter language exists: a built-in view is one of
/// these with the name and query written by us instead. Stored as the query
/// *text*, not a parsed tree — it has to survive a schema that learns new
/// terms, and the text is what the user gets back when they edit it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedFilter {
    pub id: FilterId,
    pub name: String,
    pub query: String,
    #[serde(default)]
    pub color: Color,
    #[serde(default)]
    pub order: i32,
}

impl SavedFilter {
    pub fn new(name: impl Into<String>, query: impl Into<String>, color: Color) -> Self {
        Self {
            id: FilterId::new(),
            name: name.into(),
            query: query.into(),
            color,
            order: 0,
        }
    }
}

impl Label {
    pub fn new(name: impl Into<String>, color: Color) -> Self {
        Self {
            id: LabelId::new(),
            name: name.into(),
            color,
            is_favorite: false,
            order: 0,
        }
    }

    /// Whether this label answers to a name typed in quick-add.
    pub fn matches_name(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbox_sorts_above_every_other_project() {
        let inbox = Project::inbox();
        let other = Project::new("Work", Color::Blue);
        assert!(inbox.order < other.order);
        assert!(inbox.is_inbox());
        assert!(!other.is_inbox());
    }

    #[test]
    fn sections_are_appended_in_order() {
        let mut project = Project::new("Work", Color::Blue);
        project.add_section(Section::new("Todo"));
        project.add_section(Section::new("Doing"));
        let last = project.add_section(Section::new("Done"));

        let names: Vec<&str> = project
            .sections_ordered()
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        assert_eq!(names, vec!["Todo", "Doing", "Done"]);
        assert_eq!(project.section(&last).unwrap().name, "Done");
    }

    #[test]
    fn a_removed_section_comes_back_so_it_can_be_undone() {
        let mut project = Project::new("Work", Color::Blue);
        let id = project.add_section(Section::new("Doing"));

        let removed = project.remove_section(&id).expect("section was there");
        assert_eq!(removed.name, "Doing");
        assert!(project.section(&id).is_none());
        assert_eq!(project.remove_section(&id), None);
    }

    #[test]
    fn reusing_an_order_slot_after_a_removal_does_not_collide() {
        let mut project = Project::new("Work", Color::Blue);
        project.add_section(Section::new("A"));
        let b = project.add_section(Section::new("B"));
        project.remove_section(&b);
        project.add_section(Section::new("C"));

        let orders: Vec<i32> = project
            .sections
            .iter()
            .map(|section| section.order)
            .collect();
        let unique: std::collections::HashSet<_> = orders.iter().collect();
        assert_eq!(orders.len(), unique.len(), "orders must stay distinct");
    }

    #[test]
    fn label_names_match_regardless_of_case() {
        let label = Label::new("Work", Color::Blue);
        assert!(label.matches_name("work"));
        assert!(label.matches_name("WORK"));
        assert!(!label.matches_name("working"));
    }

    #[test]
    fn a_project_round_trips_through_json() {
        let mut project = Project::new("Work", Color::Teal);
        project.add_section(Section::new("Doing"));
        project.view_style = ViewStyle::Board;
        project.sort_by = SortBy::Priority;
        project.is_favorite = true;

        let json = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);
    }
}

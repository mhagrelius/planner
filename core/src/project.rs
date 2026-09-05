//! Projects, sections and labels.
//!
//! Sections were stored *inside* their project until schema v2, because a
//! section has no meaning apart from its project and nesting made the deletion
//! cascade true by construction rather than by remembering. Sync is what
//! changed the answer: nested, two machines each adding a section were two
//! edits to one project record, and merging them kept one and lost the other.
//! They are their own records now, and [`crate::store::Store::remove_project`]
//! carries the cascade the nesting used to.
//!
//! Labels are flat for a different reason and always were: their whole purpose
//! is to cut across the project tree.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::color::Color;
use super::id::{FilterId, LabelId, ProjectId, SectionId};
use super::order::Order;

/// When a record was last changed, for a file written before the field
/// existed.
///
/// The epoch rather than "now": a record nobody has touched since this build
/// must lose to any real edit on another machine, and dating it from the
/// moment it happened to be read would make the last machine to open the file
/// win an argument it was not part of.
pub(crate) fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(0, 0).expect("the epoch is a valid instant")
}

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
///
/// A record in its own right rather than a field of its project, which it was
/// until schema v2. Nested, a section added here and a section added on another
/// machine were two edits to the same project record, and merging them by
/// last-writer-wins kept one and silently dropped the other. Given an id and a
/// `project_id` it syncs like everything else, and two machines adding a
/// section each end up with both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: SectionId,
    pub project_id: ProjectId,
    pub name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
    #[serde(default)]
    pub order: Order,
    #[serde(default = "epoch")]
    pub updated_at: DateTime<Utc>,
}

impl Section {
    pub fn new(project_id: ProjectId, name: impl Into<String>) -> Self {
        Self {
            id: SectionId::new(),
            project_id,
            name: name.into(),
            collapsed: false,
            order: Order::start(),
            updated_at: epoch(),
        }
    }

    pub fn touch(&mut self, now: DateTime<Utc>) {
        self.updated_at = now;
    }
}

/// A section as schema v1 wrote it: inside its project, with no `project_id`
/// because its position in the file was the answer.
///
/// Read-only and never written. [`Store::open_at`](crate::store::Store::open_at)
/// lifts these out into the document's own list, after which the next save
/// leaves no trace of them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LegacySection {
    pub id: SectionId,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub order: Order,
}

impl LegacySection {
    /// The same section, told which project it was found in.
    pub fn lift(self, project_id: ProjectId) -> Section {
        Section {
            id: self.id,
            project_id,
            name: self.name,
            collapsed: self.collapsed,
            order: self.order,
            // A v1 file predates the field, so the section reads as untouched
            // since the epoch. Any real edit anywhere beats it, which is right:
            // nobody has changed it since this build learned to notice.
            updated_at: epoch(),
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

    /// Sections found nested inside this project by a schema v1 file, waiting
    /// to be lifted out. Always empty once the store has opened, and never
    /// written back.
    #[serde(default, rename = "sections", skip_serializing)]
    pub legacy_sections: Vec<LegacySection>,

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
    #[serde(default = "epoch")]
    pub updated_at: DateTime<Utc>,
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
            legacy_sections: Vec::new(),
            is_favorite: false,
            is_archived: false,
            collapsed: false,
            view_style: ViewStyle::default(),
            sort_by: SortBy::default(),
            show_completed: false,
            order: 0,
            updated_at: epoch(),
        }
    }

    pub fn touch(&mut self, now: DateTime<Utc>) {
        self.updated_at = now;
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
    #[serde(default = "epoch")]
    pub updated_at: DateTime<Utc>,
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
    #[serde(default = "epoch")]
    pub updated_at: DateTime<Utc>,
}

impl SavedFilter {
    pub fn new(name: impl Into<String>, query: impl Into<String>, color: Color) -> Self {
        Self {
            id: FilterId::new(),
            name: name.into(),
            query: query.into(),
            color,
            order: 0,
            updated_at: epoch(),
        }
    }

    pub fn touch(&mut self, now: DateTime<Utc>) {
        self.updated_at = now;
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
            updated_at: epoch(),
        }
    }

    pub fn touch(&mut self, now: DateTime<Utc>) {
        self.updated_at = now;
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
    fn label_names_match_regardless_of_case() {
        let label = Label::new("Work", Color::Blue);
        assert!(label.matches_name("work"));
        assert!(label.matches_name("WORK"));
        assert!(!label.matches_name("working"));
    }

    #[test]
    fn a_project_round_trips_through_json() {
        let mut project = Project::new("Work", Color::Teal);
        project.view_style = ViewStyle::Board;
        project.sort_by = SortBy::Priority;
        project.is_favorite = true;

        let json = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&json).unwrap(), project);
    }
}

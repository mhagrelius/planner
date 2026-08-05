//! The store: every project, label and task, and the file they live in.
//!
//! One JSON file, held entirely in memory. A personal task list is tens of
//! kilobytes and every view is a linear scan over a few thousand records,
//! which is microseconds — a database would buy nothing here and cost a
//! dependency, a schema, a migration story and an async story. The file stays
//! greppable, diffable and hand-editable, which is worth more.
//!
//! The trade is that `Store` owns *all* the records and hands out queries
//! rather than collections, so nothing outside can hold a reference across a
//! mutation. That is also what would let SQLite slot in behind this API if the
//! premise ever stopped holding.
//!
//! **Writes cannot lose the previous file.** A save goes to a temporary file,
//! is flushed and `fsync`ed, and only then renamed over the original. An
//! interrupted write leaves the old file untouched.
//!
//! **A file that will not parse does not stop the app.** It is moved aside and
//! the app starts empty, because a planner that refuses to launch is worse
//! than one that has lost yesterday's edits — and the original is still there
//! to be recovered by hand.
//!
//! **A file from a newer version is never overwritten.** Downgrading and
//! silently truncating fields the older code does not know about is the one
//! failure mode that destroys data with no way back.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::color::{self, Color};
use super::id::{FilterId, LabelId, ProjectId, SectionId, TaskId};
use super::order::Order;
use super::project::{Label, Project, SavedFilter, Section};
use super::task::{Completion, Task};

/// The schema version written into every file.
///
/// v2 replaced integer positions with ordering keys. A v1 file still opens —
/// [`crate::order::Order`] reads the old numbers and places them — and is
/// rewritten as v2 on the next save. A machine still running a v1 build then
/// finds a file it will not overwrite, which is the intended outcome: it
/// degrades to read-only rather than truncating keys it cannot represent.
pub const SCHEMA_VERSION: u32 = 2;

/// The on-disk document.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Document {
    version: u32,
    #[serde(default)]
    projects: Vec<Project>,
    /// Nested inside their projects until schema v2; see [`Section`].
    #[serde(default)]
    sections: Vec<Section>,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    tasks: Vec<Task>,
    #[serde(default)]
    filters: Vec<SavedFilter>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            projects: vec![Project::inbox()],
            sections: Vec::new(),
            labels: Vec::new(),
            tasks: Vec::new(),
            filters: Vec::new(),
        }
    }
}

/// What happened when the store was opened.
///
/// Returned alongside the store rather than logged, so the caller decides
/// whether a recovery deserves a toast, a dialog, or nothing at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// An existing file was read.
    Loaded,
    /// There was no file. This is a first run.
    Fresh,
    /// The file could not be read and was set aside.
    Recovered { backup: PathBuf, reason: String },
    /// The file is from a newer version of the app. It has been loaded as far
    /// as it can be understood, and the store will refuse to save over it.
    ReadOnly { version: u32 },
}

/// Why a save failed.
#[derive(Debug)]
pub enum SaveError {
    /// The loaded file is newer than this build understands.
    Newer {
        version: u32,
    },
    Io(std::io::Error),
    Serialise(serde_json::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Newer { version } => write!(
                f,
                "the planner file is from a newer version (v{version}, this build understands \
                 v{SCHEMA_VERSION}) and will not be overwritten"
            ),
            Self::Io(error) => write!(f, "could not write the planner file: {error}"),
            Self::Serialise(error) => write!(f, "could not encode the planner file: {error}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Newer { .. } => None,
            Self::Io(error) => Some(error),
            Self::Serialise(error) => Some(error),
        }
    }
}

/// Every record, and the file they came from.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    document: Document,
    /// Set when the file on disk is from a newer schema. Every save is
    /// refused for the lifetime of the store rather than just the first one.
    read_only: bool,
}

impl Store {
    /// Where the planner file lives.
    ///
    /// `$XDG_DATA_HOME/planner/planner.json`, falling back to
    /// `$HOME/.local/share` and finally the working directory, which only
    /// happens in an environment with neither variable set.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("planner").join("planner.json")
    }

    /// Open the store at the default path.
    pub fn open() -> (Self, LoadOutcome) {
        Self::open_at(Self::default_path())
    }

    /// Open the store at a given path. Never fails: an unreadable file is set
    /// aside and an empty store returned in its place.
    pub fn open_at(path: impl Into<PathBuf>) -> (Self, LoadOutcome) {
        let path = path.into();

        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Self::empty(path), LoadOutcome::Fresh);
            }
            Err(error) => {
                let outcome = quarantine(&path, &error.to_string());
                return (Self::empty(path), outcome);
            }
        };

        match serde_json::from_str::<Document>(&raw) {
            Ok(document) if document.version > SCHEMA_VERSION => {
                let version = document.version;
                let mut store = Self {
                    path,
                    document,
                    read_only: true,
                };
                store.ensure_inbox();
                store.lift_legacy_sections();
                (store, LoadOutcome::ReadOnly { version })
            }
            Ok(document) => {
                let mut store = Self {
                    path,
                    document,
                    read_only: false,
                };
                store.ensure_inbox();
                store.lift_legacy_sections();
                (store, LoadOutcome::Loaded)
            }
            Err(error) => {
                let outcome = quarantine(&path, &error.to_string());
                (Self::empty(path), outcome)
            }
        }
    }

    fn empty(path: PathBuf) -> Self {
        Self {
            path,
            document: Document::default(),
            read_only: false,
        }
    }

    /// An empty store with nowhere to save to.
    ///
    /// For the moment between an object being constructed and its real store
    /// being read. It refuses to save rather than writing somewhere arbitrary,
    /// so a code path that forgets to open a real one fails loudly in testing
    /// instead of scattering files.
    pub fn detached() -> Self {
        Self {
            path: PathBuf::new(),
            document: Document::default(),
            read_only: true,
        }
    }

    /// Put the Inbox back if the file did not have one.
    ///
    /// A hand-edited or partially-written file that has lost its Inbox would
    /// otherwise strand every task that points at it.
    fn ensure_inbox(&mut self) {
        if !self.document.projects.iter().any(|p| p.is_inbox()) {
            self.document.projects.insert(0, Project::inbox());
        }
    }

    /// Lift schema v1's nested sections out into the document's own list.
    ///
    /// Runs on every open rather than only when the version says v1: a file
    /// that has been merged, restored from a backup, or hand-edited can hold
    /// both shapes, and a section left nested is a board column that silently
    /// stops existing. Doing it unconditionally costs one pass over the
    /// projects and cannot be forgotten.
    fn lift_legacy_sections(&mut self) {
        for project in self.document.projects.iter_mut() {
            let project_id = project.id.clone();
            for legacy in project.legacy_sections.drain(..) {
                // A section already lifted by an earlier open wins, so reading
                // a file twice does not double its columns.
                if self.document.sections.iter().any(|s| s.id == legacy.id) {
                    continue;
                }
                self.document.sections.push(legacy.lift(project_id.clone()));
            }
        }
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether saving is refused because the file is from a newer version.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Write the store out.
    ///
    /// Atomic: a temporary file is written, flushed and `fsync`ed, then
    /// renamed over the target. The rename is the only step that can be seen
    /// from outside, and it either happened or it did not.
    pub fn save(&self) -> Result<(), SaveError> {
        if self.read_only {
            return Err(SaveError::Newer {
                version: self.document.version,
            });
        }

        let encoded = serde_json::to_vec_pretty(&self.document).map_err(SaveError::Serialise)?;

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(SaveError::Io)?;
        }

        let temporary = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&temporary).map_err(SaveError::Io)?;
        file.write_all(&encoded).map_err(SaveError::Io)?;
        file.flush().map_err(SaveError::Io)?;
        file.sync_all().map_err(SaveError::Io)?;
        drop(file);

        fs::rename(&temporary, &self.path).map_err(SaveError::Io)
    }

    // --- projects -------------------------------------------------------

    pub fn projects(&self) -> &[Project] {
        &self.document.projects
    }

    pub fn project(&self, id: &ProjectId) -> Option<&Project> {
        self.document.projects.iter().find(|p| &p.id == id)
    }

    pub fn project_mut(&mut self, id: &ProjectId) -> Option<&mut Project> {
        self.document.projects.iter_mut().find(|p| &p.id == id)
    }

    /// Projects in sidebar order: the Inbox first, then by `order`.
    pub fn projects_ordered(&self) -> Vec<&Project> {
        let mut projects: Vec<&Project> = self
            .document
            .projects
            .iter()
            .filter(|p| !p.is_archived)
            .collect();
        projects.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
        projects
    }

    /// The direct children of a project, in order.
    pub fn subprojects(&self, parent: &ProjectId) -> Vec<&Project> {
        self.projects_ordered()
            .into_iter()
            .filter(|p| p.parent_id.as_ref() == Some(parent))
            .collect()
    }

    /// A project and everything nested beneath it, including itself.
    ///
    /// Guards against a parent cycle, which a hand-edited file can contain —
    /// without it, `##Project` on a cyclic tree would hang the app.
    pub fn project_and_descendants(&self, root: &ProjectId) -> Vec<ProjectId> {
        let mut found = vec![root.clone()];
        let mut index = 0;
        while index < found.len() {
            let current = found[index].clone();
            for child in self.subprojects(&current) {
                if !found.contains(&child.id) {
                    found.push(child.id.clone());
                }
            }
            index += 1;
        }
        found
    }

    /// Add a project, giving it the next free colour if it has none of its own.
    pub fn add_project(&mut self, mut project: Project) -> ProjectId {
        project.order = self
            .document
            .projects
            .iter()
            .map(|existing| existing.order)
            .max()
            .map_or(0, |max| max + 1);
        let id = project.id.clone();
        self.document.projects.push(project);
        id
    }

    /// The colour a new project should take.
    pub fn next_project_color(&self) -> Color {
        let used: Vec<Color> = self.document.projects.iter().map(|p| p.color).collect();
        color::least_used(&used)
    }

    /// Delete a project, its subprojects, and every task in any of them.
    ///
    /// Returns what was removed so the caller can offer an undo. The Inbox
    /// cannot be deleted; asking to is a no-op rather than an error.
    pub fn remove_project(&mut self, id: &ProjectId) -> Option<RemovedProject> {
        if id.is_inbox() || self.project(id).is_none() {
            return None;
        }

        let doomed = self.project_and_descendants(id);
        let projects: Vec<Project> =
            extract(&mut self.document.projects, |p| doomed.contains(&p.id));
        // Sections are their own records now, so deleting a project has to take
        // them explicitly. Left behind they would belong to nothing, and would
        // come back the moment the project did.
        let sections: Vec<Section> = extract(&mut self.document.sections, |s| {
            doomed.contains(&s.project_id)
        });
        let tasks: Vec<Task> =
            extract(&mut self.document.tasks, |t| doomed.contains(&t.project_id));

        Some(RemovedProject {
            projects,
            sections,
            tasks,
        })
    }

    // --- sections -------------------------------------------------------

    /// Find a section anywhere.
    pub fn section(&self, id: &SectionId) -> Option<&Section> {
        self.document.sections.iter().find(|s| &s.id == id)
    }

    pub fn section_mut(&mut self, id: &SectionId) -> Option<&mut Section> {
        self.document.sections.iter_mut().find(|s| &s.id == id)
    }

    /// A project's sections, in display order.
    pub fn sections_in(&self, project: &ProjectId) -> Vec<&Section> {
        let mut sections: Vec<&Section> = self
            .document
            .sections
            .iter()
            .filter(|section| &section.project_id == project)
            .collect();
        sections.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        sections
    }

    /// Add a section at the end of its project's list.
    pub fn add_section(&mut self, mut section: Section) -> SectionId {
        let last = self
            .sections_in(&section.project_id)
            .last()
            .map(|existing| existing.order.clone());
        section.order = Order::between(last.as_ref(), None);
        let id = section.id.clone();
        self.document.sections.push(section);
        id
    }

    /// Remove a section, moving its tasks back to the project's own list
    /// rather than deleting them with it. Losing tasks to a tidy-up of columns
    /// is not what anyone means by "delete section".
    pub fn remove_section(&mut self, id: &SectionId, now: DateTime<Utc>) -> Option<RemovedSection> {
        let index = self.document.sections.iter().position(|s| &s.id == id)?;
        let section = self.document.sections.remove(index);
        let project_id = section.project_id.clone();

        let mut tasks = Vec::new();
        for task in self.document.tasks.iter_mut() {
            if task.section_id.as_ref() == Some(id) {
                task.section_id = None;
                task.touch(now);
                tasks.push(task.id.clone());
            }
        }
        Some(RemovedSection {
            project: project_id,
            section,
            tasks,
        })
    }

    /// Put a removed section back, with the tasks that were in it.
    ///
    /// The tasks are named rather than carried: removing a section leaves them
    /// in the project, where they can be edited or completed before undo is
    /// pressed. Restoring a copy taken at deletion would undo that too.
    pub fn restore_section(&mut self, removed: RemovedSection, now: DateTime<Utc>) {
        let id = removed.section.id.clone();
        if self.project(&removed.project).is_none() {
            return;
        }
        // Keeps its own key rather than being appended, so an undone deletion
        // lands back where it was taken from.
        self.document.sections.push(removed.section);

        for task in self.document.tasks.iter_mut() {
            // A task moved to another project in the meantime keeps its new
            // home: a section only holds tasks of its own project.
            if task.project_id == removed.project && removed.tasks.contains(&task.id) {
                task.section_id = Some(id.clone());
                task.touch(now);
            }
        }
    }

    // --- labels ---------------------------------------------------------

    pub fn labels(&self) -> &[Label] {
        &self.document.labels
    }

    pub fn label(&self, id: &LabelId) -> Option<&Label> {
        self.document.labels.iter().find(|l| &l.id == id)
    }

    pub fn label_mut(&mut self, id: &LabelId) -> Option<&mut Label> {
        self.document.labels.iter_mut().find(|l| &l.id == id)
    }

    /// Look a label up by the name typed in quick-add or a filter query.
    pub fn label_by_name(&self, name: &str) -> Option<&Label> {
        self.document.labels.iter().find(|l| l.matches_name(name))
    }

    /// Look a project up by name, for `#project` in quick-add and queries.
    pub fn project_by_name(&self, name: &str) -> Option<&Project> {
        self.document
            .projects
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Get the label with this name, creating it if it is new.
    ///
    /// Quick-add types labels rather than picking them, so `@errand` has to
    /// mean "the errand label" whether or not it existed a moment ago.
    pub fn label_for_name(&mut self, name: &str) -> LabelId {
        if let Some(existing) = self.label_by_name(name) {
            return existing.id.clone();
        }
        let used: Vec<Color> = self.document.labels.iter().map(|l| l.color).collect();
        let mut label = Label::new(name, color::least_used(&used));
        label.order = self.document.labels.len() as i32;
        let id = label.id.clone();
        self.document.labels.push(label);
        id
    }

    /// Delete a label and take it off every task carrying it.
    pub fn remove_label(&mut self, id: &LabelId, now: DateTime<Utc>) -> Option<Label> {
        let index = self.document.labels.iter().position(|l| &l.id == id)?;
        let removed = self.document.labels.remove(index);

        for task in self.document.tasks.iter_mut() {
            if task.has_label(id) {
                task.remove_label(id);
                task.touch(now);
            }
        }
        Some(removed)
    }

    // --- saved filters --------------------------------------------------

    pub fn filters(&self) -> &[SavedFilter] {
        &self.document.filters
    }

    pub fn filter(&self, id: &FilterId) -> Option<&SavedFilter> {
        self.document.filters.iter().find(|filter| &filter.id == id)
    }

    /// Filters in sidebar order.
    pub fn filters_ordered(&self) -> Vec<&SavedFilter> {
        let mut filters: Vec<&SavedFilter> = self.document.filters.iter().collect();
        filters.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
        filters
    }

    /// Save a filter, or replace one with the same ID.
    pub fn put_filter(&mut self, filter: SavedFilter) -> FilterId {
        let id = filter.id.clone();
        match self
            .document
            .filters
            .iter_mut()
            .find(|existing| existing.id == id)
        {
            Some(existing) => *existing = filter,
            None => {
                let mut filter = filter;
                filter.order = self
                    .document
                    .filters
                    .iter()
                    .map(|existing| existing.order)
                    .max()
                    .map_or(0, |max| max + 1);
                self.document.filters.push(filter);
            }
        }
        id
    }

    /// Delete a filter, returning it so it can be undone.
    pub fn remove_filter(&mut self, id: &FilterId) -> Option<SavedFilter> {
        let index = self
            .document
            .filters
            .iter()
            .position(|filter| &filter.id == id)?;
        Some(self.document.filters.remove(index))
    }

    /// The colour a new filter should take.
    pub fn next_filter_color(&self) -> Color {
        let used: Vec<Color> = self.document.filters.iter().map(|f| f.color).collect();
        color::least_used(&used)
    }

    // --- tasks ----------------------------------------------------------

    pub fn tasks(&self) -> &[Task] {
        &self.document.tasks
    }

    pub fn task(&self, id: &TaskId) -> Option<&Task> {
        self.document.tasks.iter().find(|t| &t.id == id)
    }

    pub fn task_mut(&mut self, id: &TaskId) -> Option<&mut Task> {
        self.document.tasks.iter_mut().find(|t| &t.id == id)
    }

    /// Add a task, placing it at the end of its project or section.
    pub fn add_task(&mut self, mut task: Task) -> TaskId {
        let last = self
            .document
            .tasks
            .iter()
            .filter(|existing| {
                existing.project_id == task.project_id && existing.section_id == task.section_id
            })
            .map(|existing| &existing.order)
            .max()
            .cloned();
        task.order = Order::between(last.as_ref(), None);
        let id = task.id.clone();
        self.document.tasks.push(task);
        id
    }

    /// The names a quick-add parse needs to recognise multi-word tokens.
    pub fn vocabulary(&self) -> super::parse::Vocabulary {
        super::parse::Vocabulary {
            projects: self
                .document
                .projects
                .iter()
                .map(|project| project.name.clone())
                .collect(),
            sections: self
                .document
                .sections
                .iter()
                .map(|section| section.name.clone())
                .collect(),
            labels: self
                .document
                .labels
                .iter()
                .map(|label| label.name.clone())
                .collect(),
        }
    }

    /// Turn a parsed quick-add line into a task and file it.
    ///
    /// Names become IDs here because this is the only place that can do it:
    /// the parser sees text and has no store, and the UI should not be
    /// inventing labels. A `@label` that does not exist yet is created —
    /// quick-add types labels rather than picking them, so `@errand` has to
    /// mean "the errand label" whether or not it existed a moment ago. A
    /// `#project` that does not exist is *not*: creating a project by
    /// mistyping one is a much worse outcome than the task landing in the
    /// default, where it is visible and easy to move.
    ///
    /// `default_project` and `default_section` are where the task goes when
    /// the line does not say — normally whatever the sidebar is showing.
    pub fn add_from_quick_add(
        &mut self,
        parsed: &super::parse::QuickAdd,
        default_project: &ProjectId,
        default_section: Option<&SectionId>,
        now: DateTime<Utc>,
    ) -> TaskId {
        let project_id = parsed
            .project
            .as_deref()
            .and_then(|name| self.project_by_name(name))
            .map(|project| project.id.clone())
            .unwrap_or_else(|| {
                // A default pointing at a project that has since been deleted
                // must not strand the task somewhere invisible.
                if self.project(default_project).is_some() {
                    default_project.clone()
                } else {
                    ProjectId::inbox()
                }
            });

        let section_id = match parsed.section.as_deref() {
            Some(name) => self
                .sections_in(&project_id)
                .into_iter()
                .find(|section| section.name.eq_ignore_ascii_case(name))
                .map(|section| section.id.clone()),
            // A default section only applies in its own project. Carrying it
            // across to a `#project` named on the line would file the task
            // under a section that project does not have.
            None => default_section
                .filter(|_| parsed.project.is_none())
                .filter(|id| {
                    self.section(id)
                        .is_some_and(|section| section.project_id == project_id)
                })
                .cloned(),
        };

        let labels: Vec<LabelId> = parsed
            .labels
            .iter()
            .map(|name| self.label_for_name(name))
            .collect();

        let mut task = Task::new(project_id, parsed.title.trim(), now);
        task.section_id = section_id;
        task.labels = labels;
        task.due = parsed.due.clone();
        if let Some(priority) = parsed.priority {
            task.priority = priority;
        }
        task.reminders = parsed
            .reminders
            .iter()
            .map(|minutes| super::task::Reminder::before_due(*minutes))
            .collect();

        self.add_task(task)
    }

    // --- ordering and moving --------------------------------------------

    /// The top-level tasks of one project or section, in display order.
    ///
    /// Subtasks are excluded: they belong under their parent, not loose in
    /// the list, and a board column showing both would count each twice.
    pub fn tasks_in(&self, project: &ProjectId, section: Option<&SectionId>) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self
            .document
            .tasks
            .iter()
            .filter(|task| {
                task.parent_id.is_none()
                    && &task.project_id == project
                    && task.section_id.as_ref() == section
            })
            .collect();
        // Two machines dropping a task into the same gap can mint the same
        // key. Falling through to the id keeps the list in the same order on
        // both of them, which matters more than which one comes first.
        tasks.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.added_at.cmp(&b.added_at))
                .then_with(|| a.id.cmp(&b.id))
        });
        tasks
    }

    /// Move a task to a project and section, at a position within that list.
    ///
    /// `index` counts positions in the destination list *after* the task has
    /// been taken out of wherever it was, so dragging a task down its own list
    /// means what it looks like it means rather than landing one short.
    ///
    /// Returns `false` if the destination does not exist: an unknown task, an
    /// unknown project, or a section belonging to a different project. That
    /// last one is the reason this validates at all — a drop handler working
    /// from stale widget state can otherwise file a task under a section its
    /// project does not have, where no view will ever show it again.
    pub fn move_task(
        &mut self,
        id: &TaskId,
        project: &ProjectId,
        section: Option<&SectionId>,
        index: usize,
        now: DateTime<Utc>,
    ) -> bool {
        if self.task(id).is_none() {
            return false;
        }
        if self.project(project).is_none() {
            return false;
        }
        if let Some(section) = section {
            let belongs_to_project = self
                .section(section)
                .is_some_and(|section| section.project_id == *project);
            if !belongs_to_project {
                return false;
            }
        }

        // A dragged task takes its subtasks with it: a child left behind in
        // another project is invisible under its parent and counted in a
        // project it is not shown in.
        let family = self.task_and_descendants(id);

        // The neighbours the task is being dropped between, in the destination
        // list as it stands without it.
        let neighbours: Vec<Order> = self
            .tasks_in(project, section)
            .into_iter()
            .filter(|other| other.id != *id)
            .map(|task| task.order.clone())
            .collect();

        let index = index.min(neighbours.len());
        let before = index.checked_sub(1).and_then(|i| neighbours.get(i));
        let after = neighbours.get(index);
        let landed = Order::between(before, after);

        for member in &family {
            if let Some(task) = self.task_mut(member) {
                task.project_id = project.clone();
                // Only the dragged task moves section; its subtasks are not
                // in a section of their own to begin with.
                if member == id {
                    task.section_id = section.cloned();
                    task.order = landed.clone();
                }
                task.touch(now);
            }
        }

        // Nothing to do about the list it came from. A key says where a task
        // sits relative to its neighbours, not how many there are, so removing
        // one leaves the rest correct — which is the point: one drag is one
        // changed record, and a machine that was switched off receives one.
        true
    }

    /// Rename a section.
    pub fn rename_section(&mut self, id: &SectionId, name: &str) -> bool {
        match self.section_mut(id) {
            Some(section) => {
                section.name = name.to_string();
                true
            }
            None => false,
        }
    }

    /// Move a section to a position within its project.
    ///
    /// One record changes, for the same reason a moved task changes one — see
    /// [`crate::order`].
    pub fn move_section(&mut self, id: &SectionId, index: usize) -> bool {
        let Some(project_id) = self.section(id).map(|section| section.project_id.clone()) else {
            return false;
        };

        let neighbours: Vec<Order> = self
            .sections_in(&project_id)
            .into_iter()
            .filter(|other| other.id != *id)
            .map(|section| section.order.clone())
            .collect();

        let index = index.min(neighbours.len());
        let before = index.checked_sub(1).and_then(|i| neighbours.get(i));
        let landed = Order::between(before, neighbours.get(index));

        match self.section_mut(id) {
            Some(section) => {
                section.order = landed;
                true
            }
            None => false,
        }
    }

    /// The direct subtasks of a task, in order.
    pub fn subtasks(&self, parent: &TaskId) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self
            .document
            .tasks
            .iter()
            .filter(|t| t.parent_id.as_ref() == Some(parent))
            .collect();
        tasks.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        tasks
    }

    /// A task and every subtask beneath it, including itself.
    pub fn task_and_descendants(&self, root: &TaskId) -> Vec<TaskId> {
        let mut found = vec![root.clone()];
        let mut index = 0;
        while index < found.len() {
            let current = found[index].clone();
            for child in self.subtasks(&current) {
                if !found.contains(&child.id) {
                    found.push(child.id.clone());
                }
            }
            index += 1;
        }
        found
    }

    /// Delete a task and its subtasks, returning them so it can be undone.
    ///
    /// Deleting a parent must take its children: leaving them behind would
    /// orphan them into a project view they have no row in, where they are
    /// invisible but still count towards every total.
    pub fn remove_task(&mut self, id: &TaskId) -> Vec<Task> {
        let doomed = self.task_and_descendants(id);
        extract(&mut self.document.tasks, |t| doomed.contains(&t.id))
    }

    /// Put previously removed tasks back, for undo.
    pub fn restore_tasks(&mut self, tasks: Vec<Task>) {
        self.document.tasks.extend(tasks);
    }

    /// Put a previously removed project, its subprojects and its tasks back.
    pub fn restore_project(&mut self, removed: RemovedProject) {
        self.document.projects.extend(removed.projects);
        self.document.sections.extend(removed.sections);
        self.document.tasks.extend(removed.tasks);
    }

    /// Tick a task off.
    ///
    /// Completing a parent completes its open subtasks with it — a task with
    /// unfinished children showing as done is a lie, and making the user tick
    /// six things to close one is worse.
    pub fn complete_task(
        &mut self,
        id: &TaskId,
        now: DateTime<Utc>,
        today: NaiveDate,
    ) -> Option<Completion> {
        let affected = self.task_and_descendants(id);
        let outcome = self.task_mut(id)?.complete(now, today);

        if matches!(outcome, Completion::Done) {
            for child in affected.into_iter().skip(1) {
                if let Some(task) = self.task_mut(&child) {
                    task.complete(now, today);
                }
            }
        }
        Some(outcome)
    }

    /// Reopen a task, and its parents with it.
    ///
    /// A subtask cannot be open inside a completed parent, so reopening one
    /// reopens the chain above it.
    pub fn uncomplete_task(&mut self, id: &TaskId, now: DateTime<Utc>) {
        let mut current = Some(id.clone());
        while let Some(task_id) = current {
            let Some(task) = self.task_mut(&task_id) else {
                break;
            };
            task.uncomplete(now);
            current = task.parent_id.clone();
        }
    }

    /// How many of a project's tasks are done, out of how many there are.
    ///
    /// Counts every task in the project including subtasks, which is what
    /// makes the progress ring move when you tick off a step rather than
    /// only when you finish a whole item.
    pub fn progress(&self, project: &ProjectId) -> (usize, usize) {
        let tasks = self
            .document
            .tasks
            .iter()
            .filter(|t| &t.project_id == project);
        let mut total = 0;
        let mut done = 0;
        for task in tasks {
            total += 1;
            if task.checked {
                done += 1;
            }
        }
        (done, total)
    }

    /// Every label ID currently in use, with how many open tasks carry it.
    pub fn label_counts(&self) -> HashMap<LabelId, usize> {
        let mut counts = HashMap::new();
        for task in self.document.tasks.iter().filter(|t| !t.checked) {
            for label in &task.labels {
                *counts.entry(label.clone()).or_insert(0) += 1;
            }
        }
        counts
    }
}

/// A deleted project, its sections and its tasks, kept whole so it can be put
/// back.
#[derive(Debug)]
pub struct RemovedProject {
    pub projects: Vec<Project>,
    pub sections: Vec<Section>,
    pub tasks: Vec<Task>,
}

/// A deleted section, with the tasks it held, so it can be put back.
#[derive(Debug)]
pub struct RemovedSection {
    pub project: ProjectId,
    pub section: Section,
    pub tasks: Vec<TaskId>,
}

/// Remove everything matching `doomed` from `items` and return it.
///
/// `Vec::retain` throws away what it drops; undo needs it back.
fn extract<T>(items: &mut Vec<T>, doomed: impl Fn(&T) -> bool) -> Vec<T> {
    let mut taken = Vec::new();
    let mut index = 0;
    while index < items.len() {
        if doomed(&items[index]) {
            taken.push(items.remove(index));
        } else {
            index += 1;
        }
    }
    taken
}

/// Move an unreadable file aside so the app can start.
///
/// The timestamp in the name means repeated failures cannot overwrite the
/// first, most useful, copy.
fn quarantine(path: &Path, reason: &str) -> LoadOutcome {
    let stamp = Utc::now().timestamp();
    let backup = path.with_extension(format!("json.corrupt-{stamp}"));
    let _ = fs::rename(path, &backup);
    LoadOutcome::Recovered {
        backup,
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::due::Due;
    use crate::recurrence::{Recurrence, Unit};
    use tempfile::TempDir;

    fn instant(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let (store, outcome) = Store::open_at(dir.path().join("planner.json"));
        assert_eq!(outcome, LoadOutcome::Fresh);
        (dir, store)
    }

    fn task(store: &mut Store, content: &str) -> TaskId {
        store.add_task(Task::new(ProjectId::inbox(), content, instant(2026, 7, 30)))
    }

    #[test]
    fn a_fresh_store_has_an_inbox_and_nothing_else() {
        let (_dir, store) = store();
        assert_eq!(store.projects().len(), 1);
        assert!(store.projects()[0].is_inbox());
        assert!(store.tasks().is_empty());
    }

    #[test]
    fn a_store_survives_a_round_trip_through_the_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("planner.json");

        let (mut store, _) = Store::open_at(&path);
        let id = task(&mut store, "Water the plants");
        store.task_mut(&id).unwrap().due = Some(Due::on(date(2026, 8, 1)));
        store.save().unwrap();

        let (reopened, outcome) = Store::open_at(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(reopened.task(&id).unwrap().content, "Water the plants");
        assert_eq!(
            reopened.task(&id).unwrap().due.as_ref().unwrap().date,
            date(2026, 8, 1)
        );
    }

    #[test]
    fn an_interrupted_write_leaves_no_temporary_file_behind() {
        let (dir, mut store) = store();
        task(&mut store, "Anything");
        store.save().unwrap();

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "the temporary file must be renamed");
    }

    #[test]
    fn a_corrupt_file_is_set_aside_and_the_app_still_starts() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("planner.json");
        fs::write(&path, "{ this is not json").unwrap();

        let (store, outcome) = Store::open_at(&path);

        let LoadOutcome::Recovered { backup, .. } = outcome else {
            panic!("expected the file to be quarantined, got {outcome:?}");
        };
        assert!(backup.exists(), "the original must be kept");
        assert!(!path.exists(), "the unreadable file must be moved aside");
        assert!(store.tasks().is_empty());
        // And the app is usable: the fresh store saves over the empty slot.
        store.save().unwrap();
    }

    #[test]
    fn a_file_from_a_newer_version_is_never_overwritten() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("planner.json");
        let future = format!(
            r#"{{"version":{},"projects":[],"labels":[],"tasks":[]}}"#,
            SCHEMA_VERSION + 1
        );
        fs::write(&path, &future).unwrap();

        let (store, outcome) = Store::open_at(&path);

        assert_eq!(
            outcome,
            LoadOutcome::ReadOnly {
                version: SCHEMA_VERSION + 1
            }
        );
        assert!(store.is_read_only());
        assert!(matches!(store.save(), Err(SaveError::Newer { .. })));
        assert_eq!(fs::read_to_string(&path).unwrap(), future);
    }

    /// The upgrade that has to work on a file somebody is actually using.
    #[test]
    fn a_v1_file_opens_with_its_hand_sorted_list_still_in_order() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("planner.json");

        // Exactly what v1 wrote: positions as numbers, deliberately stored out
        // of order in the array so that only `order` can be putting them right.
        fs::write(
            &path,
            r#"{"version":1,"projects":[],"labels":[],"tasks":[
                {"id":"c","content":"Third","project_id":"inbox","added_at":"2026-07-01T12:00:00Z","updated_at":"2026-07-01T12:00:00Z","order":2},
                {"id":"a","content":"First","project_id":"inbox","added_at":"2026-07-01T12:00:00Z","updated_at":"2026-07-01T12:00:00Z","order":0},
                {"id":"b","content":"Second","project_id":"inbox","added_at":"2026-07-01T12:00:00Z","updated_at":"2026-07-01T12:00:00Z","order":1}
            ]}"#,
        )
        .unwrap();

        let (mut store, outcome) = Store::open_at(&path);

        assert_eq!(outcome, LoadOutcome::Loaded, "a v1 file is not corrupt");
        assert_eq!(
            order_of(&store, &ProjectId::inbox(), None),
            vec!["First", "Second", "Third"]
        );

        // And a task added afterwards goes to the end rather than into the
        // middle of the upgraded list.
        let added = store.add_task(Task::new(ProjectId::inbox(), "Fourth", instant(2026, 8, 1)));
        assert_eq!(
            order_of(&store, &ProjectId::inbox(), None),
            vec!["First", "Second", "Third", "Fourth"]
        );

        // Saving writes v2, and reopening finds the same list.
        store.save().expect("save");
        let (reopened, outcome) = Store::open_at(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(
            order_of(&reopened, &ProjectId::inbox(), None),
            vec!["First", "Second", "Third", "Fourth"]
        );
        assert!(reopened.task(&added).is_some());
    }

    /// v1 kept sections inside their project. They have to come out, or every
    /// board column in an existing file silently stops existing.
    #[test]
    fn a_v1_file_keeps_its_sections_and_the_tasks_filed_under_them() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("planner.json");

        fs::write(
            &path,
            r#"{"version":1,"labels":[],"projects":[
                {"id":"work","name":"Work","sections":[
                    {"id":"s2","name":"Doing","order":1},
                    {"id":"s1","name":"Todo","order":0}
                ]}
            ],"tasks":[
                {"id":"t1","content":"Ship it","project_id":"work","section_id":"s2","added_at":"2026-07-01T12:00:00Z","updated_at":"2026-07-01T12:00:00Z","order":0}
            ]}"#,
        )
        .unwrap();

        let (store, outcome) = Store::open_at(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);

        let work = ProjectId::from_raw("work");
        let names: Vec<&str> = store
            .sections_in(&work)
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        assert_eq!(names, vec!["Todo", "Doing"], "in their v1 order");

        let doing = SectionId::from_raw("s2");
        assert_eq!(store.section(&doing).unwrap().project_id, work);
        assert_eq!(
            store
                .tasks_in(&work, Some(&doing))
                .iter()
                .map(|task| task.content.as_str())
                .collect::<Vec<_>>(),
            vec!["Ship it"]
        );

        // Saving writes them at the top level and leaves nothing nested, so a
        // second open does not find both shapes and double them.
        store.save().expect("save");
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            !written.contains(r#""sections":[{"id":"s2""#),
            "sections must not be written back inside their project"
        );

        let (reopened, _) = Store::open_at(&path);
        assert_eq!(reopened.sections_in(&work).len(), 2, "not doubled");
    }

    #[test]
    fn a_file_that_has_lost_its_inbox_gets_one_back() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("planner.json");
        fs::write(
            &path,
            r#"{"version":1,"projects":[],"labels":[],"tasks":[]}"#,
        )
        .unwrap();

        let (store, outcome) = Store::open_at(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert!(store.project(&ProjectId::inbox()).is_some());
    }

    #[test]
    fn deleting_a_project_takes_its_subprojects_and_their_tasks() {
        let (_dir, mut store) = store();
        let parent = store.add_project(Project::new("Work", Color::Blue));
        let mut child = Project::new("Admin", Color::Teal);
        child.parent_id = Some(parent.clone());
        let child = store.add_project(child);

        let mut in_child = Task::new(child.clone(), "File the thing", instant(2026, 7, 30));
        in_child.project_id = child.clone();
        store.add_task(in_child);
        let doomed_section = store.add_section(Section::new(child.clone(), "Doing"));
        let survivor = task(&mut store, "Untouched inbox task");

        let removed = store.remove_project(&parent).expect("project was there");

        assert_eq!(removed.projects.len(), 2);
        assert_eq!(removed.tasks.len(), 1);
        // Sections are their own records now, so the cascade has to name them.
        // Left behind, one would belong to nothing and reappear the moment its
        // project was restored by an undo.
        assert_eq!(removed.sections.len(), 1);
        assert!(store.section(&doomed_section).is_none());
        assert!(store.project(&child).is_none());
        assert!(store.task(&survivor).is_some());

        store.restore_project(removed);
        assert!(
            store.section(&doomed_section).is_some(),
            "undo brings it back"
        );
    }

    #[test]
    fn the_inbox_cannot_be_deleted() {
        let (_dir, mut store) = store();
        assert!(store.remove_project(&ProjectId::inbox()).is_none());
        assert!(store.project(&ProjectId::inbox()).is_some());
    }

    #[test]
    fn a_deleted_project_can_be_put_back_whole() {
        let (_dir, mut store) = store();
        let id = store.add_project(Project::new("Work", Color::Blue));
        let mut task = Task::new(id.clone(), "Something", instant(2026, 7, 30));
        task.project_id = id.clone();
        let task_id = store.add_task(task);

        let removed = store.remove_project(&id).unwrap();
        store.restore_project(removed);

        assert!(store.project(&id).is_some());
        assert!(store.task(&task_id).is_some());
    }

    #[test]
    fn a_cyclic_project_tree_does_not_hang_the_walk() {
        let (_dir, mut store) = store();
        let a = store.add_project(Project::new("A", Color::Blue));
        let b = store.add_project(Project::new("B", Color::Teal));
        store.project_mut(&a).unwrap().parent_id = Some(b.clone());
        store.project_mut(&b).unwrap().parent_id = Some(a.clone());

        let walked = store.project_and_descendants(&a);
        assert_eq!(walked.len(), 2);
    }

    #[test]
    fn deleting_a_section_keeps_its_tasks() {
        let (_dir, mut store) = store();
        let project_id = store.add_project(Project::new("Work", Color::Blue));
        let section = store.add_section(Section::new(project_id.clone(), "Doing"));

        let mut task = Task::new(project_id.clone(), "In a column", instant(2026, 7, 30));
        task.section_id = Some(section.clone());
        let task_id = store.add_task(task);

        store.remove_section(&section, instant(2026, 7, 31));

        let task = store.task(&task_id).expect("the task must survive");
        assert_eq!(task.section_id, None);
        assert_eq!(task.updated_at, instant(2026, 7, 31));
    }

    #[test]
    fn undoing_a_section_deletion_puts_its_tasks_back_in_it() {
        let (_dir, mut store) = store();
        let project_id = store.add_project(Project::new("Work", Color::Blue));
        let first = store.add_section(Section::new(project_id.clone(), "Doing"));
        let second = store.add_section(Section::new(project_id.clone(), "Done"));

        let mut task = Task::new(project_id.clone(), "In a column", instant(2026, 7, 30));
        task.section_id = Some(first.clone());
        let task_id = store.add_task(task);

        let removed = store
            .remove_section(&first, instant(2026, 7, 31))
            .expect("section was there");
        store.restore_section(removed, instant(2026, 8, 1));

        assert_eq!(
            store.task(&task_id).unwrap().section_id,
            Some(first.clone())
        );
        let order: Vec<&str> = store
            .sections_in(&project_id)
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        assert_eq!(
            order,
            vec!["Doing", "Done"],
            "an undone deletion goes back where it was, not on the end"
        );
        assert_eq!(second, store.sections_in(&project_id)[1].id);
    }

    #[test]
    fn a_task_that_moved_project_is_not_dragged_back_by_an_undo() {
        let (_dir, mut store) = store();
        let work = store.add_project(Project::new("Work", Color::Blue));
        let home = store.add_project(Project::new("Home", Color::Teal));
        let section = store.add_section(Section::new(work.clone(), "Doing"));

        let mut task = Task::new(work.clone(), "In a column", instant(2026, 7, 30));
        task.section_id = Some(section.clone());
        let task_id = store.add_task(task);

        let removed = store
            .remove_section(&section, instant(2026, 7, 31))
            .expect("section was there");
        store.task_mut(&task_id).unwrap().project_id = home.clone();
        store.restore_section(removed, instant(2026, 8, 1));

        let task = store.task(&task_id).unwrap();
        assert_eq!(task.project_id, home);
        assert_eq!(task.section_id, None);
    }

    #[test]
    fn deleting_a_label_takes_it_off_every_task() {
        let (_dir, mut store) = store();
        let label = store.label_for_name("errand");
        let a = task(&mut store, "One");
        let b = task(&mut store, "Two");
        store.task_mut(&a).unwrap().add_label(label.clone());
        store.task_mut(&b).unwrap().add_label(label.clone());

        store.remove_label(&label, instant(2026, 7, 31));

        assert!(store.label(&label).is_none());
        assert!(store.task(&a).unwrap().labels.is_empty());
        assert!(store.task(&b).unwrap().labels.is_empty());
    }

    #[test]
    fn a_label_typed_twice_is_the_same_label() {
        let (_dir, mut store) = store();
        let first = store.label_for_name("Errand");
        let second = store.label_for_name("errand");
        assert_eq!(first, second);
        assert_eq!(store.labels().len(), 1);
    }

    #[test]
    fn deleting_a_task_takes_its_subtasks() {
        let (_dir, mut store) = store();
        let parent = task(&mut store, "Parent");
        let child = task(&mut store, "Child");
        let grandchild = task(&mut store, "Grandchild");
        store.task_mut(&child).unwrap().parent_id = Some(parent.clone());
        store.task_mut(&grandchild).unwrap().parent_id = Some(child.clone());

        let removed = store.remove_task(&parent);

        assert_eq!(removed.len(), 3);
        assert!(store.tasks().is_empty());
    }

    #[test]
    fn deleted_tasks_can_be_put_back_for_undo() {
        let (_dir, mut store) = store();
        let id = task(&mut store, "Oops");
        let removed = store.remove_task(&id);
        store.restore_tasks(removed);
        assert_eq!(store.task(&id).unwrap().content, "Oops");
    }

    #[test]
    fn completing_a_parent_completes_its_subtasks() {
        let (_dir, mut store) = store();
        let parent = task(&mut store, "Parent");
        let child = task(&mut store, "Child");
        store.task_mut(&child).unwrap().parent_id = Some(parent.clone());

        store.complete_task(&parent, instant(2026, 7, 30), date(2026, 7, 30));

        assert!(store.task(&parent).unwrap().checked);
        assert!(store.task(&child).unwrap().checked);
    }

    #[test]
    fn completing_a_recurring_parent_leaves_its_subtasks_alone() {
        let (_dir, mut store) = store();
        let parent = task(&mut store, "Weekly review");
        let child = task(&mut store, "Step one");
        store.task_mut(&child).unwrap().parent_id = Some(parent.clone());
        store.task_mut(&parent).unwrap().due =
            Some(Due::on(date(2026, 7, 30)).repeating(Recurrence::every(1, Unit::Week)));

        let outcome = store
            .complete_task(&parent, instant(2026, 7, 30), date(2026, 7, 30))
            .unwrap();

        assert!(matches!(outcome, Completion::Rescheduled { .. }));
        assert!(
            !store.task(&child).unwrap().checked,
            "the occurrence moved on; the subtask was not finished"
        );
    }

    #[test]
    fn reopening_a_subtask_reopens_its_parents() {
        let (_dir, mut store) = store();
        let parent = task(&mut store, "Parent");
        let child = task(&mut store, "Child");
        store.task_mut(&child).unwrap().parent_id = Some(parent.clone());
        store.complete_task(&parent, instant(2026, 7, 30), date(2026, 7, 30));

        store.uncomplete_task(&child, instant(2026, 7, 31));

        assert!(!store.task(&child).unwrap().checked);
        assert!(!store.task(&parent).unwrap().checked);
    }

    #[test]
    fn progress_counts_subtasks_too() {
        let (_dir, mut store) = store();
        let parent = task(&mut store, "Parent");
        let child = task(&mut store, "Child");
        store.task_mut(&child).unwrap().parent_id = Some(parent.clone());

        assert_eq!(store.progress(&ProjectId::inbox()), (0, 2));
        store.complete_task(&child, instant(2026, 7, 30), date(2026, 7, 30));
        assert_eq!(store.progress(&ProjectId::inbox()), (1, 2));
    }

    #[test]
    fn label_counts_ignore_completed_tasks() {
        let (_dir, mut store) = store();
        let label = store.label_for_name("errand");
        let a = task(&mut store, "One");
        let b = task(&mut store, "Two");
        store.task_mut(&a).unwrap().add_label(label.clone());
        store.task_mut(&b).unwrap().add_label(label.clone());

        assert_eq!(store.label_counts().get(&label), Some(&2));
        store.complete_task(&a, instant(2026, 7, 30), date(2026, 7, 30));
        assert_eq!(store.label_counts().get(&label), Some(&1));
    }

    #[test]
    fn new_tasks_go_to_the_end_of_their_own_list() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));

        let first = task(&mut store, "Inbox one");
        let second = task(&mut store, "Inbox two");
        let elsewhere = store.add_task(Task::new(project, "Work one", instant(2026, 7, 30)));

        let first_key = store.task(&first).unwrap().order.clone();
        let second_key = store.task(&second).unwrap().order.clone();
        assert!(first_key < second_key, "{first_key} < {second_key}");
        // A different project starts its own list rather than continuing this
        // one, so the same key in two projects means nothing.
        assert_eq!(store.task(&elsewhere).unwrap().order, first_key);
    }

    // --- ordering and moving --------------------------------------------

    /// The titles of a list, in the order they would be drawn.
    fn order_of(store: &Store, project: &ProjectId, section: Option<&SectionId>) -> Vec<String> {
        store
            .tasks_in(project, section)
            .into_iter()
            .map(|task| task.content.clone())
            .collect()
    }

    fn seeded_list(store: &mut Store, names: [&str; 3]) -> Vec<TaskId> {
        names
            .iter()
            .map(|name| store.add_task(Task::new(ProjectId::inbox(), *name, instant(2026, 7, 30))))
            .collect()
    }

    #[test]
    fn a_fresh_list_is_in_the_order_it_was_typed() {
        let (_dir, mut store) = store();
        seeded_list(&mut store, ["One", "Two", "Three"]);
        assert_eq!(
            order_of(&store, &ProjectId::inbox(), None),
            vec!["One", "Two", "Three"]
        );
    }

    #[test]
    fn a_task_moved_up_lands_where_it_was_dropped() {
        let (_dir, mut store) = store();
        let ids = seeded_list(&mut store, ["One", "Two", "Three"]);

        // Drag "Three" to the top.
        assert!(store.move_task(&ids[2], &ProjectId::inbox(), None, 0, instant(2026, 7, 31)));
        assert_eq!(
            order_of(&store, &ProjectId::inbox(), None),
            vec!["Three", "One", "Two"]
        );
    }

    #[test]
    fn a_task_moved_down_its_own_list_does_not_land_one_short() {
        let (_dir, mut store) = store();
        let ids = seeded_list(&mut store, ["One", "Two", "Three"]);

        // Drag "One" to the end. The index counts the list without it, so 2
        // is the last position — the off-by-one this test exists for is
        // getting "One", "Three", "Two" instead.
        assert!(store.move_task(&ids[0], &ProjectId::inbox(), None, 2, instant(2026, 7, 31)));
        assert_eq!(
            order_of(&store, &ProjectId::inbox(), None),
            vec!["Two", "Three", "One"]
        );
    }

    #[test]
    fn an_index_past_the_end_lands_at_the_end_rather_than_failing() {
        let (_dir, mut store) = store();
        let ids = seeded_list(&mut store, ["One", "Two", "Three"]);
        assert!(store.move_task(&ids[0], &ProjectId::inbox(), None, 99, instant(2026, 7, 31)));
        assert_eq!(
            order_of(&store, &ProjectId::inbox(), None),
            vec!["Two", "Three", "One"]
        );
    }

    #[test]
    fn moving_between_sections_closes_the_gap_it_left() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let doing = store.add_section(Section::new(project.clone(), "Doing"));

        let ids: Vec<TaskId> = ["One", "Two", "Three"]
            .iter()
            .map(|name| store.add_task(Task::new(project.clone(), *name, instant(2026, 7, 30))))
            .collect();

        assert!(store.move_task(&ids[1], &project, Some(&doing), 0, instant(2026, 7, 31)));

        assert_eq!(order_of(&store, &project, None), vec!["One", "Three"]);
        assert_eq!(order_of(&store, &project, Some(&doing)), vec!["Two"]);

        // The list left behind is *not* rewritten. A key says where a task
        // sits relative to its neighbours rather than how many there are, so
        // taking one out leaves the rest already correct — and taking a drag
        // from one changed record to three is exactly what sync cannot afford.
        let remaining: Vec<Order> = store
            .tasks_in(&project, None)
            .into_iter()
            .map(|task| task.order.clone())
            .collect();
        let mut sorted = remaining.clone();
        sorted.sort();
        assert_eq!(remaining, sorted);
    }

    /// The reason `order` is a key and not a position.
    #[test]
    fn moving_a_task_rewrites_that_task_and_no_other() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let ids: Vec<TaskId> = ["One", "Two", "Three", "Four"]
            .iter()
            .map(|name| store.add_task(Task::new(project.clone(), *name, instant(2026, 7, 30))))
            .collect();

        let before: Vec<(Order, DateTime<Utc>)> = ids
            .iter()
            .map(|id| {
                let task = store.task(id).unwrap();
                (task.order.clone(), task.updated_at)
            })
            .collect();

        // Drag the last task to the top.
        assert!(store.move_task(&ids[3], &project, None, 0, instant(2026, 7, 31)));
        assert_eq!(
            order_of(&store, &project, None),
            vec!["Four", "One", "Two", "Three"]
        );

        let touched: Vec<&str> = ids
            .iter()
            .zip(&before)
            .filter(|(id, (order, updated_at))| {
                let task = store.task(id).unwrap();
                task.order != *order || task.updated_at != *updated_at
            })
            .map(|(id, _)| store.task(id).unwrap().content.as_str())
            .collect();

        // Under sync, a drag that rewrote the whole list would send the whole
        // list — and two machines dragging within it would merge into an order
        // neither of them arranged.
        assert_eq!(touched, vec!["Four"]);
    }

    #[test]
    fn a_moved_task_takes_its_subtasks_to_the_new_project() {
        let (_dir, mut store) = store();
        let work = store.add_project(Project::new("Work", Color::Blue));

        let parent = store.add_task(Task::new(
            ProjectId::inbox(),
            "Move house",
            instant(2026, 7, 30),
        ));
        let child = store.add_task(Task::new(ProjectId::inbox(), "Pack", instant(2026, 7, 30)));
        store.task_mut(&child).unwrap().parent_id = Some(parent.clone());

        assert!(store.move_task(&parent, &work, None, 0, instant(2026, 7, 31)));

        assert_eq!(store.task(&parent).unwrap().project_id, work);
        assert_eq!(
            store.task(&child).unwrap().project_id,
            work,
            "a subtask left behind is invisible under its parent"
        );
        // The subtask is still a subtask, not loose in the new project.
        assert_eq!(store.task(&child).unwrap().parent_id, Some(parent));
        assert_eq!(order_of(&store, &work, None), vec!["Move house"]);
    }

    #[test]
    fn a_section_from_another_project_is_refused() {
        let (_dir, mut store) = store();
        let work = store.add_project(Project::new("Work", Color::Blue));
        let home = store.add_project(Project::new("Home", Color::Green));
        let doing = store.add_section(Section::new(work.clone(), "Doing"));

        let id = store.add_task(Task::new(
            home.clone(),
            "Mow the lawn",
            instant(2026, 7, 30),
        ));

        assert!(
            !store.move_task(&id, &home, Some(&doing), 0, instant(2026, 7, 31)),
            "Home has no Doing section"
        );
        assert_eq!(store.task(&id).unwrap().section_id, None);
        assert_eq!(store.task(&id).unwrap().project_id, home);
    }

    #[test]
    fn moving_to_a_project_that_is_gone_is_refused() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        store.remove_project(&project);
        let id = store.add_task(Task::new(
            ProjectId::inbox(),
            "Still here",
            instant(2026, 7, 30),
        ));

        assert!(!store.move_task(&id, &project, None, 0, instant(2026, 7, 31)));
        assert!(store.task(&id).unwrap().project_id.is_inbox());
    }

    #[test]
    fn subtasks_do_not_appear_loose_in_their_project() {
        let (_dir, mut store) = store();
        let parent = store.add_task(Task::new(
            ProjectId::inbox(),
            "Parent",
            instant(2026, 7, 30),
        ));
        let child = store.add_task(Task::new(ProjectId::inbox(), "Child", instant(2026, 7, 30)));
        store.task_mut(&child).unwrap().parent_id = Some(parent);

        assert_eq!(order_of(&store, &ProjectId::inbox(), None), vec!["Parent"]);
    }

    #[test]
    fn renumbering_does_not_count_as_editing_the_tasks() {
        let (_dir, mut store) = store();
        let ids = seeded_list(&mut store, ["One", "Two", "Three"]);
        let untouched = store.task(&ids[1]).unwrap().updated_at;

        store.move_task(&ids[2], &ProjectId::inbox(), None, 0, instant(2026, 7, 31));

        assert_eq!(
            store.task(&ids[1]).unwrap().updated_at,
            untouched,
            "a neighbour shifting is not an edit to this task"
        );
        assert_eq!(
            store.task(&ids[2]).unwrap().updated_at,
            instant(2026, 7, 31),
            "but the task that actually moved did change"
        );
    }

    #[test]
    fn sections_can_be_renamed_and_reordered() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let a = store.add_section(Section::new(project.clone(), "A"));
        let b = store.add_section(Section::new(project.clone(), "B"));
        let c = store.add_section(Section::new(project.clone(), "C"));

        assert!(store.rename_section(&b, "Middle"));
        assert!(store.move_section(&c, 0));

        let names: Vec<&str> = store
            .sections_in(&project)
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        assert_eq!(names, vec!["C", "A", "Middle"]);
        let _ = a;
    }

    #[test]
    fn renaming_a_section_that_is_gone_is_refused_rather_than_panicking() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let id = store.add_section(Section::new(project.clone(), "Doing"));
        store.remove_section(&id, instant(2026, 7, 31));

        assert!(!store.rename_section(&id, "Anything"));
        assert!(!store.move_section(&id, 0));
    }

    // --- quick add ------------------------------------------------------

    fn quick_add(store: &mut Store, line: &str) -> TaskId {
        let parsed = crate::parse::parse_quick_add(line, date(2026, 7, 30), &store.vocabulary());
        store.add_from_quick_add(&parsed, &ProjectId::inbox(), None, instant(2026, 7, 30))
    }

    #[test]
    fn a_quick_add_line_becomes_a_task_with_everything_it_named() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        store.add_section(Section::new(project.clone(), "Admin"));

        let id = quick_add(&mut store, "Email Sam #Work /Admin @email p2 friday !30m");
        let task = store.task(&id).unwrap();

        assert_eq!(task.content, "Email Sam");
        assert_eq!(task.project_id, project);
        assert!(task.section_id.is_some());
        assert_eq!(task.priority, crate::Priority::P2);
        assert_eq!(task.due.as_ref().unwrap().date, date(2026, 7, 31));
        assert_eq!(task.labels.len(), 1);
        assert_eq!(task.reminders.len(), 1);
        assert_eq!(store.label(&task.labels[0]).unwrap().name, "email");
    }

    #[test]
    fn a_label_that_does_not_exist_yet_is_created() {
        let (_dir, mut store) = store();
        assert!(store.labels().is_empty());
        quick_add(&mut store, "Post a letter @errand");
        assert_eq!(store.labels().len(), 1);
        assert_eq!(store.labels()[0].name, "errand");
    }

    #[test]
    fn a_project_that_does_not_exist_is_not_created_by_a_typo() {
        let (_dir, mut store) = store();
        store.add_project(Project::new("Work", Color::Blue));

        let id = quick_add(&mut store, "Something #Wrok");

        assert_eq!(store.projects().len(), 2, "no project was invented");
        assert!(
            store.task(&id).unwrap().project_id.is_inbox(),
            "the task lands somewhere visible instead"
        );
    }

    #[test]
    fn a_task_lands_in_the_default_project_when_the_line_does_not_say() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let parsed =
            crate::parse::parse_quick_add("Just a task", date(2026, 7, 30), &store.vocabulary());

        let id = store.add_from_quick_add(&parsed, &project, None, instant(2026, 7, 30));
        assert_eq!(store.task(&id).unwrap().project_id, project);
    }

    #[test]
    fn a_default_project_that_has_been_deleted_falls_back_to_the_inbox() {
        let (_dir, mut store) = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        store.remove_project(&project);

        let parsed =
            crate::parse::parse_quick_add("Orphan", date(2026, 7, 30), &store.vocabulary());
        let id = store.add_from_quick_add(&parsed, &project, None, instant(2026, 7, 30));
        assert!(store.task(&id).unwrap().project_id.is_inbox());
    }

    #[test]
    fn a_default_section_does_not_follow_a_task_into_another_project() {
        let (_dir, mut store) = store();
        let work = store.add_project(Project::new("Work", Color::Blue));
        let section = store.add_section(Section::new(work.clone(), "Admin"));
        let home = store.add_project(Project::new("Home", Color::Green));

        // Typed from inside Work's Admin section, but naming another project.
        let parsed = crate::parse::parse_quick_add(
            "Mow the lawn #Home",
            date(2026, 7, 30),
            &store.vocabulary(),
        );
        let id = store.add_from_quick_add(&parsed, &work, Some(&section), instant(2026, 7, 30));

        let task = store.task(&id).unwrap();
        assert_eq!(task.project_id, home);
        assert_eq!(
            task.section_id, None,
            "Home has no Admin section to file it under"
        );
    }

    #[test]
    fn the_vocabulary_carries_every_name_the_parser_needs() {
        let (_dir, mut store) = store();
        let work = store.add_project(Project::new("My Big Project", Color::Blue));
        store.add_section(Section::new(work.clone(), "In Progress"));
        store.label_for_name("high energy");

        let vocabulary = store.vocabulary();
        assert!(vocabulary.projects.contains(&"My Big Project".to_string()));
        assert!(vocabulary.sections.contains(&"In Progress".to_string()));
        assert!(vocabulary.labels.contains(&"high energy".to_string()));

        // And a multi-word name round-trips through the parser.
        let id = quick_add(&mut store, "Do it #My Big Project");
        assert_eq!(store.task(&id).unwrap().project_id, work);
        assert_eq!(store.task(&id).unwrap().content, "Do it");
    }

    // --- saved filters --------------------------------------------------

    #[test]
    fn a_saved_filter_survives_a_round_trip_through_the_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("planner.json");

        let id = {
            let (mut store, _) = Store::open_at(&path);
            let id = store.put_filter(SavedFilter::new(
                "Urgent this week",
                "p1 & due before: next week",
                Color::Red,
            ));
            store.save().unwrap();
            id
        };

        let (store, _) = Store::open_at(&path);
        let filter = store.filter(&id).expect("the filter came back");
        assert_eq!(filter.name, "Urgent this week");
        assert_eq!(filter.query, "p1 & due before: next week");
        assert_eq!(filter.color, Color::Red);
    }

    #[test]
    fn saving_a_filter_twice_replaces_it_rather_than_duplicating_it() {
        let (_dir, mut store) = store();
        let mut filter = SavedFilter::new("Work", "#Work", Color::Blue);
        let id = store.put_filter(filter.clone());

        filter.name = "Work, urgent".into();
        filter.query = "#Work & p1".into();
        store.put_filter(filter);

        assert_eq!(store.filters().len(), 1);
        assert_eq!(store.filter(&id).unwrap().name, "Work, urgent");
    }

    #[test]
    fn a_deleted_filter_comes_back_so_it_can_be_undone() {
        let (_dir, mut store) = store();
        let id = store.put_filter(SavedFilter::new("Gone", "p1", Color::Blue));

        let removed = store.remove_filter(&id).expect("it was there");
        assert_eq!(removed.name, "Gone");
        assert!(store.filter(&id).is_none());
        assert_eq!(store.remove_filter(&id), None);

        store.put_filter(removed);
        assert!(store.filter(&id).is_some());
    }

    #[test]
    fn filters_keep_the_order_they_were_added_in() {
        let (_dir, mut store) = store();
        for name in ["One", "Two", "Three"] {
            store.put_filter(SavedFilter::new(name, "p1", Color::Blue));
        }
        let names: Vec<&str> = store
            .filters_ordered()
            .iter()
            .map(|filter| filter.name.as_str())
            .collect();
        assert_eq!(names, vec!["One", "Two", "Three"]);
    }

    #[test]
    fn a_store_written_before_filters_existed_still_opens() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("planner.json");
        // No `filters` key at all: the schema grew after this file was written.
        fs::write(
            &path,
            r#"{"version":1,"projects":[],"labels":[],"tasks":[]}"#,
        )
        .unwrap();

        let (store, outcome) = Store::open_at(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert!(store.filters().is_empty());
    }

    #[test]
    fn each_new_project_gets_a_fresh_colour() {
        let (_dir, mut store) = store();
        let first = store.next_project_color();
        store.add_project(Project::new("One", first));
        let second = store.next_project_color();
        assert_ne!(first, second);
    }
}

//! What comes back.
//!
//! These are deliberately not the stored records. A [`Task`] carries a project
//! id, a section id and a list of label ids, which are exactly the fields a
//! reader who is not holding the whole store cannot do anything with — an
//! assistant handed `"project_id": "1f3a2b-7"` has learned nothing and has to
//! ask again. So a view resolves every id to the name the user would say, and
//! flattens the parts of a due date that need three types to store into the
//! one phrase that produced them.
//!
//! Ids are still present, because they are how the *next* call refers to this
//! thing without ambiguity. Names are for understanding; ids are for acting.
//!
//! Everything empty is omitted rather than serialised as `null` or `[]`. A
//! response that goes into a context window should spend its tokens on what is
//! there.

use serde::Serialize;

use super::help::Verb;
use crate::id::TaskId;
use crate::search::Hit;
use crate::store::Store;
use crate::task::{Completion, Reminder, Task, Trigger};
use crate::{Label, Project, SavedFilter};
use chrono::NaiveDate;

/// A task, with every id it holds resolved to a name.
#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub id: String,
    pub content: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// The date, and the time if there is one: `2026-08-07`, `2026-08-07 09:00`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// The repeat rule as the phrase that would produce it, so it can be read
    /// back to the user and typed back into `due=` unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeats: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// `p1`–`p4`. Always present: an absent priority and `p4` are the same
    /// thing, and saying so is shorter than explaining it.
    pub priority: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub overdue: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub completed: bool,
    /// The task this is a subtask of, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// How many subtasks it has, and how many of those are done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask_count: Option<Progress>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<String>,
    /// Filled by `show` only. `list` returning every subtask of every task
    /// would bury the list it was asked for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subtasks: Vec<TaskView>,
}

/// How much of something is finished.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<String>,
    pub open: usize,
    pub done: usize,
    #[serde(skip_serializing_if = "is_false")]
    pub inbox: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LabelView {
    pub name: String,
    /// How many open tasks carry it.
    pub open: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FilterView {
    pub name: String,
    /// The query it saves, which `list` will take verbatim.
    pub query: String,
}

/// What the store looks like in total, so a caller can orient in one call.
#[derive(Debug, Clone, Serialize)]
pub struct Counts {
    pub open: usize,
    pub completed: usize,
    pub due_today: usize,
    pub overdue: usize,
    pub inbox: usize,
}

/// A search result.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HitView {
    Task {
        id: String,
        title: String,
        context: String,
    },
    Project {
        id: String,
        name: String,
    },
    Label {
        name: String,
    },
}

/// A task that was deleted.
#[derive(Debug, Clone, Serialize)]
pub struct RemovedView {
    pub id: String,
    pub content: String,
}

/// What ticking a task off did.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum CompletionView {
    /// Finished, and it will not come back.
    Done,
    /// It repeats: the occurrence was completed, and the task is open again on
    /// a later date. Reporting this as `done` would be the single most
    /// misleading thing this interface could say — the user would be told they
    /// had finished something that is still in their list.
    ///
    /// Named for the completion rather than for the reschedule on purpose. The
    /// obvious name, `rescheduled`, describes what happened to the *task* and
    /// reads like the request was turned into something else, which invites a
    /// reader to report "I moved that to Thursday" instead of "done, it comes
    /// back Thursday". `next_due` rather than `due` for the same reason: it is
    /// not a date anybody asked to set.
    CompletedAndRepeats { next_due: String },
    /// It was already ticked off. Nothing happened.
    AlreadyDone,
}

impl From<Completion> for CompletionView {
    fn from(completion: Completion) -> Self {
        match completion {
            Completion::Done => Self::Done,
            Completion::Rescheduled { due } => Self::CompletedAndRepeats {
                next_due: due_phrase(&due),
            },
            Completion::AlreadyDone => Self::AlreadyDone,
        }
    }
}

/// Everything a verb can answer with.
///
/// Internally tagged, so every response says which verb produced it. A caller
/// reading a transcript of several calls can tell them apart without tracking
/// what it asked for.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Response {
    /// Plain text, printed as-is rather than as JSON.
    Help {
        #[serde(skip)]
        text: String,
    },
    Describe {
        verbs: &'static [Verb],
    },
    Overview {
        projects: Vec<ProjectView>,
        labels: Vec<LabelView>,
        filters: Vec<FilterView>,
        counts: Counts,
    },
    List {
        #[serde(skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        /// How many were returned.
        count: usize,
        /// How many matched in total. Differs from `count` only when the limit
        /// cut the list short, which is what `truncated` announces.
        matched: usize,
        #[serde(skip_serializing_if = "is_false")]
        truncated: bool,
        tasks: Vec<TaskView>,
    },
    Show {
        task: TaskView,
    },
    Search {
        count: usize,
        hits: Vec<HitView>,
    },
    Added {
        task: TaskView,
    },
    Completed {
        task: TaskView,
        #[serde(flatten)]
        outcome: CompletionView,
    },
    Reopened {
        task: TaskView,
        /// Whether it had actually been completed. `false` means it was open
        /// already and nothing was undone.
        reopened: bool,
    },
    Deleted {
        count: usize,
        removed: Vec<RemovedView>,
    },
    Updated {
        task: TaskView,
        /// What actually changed, in words. A field set to the value it already
        /// had is absent from this, so a no-op is visible as one.
        applied: Vec<String>,
    },
    ProjectAdded {
        project: ProjectView,
    },
    ProjectRenamed {
        project: ProjectView,
    },
    ProjectRemoved {
        name: String,
        /// The project itself plus any subprojects that went with it.
        projects: usize,
        tasks: usize,
    },
}

impl Response {
    /// Whether producing this changed the store.
    ///
    /// Derived from the response rather than from the command, so the one case
    /// where a write verb wrote nothing — completing an already-completed task
    /// — does not mark the store dirty and rewrite the file for nothing.
    pub fn changed_the_store(&self) -> bool {
        match self {
            Self::Completed { outcome, .. } => !matches!(outcome, CompletionView::AlreadyDone),
            Self::Updated { applied, .. } => !applied.is_empty(),
            Self::Reopened { reopened, .. } => *reopened,
            Self::Added { .. }
            | Self::Deleted { .. }
            | Self::ProjectAdded { .. }
            | Self::ProjectRenamed { .. }
            | Self::ProjectRemoved { .. } => true,
            Self::Help { .. }
            | Self::Describe { .. }
            | Self::Overview { .. }
            | Self::List { .. }
            | Self::Show { .. }
            | Self::Search { .. } => false,
        }
    }
}

impl TaskView {
    /// A task as it appears in a list: everything but the description, the
    /// reminders and the subtasks themselves.
    pub fn of(store: &Store, task: &Task, today: NaiveDate) -> Self {
        let subtasks = store.subtasks(&task.id);
        let subtask_count = (!subtasks.is_empty()).then(|| Progress {
            done: subtasks.iter().filter(|child| child.checked).count(),
            total: subtasks.len(),
        });

        Self {
            id: task.id.to_string(),
            content: task.content.clone(),
            description: String::new(),
            project: store
                .project(&task.project_id)
                .map(|project| project.name.clone())
                // A task whose project has gone is a hand-edited file, not a
                // state the app produces. Saying so is more use than an id.
                .unwrap_or_else(|| "(unknown project)".to_string()),
            section: task
                .section_id
                .as_ref()
                .and_then(|id| store.section(id))
                .map(|section| section.name.clone()),
            due: task.due.as_ref().map(due_phrase),
            repeats: task
                .due
                .as_ref()
                .and_then(|due| due.recurrence.as_ref())
                .map(|recurrence| recurrence.describe()),
            deadline: task.deadline.map(|date| date.to_string()),
            priority: task.priority.token(),
            labels: task
                .labels
                .iter()
                .filter_map(|id| store.label(id))
                .map(|label| label.name.clone())
                .collect(),
            pinned: task.pinned,
            overdue: task.is_overdue(today),
            completed: task.checked,
            parent: task.parent_id.as_ref().map(TaskId::to_string),
            subtask_count,
            reminders: Vec::new(),
            subtasks: Vec::new(),
        }
    }

    /// A task in full: the description, the reminders, and the subtasks
    /// beneath it, recursively.
    pub fn detailed(store: &Store, task: &Task, today: NaiveDate) -> Self {
        let mut view = Self::of(store, task, today);
        view.description = task.description.clone();
        view.reminders = task.reminders.iter().map(reminder_phrase).collect();
        view.subtasks = store
            .subtasks(&task.id)
            .into_iter()
            .map(|child| Self::detailed(store, child, today))
            .collect();
        view
    }
}

impl ProjectView {
    pub fn of(store: &Store, project: &Project) -> Self {
        let (done, total) = store.progress(&project.id);
        Self {
            id: project.id.to_string(),
            name: project.name.clone(),
            parent: project
                .parent_id
                .as_ref()
                .and_then(|id| store.project(id))
                .map(|parent| parent.name.clone()),
            sections: store
                .sections_in(&project.id)
                .into_iter()
                .map(|section| section.name.clone())
                .collect(),
            open: total - done,
            done,
            inbox: project.is_inbox(),
        }
    }
}

impl LabelView {
    pub fn of(label: &Label, open: usize) -> Self {
        Self {
            name: label.name.clone(),
            open,
        }
    }
}

impl FilterView {
    pub fn of(filter: &SavedFilter) -> Self {
        Self {
            name: filter.name.clone(),
            query: filter.query.clone(),
        }
    }
}

impl HitView {
    pub fn of(hit: &Hit) -> Self {
        match hit {
            Hit::Task { id, title, context } => Self::Task {
                id: id.to_string(),
                title: title.clone(),
                context: context.clone(),
            },
            Hit::Project { id, name } => Self::Project {
                id: id.to_string(),
                name: name.clone(),
            },
            // A label has an id, but nothing takes one: every verb that deals
            // in labels takes the name, because that is what the user says.
            Hit::Label { name, .. } => Self::Label { name: name.clone() },
        }
    }
}

impl RemovedView {
    pub fn of(task: &Task) -> Self {
        Self {
            id: task.id.to_string(),
            content: task.content.clone(),
        }
    }
}

/// A due date as one string: the date, and the time if there is one.
///
/// Not an RFC 3339 instant. A due date is a date in the user's own day, and
/// rendering "Friday" as `2026-08-07T00:00:00Z` would invent a midnight the
/// user never asked for and move it across a timezone besides.
pub fn due_phrase(due: &crate::Due) -> String {
    match due.time {
        Some(time) => format!("{} {}", due.date, time.format("%H:%M")),
        None => due.date.to_string(),
    }
}

fn reminder_phrase(reminder: &Reminder) -> String {
    match &reminder.trigger {
        Trigger::Absolute { at } => at.format("%Y-%m-%d %H:%M UTC").to_string(),
        Trigger::BeforeDue { minutes } => match minutes {
            m if *m % 1440 == 0 => format!("{} days before", m / 1440),
            m if *m % 60 == 0 => format!("{} hours before", m / 60),
            m => format!("{m} minutes before"),
        },
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::id::ProjectId;
    use crate::project::Section;
    use crate::recurrence::{Recurrence, Unit};
    use crate::Due;
    use chrono::{DateTime, Utc};

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
    }

    fn now() -> DateTime<Utc> {
        today().and_hms_opt(12, 0, 0).unwrap().and_utc()
    }

    fn store() -> Store {
        let (store, _) = Store::open_at(
            tempfile::TempDir::new()
                .expect("a temporary directory")
                .keep()
                .join("planner.json"),
        );
        store
    }

    fn json(view: &TaskView) -> serde_json::Value {
        serde_json::to_value(view).expect("a task view serialises")
    }

    #[test]
    fn a_view_names_what_the_record_only_points_at() {
        let mut store = store();
        let project = store.add_project(Project::new("Work", Color::Blue), now());
        let section = store.add_section(Section::new(project.clone(), "Admin"), now());
        let label = store.label_for_name("email", now());

        let mut task = Task::new(project, "Email Sam", now());
        task.section_id = Some(section);
        task.labels = vec![label];
        let id = store.add_task(task);

        let view = TaskView::of(&store, store.task(&id).unwrap(), today());

        assert_eq!(view.project, "Work");
        assert_eq!(view.section.as_deref(), Some("Admin"));
        assert_eq!(view.labels, vec!["email"]);
    }

    #[test]
    fn an_empty_field_is_left_out_rather_than_sent_as_null() {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Plain", now()));
        let view = json(&TaskView::of(&store, store.task(&id).unwrap(), today()));

        let object = view.as_object().expect("an object");
        for absent in ["due", "labels", "section", "deadline", "subtasks", "pinned"] {
            assert!(!object.contains_key(absent), "{absent} should be omitted");
        }
        // But a priority is always there, because p4 is a real answer.
        assert_eq!(object["priority"], "p4");
    }

    #[test]
    fn a_due_date_keeps_the_time_only_when_there_is_one() {
        let bare = Due::on(NaiveDate::from_ymd_opt(2026, 8, 7).unwrap());
        assert_eq!(due_phrase(&bare), "2026-08-07");

        let timed = Due::at(
            NaiveDate::from_ymd_opt(2026, 8, 7).unwrap(),
            chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        );
        assert_eq!(due_phrase(&timed), "2026-08-07 09:00");
    }

    #[test]
    fn a_repeat_comes_back_as_the_phrase_that_would_make_it() {
        let mut store = store();
        let mut task = Task::new(ProjectId::inbox(), "Water the plants", now());
        task.due = Some(Due::on(today()).repeating(Recurrence::every(2, Unit::Week)));
        let id = store.add_task(task);

        let view = TaskView::of(&store, store.task(&id).unwrap(), today());
        assert_eq!(view.repeats.as_deref(), Some("every other week"));
    }

    #[test]
    fn a_list_view_counts_its_subtasks_without_carrying_them() {
        let mut store = store();
        let parent = store.add_task(Task::new(ProjectId::inbox(), "Move house", now()));
        for name in ["Pack", "Book a van"] {
            let child = store.add_task(Task::new(ProjectId::inbox(), name, now()));
            store.task_mut(&child).unwrap().parent_id = Some(parent.clone());
        }
        store.complete_task(&store.subtasks(&parent)[0].id.clone(), now(), today());

        let listed = TaskView::of(&store, store.task(&parent).unwrap(), today());
        assert!(listed.subtasks.is_empty(), "a list must not nest subtasks");
        let count = listed.subtask_count.expect("a count");
        assert_eq!((count.done, count.total), (1, 2));

        // `show` is the one that carries them.
        let shown = TaskView::detailed(&store, store.task(&parent).unwrap(), today());
        assert_eq!(shown.subtasks.len(), 2);
    }

    #[test]
    fn a_repeating_completion_is_named_for_the_completion_not_the_reschedule() {
        // The wire name has to be unmissable: a reader that skims this and
        // reports "I rescheduled it" has told the user the opposite of what
        // they asked for.
        let view = CompletionView::from(Completion::Rescheduled {
            due: Due::on(NaiveDate::from_ymd_opt(2026, 8, 6).unwrap()),
        });
        let json = serde_json::to_value(&view).unwrap();

        assert_eq!(json["outcome"], "completed-and-repeats");
        assert_eq!(json["next_due"], "2026-08-06");
        assert_eq!(
            json["due"],
            serde_json::Value::Null,
            "`due` would read as a date somebody asked to set"
        );
    }

    #[test]
    fn a_completion_that_did_nothing_does_not_dirty_the_store() {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Done already", now()));
        let task = TaskView::of(&store, store.task(&id).unwrap(), today());

        let nothing = Response::Completed {
            task: task.clone(),
            outcome: CompletionView::AlreadyDone,
        };
        assert!(!nothing.changed_the_store());

        let something = Response::Completed {
            task,
            outcome: CompletionView::Done,
        };
        assert!(something.changed_the_store());
    }

    #[test]
    fn an_update_that_applied_nothing_does_not_dirty_the_store() {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Unchanged", now()));
        let task = TaskView::of(&store, store.task(&id).unwrap(), today());

        assert!(!Response::Updated {
            task: task.clone(),
            applied: Vec::new(),
        }
        .changed_the_store());
        assert!(Response::Updated {
            task,
            applied: vec!["priority → p1".into()],
        }
        .changed_the_store());
    }
}

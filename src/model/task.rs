//! The task record.
//!
//! A plain data type with no behaviour beyond keeping its own invariants.
//! Nothing here reads the clock except through an explicit argument: every
//! method that needs "now" is handed it. That is what makes "complete a
//! recurring task the day after it was due" a test rather than a thing you
//! find out about in November.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::due::Due;
use super::id::{LabelId, ProjectId, ReminderId, SectionId, TaskId};
use super::priority::Priority;

/// When a reminder fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Trigger {
    /// At a fixed instant, whatever the task's due date says.
    Absolute { at: DateTime<Utc> },
    /// This many minutes before the task is due. Only meaningful on a task
    /// with a due *time*: "30 minutes before some time on Friday" has no
    /// answer, so the scheduler skips it rather than guessing midnight.
    BeforeDue { minutes: i64 },
}

/// A notification attached to a task. A task may have several.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: ReminderId,
    pub trigger: Trigger,
}

impl Reminder {
    pub fn absolute(at: DateTime<Utc>) -> Self {
        Self {
            id: ReminderId::new(),
            trigger: Trigger::Absolute { at },
        }
    }

    pub fn before_due(minutes: i64) -> Self {
        Self {
            id: ReminderId::new(),
            trigger: Trigger::BeforeDue { minutes },
        }
    }
}

/// One thing to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    /// The title. Markdown, inline only.
    pub content: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    pub project_id: ProjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionId>,
    /// The task this is a subtask of. Subtasks nest arbitrarily and always
    /// share their parent's project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TaskId>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Due>,
    /// The hard date, as distinct from the day you plan to do it. A task can
    /// be scheduled for Tuesday and due Friday; collapsing the two loses the
    /// only information that tells you whether being late matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<NaiveDate>,

    #[serde(default)]
    pub priority: Priority,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<LabelId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<Reminder>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,

    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,

    pub added_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Position within its project or section under manual sorting.
    #[serde(default)]
    pub order: i32,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// What completing a task did.
///
/// Ticking a recurring task does not complete it — it moves it on. Returning
/// that as a value rather than leaving the caller to re-inspect the task means
/// the UI can say "Next: Thursday" instead of guessing, and the undo path
/// knows which of the two things it has to reverse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    /// The task is done and will not come back.
    Done,
    /// The task repeated; this is its new due date.
    Rescheduled { due: Due },
    /// The task was already complete. Nothing happened.
    AlreadyDone,
}

impl Task {
    /// A new task in a project, with everything else unset.
    pub fn new(project_id: ProjectId, content: impl Into<String>, now: DateTime<Utc>) -> Self {
        Self {
            id: TaskId::new(),
            content: content.into(),
            description: String::new(),
            project_id,
            section_id: None,
            parent_id: None,
            due: None,
            deadline: None,
            priority: Priority::default(),
            labels: Vec::new(),
            reminders: Vec::new(),
            pinned: false,
            checked: false,
            completed_at: None,
            added_at: now,
            updated_at: now,
            order: 0,
        }
    }

    /// Record that something about this task changed.
    ///
    /// Called by the store on every mutation rather than by each caller, so
    /// there is one place that can forget rather than thirty.
    pub fn touch(&mut self, now: DateTime<Utc>) {
        self.updated_at = now;
    }

    /// Whether this is a subtask of something.
    pub fn is_subtask(&self) -> bool {
        self.parent_id.is_some()
    }

    /// Whether the due date has passed. A completed task is never overdue,
    /// however long ago it was due.
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        !self.checked && self.due.as_ref().is_some_and(|due| due.is_overdue(today))
    }

    /// Whether the deadline has passed and the task is still open.
    pub fn is_past_deadline(&self, today: NaiveDate) -> bool {
        !self.checked && self.deadline.is_some_and(|deadline| deadline < today)
    }

    /// Whether the task carries a label.
    pub fn has_label(&self, label: &LabelId) -> bool {
        self.labels.contains(label)
    }

    /// Add a label, ignoring a duplicate.
    pub fn add_label(&mut self, label: LabelId) {
        if !self.has_label(&label) {
            self.labels.push(label);
        }
    }

    /// Remove a label if present.
    pub fn remove_label(&mut self, label: &LabelId) {
        self.labels.retain(|existing| existing != label);
    }

    /// Tick the task off.
    ///
    /// A recurring task moves to its next occurrence and stays open; anything
    /// else is marked done. Completing an already-completed task is a no-op
    /// rather than an error — a double click on a checkbox is not a failure.
    pub fn complete(&mut self, now: DateTime<Utc>, today: NaiveDate) -> Completion {
        if self.checked {
            return Completion::AlreadyDone;
        }

        if let Some(next) = self.due.as_ref().and_then(|due| due.advance(today)) {
            self.due = Some(next.clone());
            self.touch(now);
            return Completion::Rescheduled { due: next };
        }

        self.checked = true;
        self.completed_at = Some(now);
        // A recurrence that has run out must not sit on the completed task:
        // reopening it would otherwise resurrect a rule that is finished.
        if let Some(due) = self.due.as_mut() {
            due.recurrence = None;
        }
        self.touch(now);
        Completion::Done
    }

    /// Reopen a completed task.
    pub fn uncomplete(&mut self, now: DateTime<Utc>) {
        if !self.checked {
            return;
        }
        self.checked = false;
        self.completed_at = None;
        self.touch(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::recurrence::{End, Recurrence, Unit};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn instant(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        date(y, m, d).and_hms_opt(12, 0, 0).unwrap().and_utc()
    }

    fn task() -> Task {
        Task::new(ProjectId::inbox(), "Water the plants", instant(2026, 7, 1))
    }

    #[test]
    fn completing_a_plain_task_marks_it_done() {
        let mut task = task();
        assert_eq!(
            task.complete(instant(2026, 7, 30), date(2026, 7, 30)),
            Completion::Done
        );
        assert!(task.checked);
        assert_eq!(task.completed_at, Some(instant(2026, 7, 30)));
    }

    #[test]
    fn completing_a_recurring_task_reschedules_it_instead_of_closing_it() {
        let mut task = task();
        task.due = Some(Due::on(date(2026, 7, 30)).repeating(Recurrence::every(1, Unit::Week)));

        let outcome = task.complete(instant(2026, 7, 30), date(2026, 7, 30));

        assert!(matches!(outcome, Completion::Rescheduled { .. }));
        assert!(!task.checked, "a recurring task stays open");
        assert_eq!(task.completed_at, None);
        assert_eq!(task.due.as_ref().unwrap().date, date(2026, 8, 6));
    }

    #[test]
    fn the_last_occurrence_of_a_recurring_task_completes_it_for_good() {
        let mut task = task();
        task.due = Some(Due::on(date(2026, 7, 30)).repeating(Recurrence {
            end: End::After { remaining: 0 },
            ..Recurrence::every(1, Unit::Day)
        }));

        assert_eq!(
            task.complete(instant(2026, 7, 30), date(2026, 7, 30)),
            Completion::Done
        );
        assert!(task.checked);
    }

    #[test]
    fn a_finished_recurrence_is_not_left_on_the_completed_task() {
        let mut task = task();
        task.due = Some(Due::on(date(2026, 7, 30)).repeating(Recurrence {
            end: End::After { remaining: 0 },
            ..Recurrence::every(1, Unit::Day)
        }));
        task.complete(instant(2026, 7, 30), date(2026, 7, 30));

        // Reopening must not bring the exhausted rule back to life.
        task.uncomplete(instant(2026, 7, 31));
        assert!(!task.due.as_ref().unwrap().is_recurring());
    }

    #[test]
    fn completing_twice_changes_nothing_the_second_time() {
        let mut task = task();
        task.complete(instant(2026, 7, 30), date(2026, 7, 30));
        let before = task.clone();

        assert_eq!(
            task.complete(instant(2026, 7, 31), date(2026, 7, 31)),
            Completion::AlreadyDone
        );
        assert_eq!(task, before);
    }

    #[test]
    fn a_completed_task_is_never_overdue() {
        let mut task = task();
        task.due = Some(Due::on(date(2026, 7, 1)));
        assert!(task.is_overdue(date(2026, 7, 30)));

        task.complete(instant(2026, 7, 30), date(2026, 7, 30));
        assert!(!task.is_overdue(date(2026, 7, 30)));
    }

    #[test]
    fn a_deadline_is_tracked_separately_from_the_due_date() {
        let mut task = task();
        task.due = Some(Due::on(date(2026, 8, 4)));
        task.deadline = Some(date(2026, 7, 29));

        // Scheduled for next week, but the deadline has already gone.
        assert!(!task.is_overdue(date(2026, 7, 30)));
        assert!(task.is_past_deadline(date(2026, 7, 30)));
    }

    #[test]
    fn adding_the_same_label_twice_does_not_duplicate_it() {
        let mut task = task();
        let label = LabelId::new();
        task.add_label(label.clone());
        task.add_label(label.clone());
        assert_eq!(task.labels, vec![label.clone()]);

        task.remove_label(&label);
        assert!(task.labels.is_empty());
    }

    #[test]
    fn an_empty_task_serialises_without_a_field_per_unset_option() {
        let mut task = task();
        task.id = TaskId::from_raw("t1");
        task.order = 0;
        let json = serde_json::to_string(&task).unwrap();
        assert_eq!(
            json,
            r#"{"id":"t1","content":"Water the plants","project_id":"inbox","priority":"P4","added_at":"2026-07-01T12:00:00Z","updated_at":"2026-07-01T12:00:00Z","order":0}"#
        );
    }

    #[test]
    fn a_task_round_trips_through_json() {
        let mut task = task();
        task.due = Some(Due::at(
            date(2026, 7, 30),
            chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        ));
        task.deadline = Some(date(2026, 8, 1));
        task.priority = Priority::P1;
        task.labels = vec![LabelId::from_raw("errand")];
        task.reminders = vec![Reminder::before_due(30)];
        task.pinned = true;

        let json = serde_json::to_string(&task).unwrap();
        assert_eq!(serde_json::from_str::<Task>(&json).unwrap(), task);
    }
}

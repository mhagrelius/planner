//! Which reminders are due, and when the next one is.
//!
//! Pure, so "does a reminder set for 30 minutes before a 09:00 task fire at
//! 08:30" is a test rather than a morning spent waiting. The application
//! supplies the clock and the local timezone offset; nothing here reads
//! either.
//!
//! A reminder that has already passed when the app starts does not fire.
//! Launching on Tuesday and being told about Monday morning's meeting is
//! noise, and worse, it is noise that arrives every time you open the app
//! until you deal with the task. Only reminders that come due *while the app
//! is running* are shown, and [`Schedule`] remembers which have gone off so a
//! re-arm does not repeat them.

use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use super::id::{ReminderId, TaskId};
use super::store::Store;
use super::task::Trigger;

/// A reminder that has come due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Firing {
    pub task: TaskId,
    pub reminder: ReminderId,
    pub title: String,
    /// When it was supposed to fire.
    pub at: DateTime<Utc>,
}

/// Keeps track of which reminders have already been shown.
#[derive(Debug, Default)]
pub struct Schedule {
    fired: HashSet<(TaskId, ReminderId)>,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything that has come due at or before `now` and has not been shown.
    ///
    /// Marks them as shown, so calling this twice does not fire twice.
    pub fn take_due<Tz: TimeZone>(
        &mut self,
        store: &Store,
        now: DateTime<Utc>,
        zone: &Tz,
    ) -> Vec<Firing> {
        let mut due: Vec<Firing> = pending(store, zone)
            .into_iter()
            .filter(|firing| firing.at <= now)
            .filter(|firing| {
                !self
                    .fired
                    .contains(&(firing.task.clone(), firing.reminder.clone()))
            })
            .collect();

        due.sort_by_key(|firing| firing.at);
        for firing in &due {
            self.fired
                .insert((firing.task.clone(), firing.reminder.clone()));
        }
        due
    }

    /// Treat everything currently due as already shown.
    ///
    /// Called once at startup so that reminders which passed while the app was
    /// closed stay quiet, without special-casing the first tick.
    pub fn catch_up<Tz: TimeZone>(&mut self, store: &Store, now: DateTime<Utc>, zone: &Tz) {
        for firing in pending(store, zone) {
            if firing.at <= now {
                self.fired.insert((firing.task, firing.reminder));
            }
        }
    }

    /// When the next reminder comes due, if any.
    pub fn next_after<Tz: TimeZone>(
        &self,
        store: &Store,
        now: DateTime<Utc>,
        zone: &Tz,
    ) -> Option<DateTime<Utc>> {
        pending(store, zone)
            .into_iter()
            .filter(|firing| firing.at > now)
            .map(|firing| firing.at)
            .min()
    }

    /// Forget that a reminder fired, so it can fire again.
    ///
    /// A recurring task keeps its reminders across occurrences; without this
    /// the second Monday's standup would be silent.
    pub fn forget(&mut self, task: &TaskId) {
        self.fired.retain(|(id, _)| id != task);
    }

    /// How many reminders are on record as having fired, for tests.
    pub fn fired_count(&self) -> usize {
        self.fired.len()
    }
}

/// Every reminder on an open task, resolved to an instant.
///
/// A `BeforeDue` reminder on a task with a date but no *time* is skipped:
/// "30 minutes before some time on Friday" has no answer, and picking midnight
/// would fire it at half past eleven on Thursday night.
fn pending<Tz: TimeZone>(store: &Store, zone: &Tz) -> Vec<Firing> {
    let mut firings = Vec::new();

    for task in store.tasks().iter().filter(|task| !task.checked) {
        for reminder in &task.reminders {
            let at = match &reminder.trigger {
                Trigger::Absolute { at } => Some(*at),
                Trigger::BeforeDue { minutes } => task
                    .due
                    .as_ref()
                    .and_then(|due| due.time.map(|time| (due.date, time)))
                    .and_then(|(date, time)| local_instant(date, time, zone))
                    .map(|instant| instant - chrono::Duration::minutes(*minutes)),
            };
            if let Some(at) = at {
                firings.push(Firing {
                    task: task.id.clone(),
                    reminder: reminder.id.clone(),
                    title: task.content.clone(),
                    at,
                });
            }
        }
    }
    firings
}

/// A local date and time as an instant.
///
/// Ambiguous and skipped times happen twice a year when the clocks change.
/// `earliest` picks the first of an ambiguous pair and `None` falls back to
/// the date at midnight rather than dropping the reminder, because a reminder
/// an hour out is better than one that never arrives.
fn local_instant<Tz: TimeZone>(
    date: NaiveDate,
    time: chrono::NaiveTime,
    zone: &Tz,
) -> Option<DateTime<Utc>> {
    let naive = date.and_time(time);
    zone.from_local_datetime(&naive)
        .earliest()
        .or_else(|| {
            zone.from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
                .earliest()
        })
        .map(|local| local.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::due::Due;
    use crate::id::ProjectId;
    use crate::task::{Reminder, Task};
    use chrono::{FixedOffset, NaiveTime};

    /// UTC, so the arithmetic in the tests is the arithmetic in the code.
    fn zone() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        date(y, m, d).and_hms_opt(h, min, 0).unwrap().and_utc()
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

    /// A task due at 09:00 on 31 July 2026, with a reminder N minutes before.
    fn with_reminder(store: &mut Store, minutes: i64) -> TaskId {
        let id = store.add_task(Task::new(
            ProjectId::inbox(),
            "Standup",
            at(2026, 7, 30, 12, 0),
        ));
        let task = store.task_mut(&id).unwrap();
        task.due = Some(Due::at(
            date(2026, 7, 31),
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        ));
        task.reminders = vec![Reminder::before_due(minutes)];
        id
    }

    #[test]
    fn a_reminder_fires_the_stated_time_before_the_task_is_due() {
        let mut store = store();
        let id = with_reminder(&mut store, 30);
        let mut schedule = Schedule::new();

        // A minute early: nothing yet.
        assert!(schedule
            .take_due(&store, at(2026, 7, 31, 8, 29), &zone())
            .is_empty());

        let due = schedule.take_due(&store, at(2026, 7, 31, 8, 30), &zone());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].task, id);
        assert_eq!(due[0].title, "Standup");
        assert_eq!(due[0].at, at(2026, 7, 31, 8, 30));
    }

    #[test]
    fn a_reminder_fires_once_however_often_the_tick_runs() {
        let mut store = store();
        with_reminder(&mut store, 30);
        let mut schedule = Schedule::new();

        assert_eq!(
            schedule
                .take_due(&store, at(2026, 7, 31, 9, 0), &zone())
                .len(),
            1
        );
        assert!(schedule
            .take_due(&store, at(2026, 7, 31, 9, 0), &zone())
            .is_empty());
        assert!(schedule
            .take_due(&store, at(2026, 7, 31, 23, 0), &zone())
            .is_empty());
    }

    #[test]
    fn a_reminder_that_passed_while_the_app_was_shut_stays_quiet() {
        let mut store = store();
        with_reminder(&mut store, 30);
        let mut schedule = Schedule::new();

        // Started the app the next day.
        schedule.catch_up(&store, at(2026, 8, 1, 10, 0), &zone());
        assert!(schedule
            .take_due(&store, at(2026, 8, 1, 10, 0), &zone())
            .is_empty());
    }

    #[test]
    fn catching_up_does_not_silence_a_reminder_still_to_come() {
        let mut store = store();
        with_reminder(&mut store, 30);
        let mut schedule = Schedule::new();

        schedule.catch_up(&store, at(2026, 7, 31, 8, 0), &zone());
        assert_eq!(
            schedule
                .take_due(&store, at(2026, 7, 31, 8, 30), &zone())
                .len(),
            1
        );
    }

    #[test]
    fn a_completed_task_does_not_remind_anyone() {
        let mut store = store();
        let id = with_reminder(&mut store, 30);
        store.complete_task(&id, at(2026, 7, 31, 8, 0), date(2026, 7, 31));

        let mut schedule = Schedule::new();
        assert!(schedule
            .take_due(&store, at(2026, 7, 31, 9, 0), &zone())
            .is_empty());
    }

    #[test]
    fn a_relative_reminder_on_a_task_with_no_time_is_skipped() {
        let mut store = store();
        let id = with_reminder(&mut store, 30);
        // Due on Friday, but not at any particular time.
        store.task_mut(&id).unwrap().due = Some(Due::on(date(2026, 7, 31)));

        let mut schedule = Schedule::new();
        assert!(
            schedule
                .take_due(&store, at(2026, 8, 1, 0, 0), &zone())
                .is_empty(),
            "midnight is a guess, and it would fire the night before"
        );
    }

    #[test]
    fn an_absolute_reminder_needs_no_due_date_at_all() {
        let mut store = store();
        let id = store.add_task(Task::new(
            ProjectId::inbox(),
            "Ring the bank",
            at(2026, 7, 30, 12, 0),
        ));
        store.task_mut(&id).unwrap().reminders = vec![Reminder::absolute(at(2026, 7, 31, 14, 0))];

        let mut schedule = Schedule::new();
        assert!(schedule
            .take_due(&store, at(2026, 7, 31, 13, 59), &zone())
            .is_empty());
        assert_eq!(
            schedule
                .take_due(&store, at(2026, 7, 31, 14, 0), &zone())
                .len(),
            1
        );
    }

    #[test]
    fn the_next_reminder_is_the_soonest_one_still_ahead() {
        let mut store = store();
        with_reminder(&mut store, 30);
        let id = store.add_task(Task::new(
            ProjectId::inbox(),
            "Later",
            at(2026, 7, 30, 12, 0),
        ));
        store.task_mut(&id).unwrap().reminders = vec![Reminder::absolute(at(2026, 7, 31, 18, 0))];

        let schedule = Schedule::new();
        assert_eq!(
            schedule.next_after(&store, at(2026, 7, 31, 0, 0), &zone()),
            Some(at(2026, 7, 31, 8, 30))
        );
        // Once the first has passed, the next one is the later one.
        assert_eq!(
            schedule.next_after(&store, at(2026, 7, 31, 9, 0), &zone()),
            Some(at(2026, 7, 31, 18, 0))
        );
        assert_eq!(
            schedule.next_after(&store, at(2026, 8, 2, 0, 0), &zone()),
            None
        );
    }

    #[test]
    fn a_recurring_task_reminds_again_at_its_next_occurrence() {
        let mut store = store();
        let id = with_reminder(&mut store, 30);
        store.task_mut(&id).unwrap().due = Some(
            Due::at(date(2026, 7, 31), NaiveTime::from_hms_opt(9, 0, 0).unwrap())
                .repeating(crate::Recurrence::every(1, crate::recurrence::Unit::Day)),
        );

        let mut schedule = Schedule::new();
        assert_eq!(
            schedule
                .take_due(&store, at(2026, 7, 31, 8, 30), &zone())
                .len(),
            1
        );

        // Ticking it off moves it to tomorrow, and the reminder is live again.
        store.complete_task(&id, at(2026, 7, 31, 9, 0), date(2026, 7, 31));
        schedule.forget(&id);

        assert_eq!(
            schedule
                .take_due(&store, at(2026, 8, 1, 8, 30), &zone())
                .len(),
            1
        );
    }

    #[test]
    fn a_reminder_is_resolved_in_the_local_zone_not_utc() {
        let mut store = store();
        with_reminder(&mut store, 0);
        // Two hours ahead of UTC: 09:00 local is 07:00 UTC.
        let zone = FixedOffset::east_opt(2 * 3600).unwrap();

        let mut schedule = Schedule::new();
        assert!(schedule
            .take_due(&store, at(2026, 7, 31, 6, 59), &zone)
            .is_empty());
        assert_eq!(
            schedule
                .take_due(&store, at(2026, 7, 31, 7, 0), &zone)
                .len(),
            1
        );
    }

    #[test]
    fn several_reminders_arrive_oldest_first() {
        let mut store = store();
        let id = with_reminder(&mut store, 30);
        store
            .task_mut(&id)
            .unwrap()
            .reminders
            .push(Reminder::before_due(60));

        let mut schedule = Schedule::new();
        let due = schedule.take_due(&store, at(2026, 7, 31, 9, 0), &zone());
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].at, at(2026, 7, 31, 8, 0));
        assert_eq!(due[1].at, at(2026, 7, 31, 8, 30));
    }
}

//! When a task is due.
//!
//! A due date is a *date*, optionally with a clock time, optionally repeating.
//! It is never stored as an instant. "Friday at 09:00" means nine o'clock
//! wherever you are on Friday — if it were a `DateTime<Utc>` it would drift an
//! hour every time the clocks change and land at 08:00 for half the year.
//! Instants belong to reminders, which genuinely fire at a moment in time.

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use super::recurrence::Recurrence;

/// A task's due date, and the rule that produces the next one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Due {
    pub date: NaiveDate,
    /// The clock time, if the user gave one. A task due "Friday" with no time
    /// is not due at midnight; it is due some time on Friday, and sorting and
    /// notification both need to tell those apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<NaiveTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Recurrence>,
}

impl Due {
    /// Due on a date, at no particular time.
    pub fn on(date: NaiveDate) -> Self {
        Self {
            date,
            time: None,
            recurrence: None,
        }
    }

    /// Due at a specific time on a date.
    pub fn at(date: NaiveDate, time: NaiveTime) -> Self {
        Self {
            date,
            time: Some(time),
            recurrence: None,
        }
    }

    /// The same due date, repeating.
    pub fn repeating(mut self, recurrence: Recurrence) -> Self {
        self.recurrence = Some(recurrence);
        self
    }

    /// Whether this task repeats.
    pub fn is_recurring(&self) -> bool {
        self.recurrence.is_some()
    }

    /// Whether the date has passed, relative to `today`.
    ///
    /// Overdue is measured in whole days and ignores the clock time. A task
    /// due at 09:00 is not "overdue" at 09:01 — it is due today, and showing
    /// it in red from mid-morning would make the overdue list meaningless.
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        self.date < today
    }

    /// Advance a completed recurring task to its next occurrence.
    ///
    /// `None` means the task is finished for good: either it never repeated,
    /// or the rule has reached its end. The clock time carries across
    /// untouched — a recurrence steps dates, never times.
    pub fn advance(&self, completed_on: NaiveDate) -> Option<Self> {
        let recurrence = self.recurrence.as_ref()?;
        let (date, recurrence) = recurrence.advance(self.date, completed_on)?;
        Some(Self {
            date,
            time: self.time,
            recurrence: Some(recurrence),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recurrence::{End, Unit};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn nine_am() -> NaiveTime {
        NaiveTime::from_hms_opt(9, 0, 0).unwrap()
    }

    #[test]
    fn overdue_is_measured_in_days_not_minutes() {
        let due = Due::at(date(2026, 7, 30), nine_am());
        assert!(!due.is_overdue(date(2026, 7, 30)));
        assert!(due.is_overdue(date(2026, 7, 31)));
    }

    #[test]
    fn advancing_keeps_the_time_of_day() {
        let due = Due::at(date(2026, 7, 30), nine_am()).repeating(Recurrence::every(1, Unit::Week));
        let next = due.advance(date(2026, 7, 30)).unwrap();
        assert_eq!(next.date, date(2026, 8, 6));
        assert_eq!(next.time, Some(nine_am()));
    }

    #[test]
    fn a_task_that_does_not_repeat_has_nothing_to_advance_to() {
        let due = Due::on(date(2026, 7, 30));
        assert_eq!(due.advance(date(2026, 7, 30)), None);
    }

    #[test]
    fn the_last_occurrence_of_a_limited_rule_ends_the_task() {
        let due = Due::on(date(2026, 7, 30)).repeating(Recurrence {
            end: End::After { remaining: 0 },
            ..Recurrence::every(1, Unit::Day)
        });
        assert_eq!(due.advance(date(2026, 7, 30)), None);
    }

    #[test]
    fn the_countdown_carries_through_the_advanced_due_date() {
        let due = Due::on(date(2026, 7, 30)).repeating(Recurrence {
            end: End::After { remaining: 2 },
            ..Recurrence::every(1, Unit::Day)
        });
        let next = due.advance(date(2026, 7, 30)).unwrap();
        assert_eq!(
            next.recurrence.as_ref().unwrap().end,
            End::After { remaining: 1 }
        );

        let last = next.advance(next.date).unwrap();
        assert_eq!(last.date, date(2026, 8, 1));
        assert_eq!(last.advance(last.date), None);
    }

    #[test]
    fn a_plain_date_serialises_without_empty_fields() {
        let json = serde_json::to_string(&Due::on(date(2026, 7, 30))).unwrap();
        assert_eq!(json, r#"{"date":"2026-07-30"}"#);
    }
}

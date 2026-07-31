//! The GObject a task row binds to.
//!
//! `Task` is a plain serde record and cannot carry GObject properties, so the
//! list view gets this instead: a flat, already-formatted view of one task.
//! Everything a row displays is a property here, so binding a recycled row to
//! a different item needs no code beyond swapping the bindings.
//!
//! It is a *projection*, not a second source of truth. Nothing writes back
//! through it; the store stays canonical and a refresh rebuilds these from it.
//! Formatting a due date into a string here rather than in the row also means
//! "does 3 August render as `3 Aug`?" is a headless unit test.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use chrono::{Datelike, NaiveDate};

use crate::model::store::Store;
use crate::model::task::Task;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::TaskObject)]
    pub struct TaskObject {
        #[property(get, set, construct_only)]
        pub id: RefCell<String>,
        #[property(get, set)]
        pub content: RefCell<String>,
        #[property(get, set)]
        pub checked: Cell<bool>,

        /// The CSS class for the priority flag, empty when unset.
        #[property(get, set)]
        pub priority_class: RefCell<String>,
        #[property(get, set)]
        pub has_priority: Cell<bool>,

        #[property(get, set)]
        pub due_label: RefCell<String>,
        #[property(get, set)]
        pub due_class: RefCell<String>,
        #[property(get, set)]
        pub has_due: Cell<bool>,

        #[property(get, set)]
        pub deadline_label: RefCell<String>,
        #[property(get, set)]
        pub deadline_past: Cell<bool>,
        #[property(get, set)]
        pub has_deadline: Cell<bool>,

        #[property(get, set)]
        pub recurring: Cell<bool>,
        /// The repeat rule in words, for the icon's tooltip. The icon says
        /// *that* a task repeats and there is no room on a row to say more,
        /// but hovering it should answer *how* without opening the task.
        #[property(get, set)]
        pub repeat_label: RefCell<String>,
        /// Label names, one per entry. The row turns each into its own chip,
        /// so this stays a list rather than a joined string — `", "` is a
        /// separator the row would have to guess back out again, and a label
        /// name is free text that can contain one.
        #[property(get, set)]
        pub labels: RefCell<glib::StrV>,
        #[property(get, set)]
        pub has_labels: Cell<bool>,
        /// "2 of 5" for a task with subtasks, empty for one without.
        #[property(get, set)]
        pub subtasks: RefCell<String>,
        #[property(get, set)]
        pub has_subtasks: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TaskObject {
        const NAME: &'static str = "PlannerTaskObject";
        type Type = super::TaskObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for TaskObject {}
}

glib::wrapper! {
    pub struct TaskObject(ObjectSubclass<imp::TaskObject>);
}

impl TaskObject {
    /// Project a task into the shape a row displays.
    pub fn from_task(task: &Task, store: &Store, today: NaiveDate) -> Self {
        let object: Self = glib::Object::builder()
            .property("id", task.id.as_str())
            .build();
        object.update(task, store, today);
        object
    }

    /// Refresh every property from the task.
    ///
    /// Setting through the generated setters is what notifies, and a set to
    /// the same value still notifies — harmless for a handful of properties,
    /// and much easier to reason about than a per-field comparison.
    pub fn update(&self, task: &Task, store: &Store, today: NaiveDate) {
        self.set_content(task.content.clone());
        self.set_checked(task.checked);

        self.set_has_priority(task.priority.is_set());
        self.set_priority_class(task.priority.css_class().unwrap_or_default());

        match &task.due {
            Some(due) => {
                let (label, class) = format_due(due.date, due.time, today);
                self.set_has_due(true);
                self.set_due_label(label);
                self.set_due_class(class);
                self.set_recurring(due.is_recurring());
                self.set_repeat_label(
                    due.recurrence
                        .as_ref()
                        .map(|rule| capitalise(&rule.describe()))
                        .unwrap_or_default(),
                );
            }
            None => {
                self.set_has_due(false);
                self.set_due_label("");
                self.set_due_class("");
                self.set_recurring(false);
                self.set_repeat_label("");
            }
        }

        match task.deadline {
            Some(deadline) => {
                self.set_has_deadline(true);
                self.set_deadline_label(format!("Due {}", format_date(deadline, today)));
                self.set_deadline_past(task.is_past_deadline(today));
            }
            None => {
                self.set_has_deadline(false);
                self.set_deadline_label("");
                self.set_deadline_past(false);
            }
        }

        let names: Vec<&str> = task
            .labels
            .iter()
            .filter_map(|id| store.label(id))
            .map(|label| label.name.as_str())
            .collect();
        self.set_has_labels(!names.is_empty());
        self.set_labels(glib::StrV::from_iter(
            names.into_iter().map(glib::GString::from),
        ));

        let subtasks = store.subtasks(&task.id);
        let done = subtasks.iter().filter(|child| child.checked).count();
        self.set_has_subtasks(!subtasks.is_empty());
        self.set_subtasks(if subtasks.is_empty() {
            String::new()
        } else {
            format!("{done} of {}", subtasks.len())
        });
    }

    /// The task this stands for.
    pub fn task_id(&self) -> crate::model::TaskId {
        crate::model::TaskId::from_raw(self.id())
    }
}

/// How a due date reads on a row, and the class it takes.
///
/// Relative words for the days either side of today, a weekday name for the
/// coming week, and a date beyond that. Anyone can work out what "Fri" means;
/// "in 2 days" makes you count.
pub fn format_due(
    date: NaiveDate,
    time: Option<chrono::NaiveTime>,
    today: NaiveDate,
) -> (String, String) {
    let class = if date < today {
        "overdue"
    } else if date == today {
        "today"
    } else {
        ""
    };

    let mut label = format_date(date, today);
    if let Some(time) = time {
        label.push(' ');
        label.push_str(&time.format("%H:%M").to_string());
    }
    (label, class.to_string())
}

/// A date on its own, without the time.
/// `describe` writes a fragment, lower case, because it usually follows
/// something. A tooltip is a sentence on its own and wants a capital.
fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn format_date(date: NaiveDate, today: NaiveDate) -> String {
    let days = (date - today).num_days();
    match days {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        -1 => "Yesterday".to_string(),
        // Within the coming week a weekday name is unambiguous and shorter.
        2..=6 => date.format("%a").to_string(),
        _ if date.year() == today.year() => date.format("%-d %b").to_string(),
        _ => date.format("%-d %b %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    /// Thursday, 30 July 2026.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_days_either_side_of_today_are_named() {
        assert_eq!(format_date(today(), today()), "Today");
        assert_eq!(format_date(date(2026, 7, 31), today()), "Tomorrow");
        assert_eq!(format_date(date(2026, 7, 29), today()), "Yesterday");
    }

    #[test]
    fn the_coming_week_is_a_weekday_name() {
        assert_eq!(format_date(date(2026, 8, 1), today()), "Sat");
        assert_eq!(format_date(date(2026, 8, 5), today()), "Wed");
    }

    #[test]
    fn beyond_a_week_is_a_date() {
        assert_eq!(format_date(date(2026, 8, 20), today()), "20 Aug");
    }

    #[test]
    fn another_year_says_which() {
        assert_eq!(format_date(date(2027, 1, 3), today()), "3 Jan 2027");
        assert_eq!(format_date(date(2025, 1, 3), today()), "3 Jan 2025");
    }

    #[test]
    fn a_time_is_appended_when_there_is_one() {
        let (label, _) = format_due(today(), NaiveTime::from_hms_opt(9, 0, 0), today());
        assert_eq!(label, "Today 09:00");
        let (label, _) = format_due(today(), None, today());
        assert_eq!(label, "Today");
    }

    #[test]
    fn the_class_marks_overdue_and_today_and_nothing_else() {
        assert_eq!(format_due(date(2026, 7, 1), None, today()).1, "overdue");
        assert_eq!(format_due(today(), None, today()).1, "today");
        assert_eq!(format_due(date(2026, 8, 20), None, today()).1, "");
    }
}

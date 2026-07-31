//! Choosing a date, four ways at once.
//!
//! Quick buttons for the answers people actually give ("today", "tomorrow"),
//! a calendar for the ones they do not, a time field, and a text box that
//! takes the same natural language quick-add does. The last one is nearly
//! free: [`parse_date`] and [`parse_time`] are already written and already
//! tested, and having "next friday" work in the picker as well as in the entry
//! is the difference between a syntax and a trick.
//!
//! The picker never sets a date on anything. It reports one, and the panel
//! decides what that means for the task — which matters most for recurrence:
//! moving a repeating task to Thursday must not quietly stop it repeating, so
//! the existing rule is carried through rather than being the picker's to lose.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};

use crate::model::due::Due;
use crate::model::parse::{parse_date, parse_recurrence, parse_time};
use crate::model::recurrence::Recurrence;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    type Callback = Box<dyn Fn(Option<Due>)>;

    #[derive(Default)]
    pub struct DatePicker {
        pub calendar: RefCell<Option<gtk::Calendar>>,
        pub natural: RefCell<Option<gtk::Entry>>,
        pub time: RefCell<Option<gtk::Entry>>,
        pub feedback: RefCell<Option<gtk::Label>>,
        pub repeat: RefCell<Option<gtk::Entry>>,
        pub repeat_row: RefCell<Option<gtk::Box>>,
        pub repeat_feedback: RefCell<Option<gtk::Label>>,
        /// Cleared by [`hide_repeat`](super::DatePicker::hide_repeat) for the
        /// deadline picker, which has no repeat to edit.
        pub repeat_enabled: Cell<bool>,

        pub today: Cell<Option<NaiveDate>>,
        /// The rule the task has, as the box was last committed.
        pub recurrence: RefCell<Option<Recurrence>>,
        /// Whether the task had a date of its own. The calendar always shows
        /// one, so it cannot answer this, and setting a repeat on a task with
        /// no date has to pick a first occurrence rather than use today.
        pub has_date: Cell<bool>,
        pub chosen: RefCell<Option<Callback>>,
        /// Set while the picker writes its own widgets, so the resulting
        /// `day-selected` is not treated as the user choosing a date.
        pub loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DatePicker {
        const NAME: &'static str = "PlannerDatePicker";
        type Type = super::DatePicker;
        type ParentType = gtk::Popover;
    }

    impl ObjectImpl for DatePicker {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for DatePicker {}
    impl PopoverImpl for DatePicker {}
}

glib::wrapper! {
    pub struct DatePicker(ObjectSubclass<imp::DatePicker>)
        @extends gtk::Popover, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
                    gtk::ShortcutManager;
}

impl Default for DatePicker {
    fn default() -> Self {
        Self::new()
    }
}

impl DatePicker {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Called with the chosen date, or `None` when the date is cleared.
    ///
    /// A Rust callback rather than a GObject signal: `Option<Due>` has no
    /// `glib::Value` representation short of boxing it, and both ends of this
    /// wire are Rust. See [`crate::ui::detail_panel`] for the same reasoning
    /// at more length.
    pub fn connect_chosen(&self, callback: impl Fn(Option<Due>) + 'static) {
        self.imp().chosen.replace(Some(Box::new(callback)));
    }

    /// Drop the repeat box. A deadline is a date something is due *by*; it
    /// does not repeat, and offering the control would imply it could.
    pub fn hide_repeat(&self) {
        self.imp().repeat_enabled.set(false);
        if let Some(row) = self.imp().repeat_row.borrow().as_ref() {
            row.set_visible(false);
        }
        self.set_repeat_feedback("");
    }

    /// Show the picker for a task's current date.
    pub fn load(&self, due: Option<&Due>, today: NaiveDate) {
        let imp = self.imp();
        imp.loading.set(true);
        imp.today.set(Some(today));
        let recurrence = due.and_then(|due| due.recurrence.clone());
        imp.recurrence.replace(recurrence.clone());
        imp.has_date.set(due.is_some());

        let date = due.map(|due| due.date).unwrap_or(today);
        if let Some(calendar) = imp.calendar.borrow().as_ref() {
            // Year and month before day: `select_day` is deprecated since
            // 4.20, and setting the day first would clamp it against whatever
            // month happened to be showing (a 31st against February).
            calendar.set_year(date.year());
            calendar.set_month(date.month0() as i32);
            calendar.set_day(date.day() as i32);
        }
        if let Some(entry) = imp.time.borrow().as_ref() {
            entry.set_text(
                &due.and_then(|due| due.time)
                    .map(|time| time.format("%H:%M").to_string())
                    .unwrap_or_default(),
            );
        }
        if let Some(entry) = imp.natural.borrow().as_ref() {
            entry.set_text("");
        }
        // Prefilled with the rule rather than left blank: `describe` writes
        // the phrase that would have produced it, so the box shows what the
        // task does and editing it is editing that phrase.
        if let Some(entry) = imp.repeat.borrow().as_ref() {
            entry.set_text(&recurrence.map(|rule| rule.describe()).unwrap_or_default());
        }
        self.set_feedback("");
        self.set_repeat_feedback("");
        imp.loading.set(false);
    }

    fn build(&self) {
        let imp = self.imp();

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(8)
            .margin_end(8)
            .build();

        let natural = gtk::Entry::builder()
            .placeholder_text("next friday, in 3 days…")
            .build();
        natural.connect_activate(glib::clone!(
            #[weak(rename_to = picker)]
            self,
            move |_| picker.commit_natural()
        ));
        natural.connect_changed(glib::clone!(
            #[weak(rename_to = picker)]
            self,
            move |_| picker.preview_natural()
        ));
        body.append(&natural);

        let feedback = gtk::Label::builder().xalign(0.0).build();
        feedback.add_css_class("caption");
        feedback.add_css_class("dimmed");
        feedback.set_visible(false);
        body.append(&feedback);

        // The four answers that cover most of the traffic, so the common case
        // is one click rather than a hunt through a calendar.
        let quick = adw::WrapBox::builder()
            .child_spacing(6)
            .line_spacing(6)
            .build();
        for (label, offset) in [
            ("Today", Some(0)),
            ("Tomorrow", Some(1)),
            ("Next week", Some(7)),
        ] {
            let button = gtk::Button::with_label(label);
            button.add_css_class("pill");
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = picker)]
                self,
                move |_| picker.choose_offset(offset.expect("a fixed offset"))
            ));
            quick.append(&button);
        }
        let clear = gtk::Button::with_label("No date");
        clear.add_css_class("pill");
        clear.connect_clicked(glib::clone!(
            #[weak(rename_to = picker)]
            self,
            move |_| picker.clear_date()
        ));
        quick.append(&clear);
        body.append(&quick);

        let calendar = gtk::Calendar::new();
        calendar.connect_day_selected(glib::clone!(
            #[weak(rename_to = picker)]
            self,
            move |_| picker.commit_calendar()
        ));
        body.append(&calendar);

        let time_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        let time_label = gtk::Label::new(Some("Time"));
        time_label.add_css_class("dimmed");
        time_row.append(&time_label);
        let time = gtk::Entry::builder()
            .placeholder_text("09:00")
            .max_width_chars(8)
            .hexpand(true)
            .build();
        time.connect_activate(glib::clone!(
            #[weak(rename_to = picker)]
            self,
            move |_| picker.commit_calendar()
        ));
        time_row.append(&time);
        body.append(&time_row);

        // --- repeat -----------------------------------------------------
        // A text box rather than a bank of spinners and toggles, for the
        // reason `parse::recurrence` gives: `every!` — repeat from when you
        // finished, not from when it was due — has no obvious widget, and a
        // control set that cannot express it would be a worse editor than the
        // syntax it replaced. The phrase already parses, and `describe` writes
        // it back, so the box round-trips.
        let repeat_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        let repeat_label = gtk::Label::new(Some("Repeat"));
        repeat_label.add_css_class("dimmed");
        repeat_row.append(&repeat_label);
        let repeat = gtk::Entry::builder()
            .placeholder_text("every week, every! 10 days…")
            .hexpand(true)
            .build();
        repeat.connect_activate(glib::clone!(
            #[weak(rename_to = picker)]
            self,
            move |_| picker.commit_repeat()
        ));
        repeat.connect_changed(glib::clone!(
            #[weak(rename_to = picker)]
            self,
            move |_| picker.preview_repeat()
        ));
        repeat_row.append(&repeat);
        let never = gtk::Button::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text("Do not repeat")
            .css_classes(["flat"])
            .build();
        never.connect_clicked(glib::clone!(
            #[weak(rename_to = picker)]
            self,
            move |_| picker.set_repeat(None)
        ));
        repeat_row.append(&never);
        body.append(&repeat_row);
        imp.repeat_enabled.set(true);

        let repeat_feedback = gtk::Label::builder().xalign(0.0).build();
        repeat_feedback.add_css_class("caption");
        repeat_feedback.add_css_class("dimmed");
        repeat_feedback.set_visible(false);
        body.append(&repeat_feedback);

        self.set_child(Some(&body));

        imp.calendar.replace(Some(calendar));
        imp.natural.replace(Some(natural));
        imp.time.replace(Some(time));
        imp.feedback.replace(Some(feedback));
        imp.repeat.replace(Some(repeat));
        imp.repeat_row.replace(Some(repeat_row));
        imp.repeat_feedback.replace(Some(repeat_feedback));
    }

    fn today(&self) -> NaiveDate {
        self.imp().today.get().unwrap_or_else(crate::ui::today)
    }

    /// Show what the typed phrase would mean, without committing to it.
    fn preview_natural(&self) {
        let Some(entry) = self.imp().natural.borrow().clone() else {
            return;
        };
        let text = entry.text().to_string();
        if text.trim().is_empty() {
            self.set_feedback("");
            return;
        }
        match parse_date(&text, self.today()) {
            Some(date) => {
                self.set_feedback(&crate::ui::task_object::format_date(date, self.today()))
            }
            None => self.set_feedback("Not a date"),
        }
    }

    fn set_feedback(&self, text: &str) {
        if let Some(label) = self.imp().feedback.borrow().as_ref() {
            label.set_label(text);
            label.set_visible(!text.is_empty());
        }
    }

    fn commit_natural(&self) {
        let Some(entry) = self.imp().natural.borrow().clone() else {
            return;
        };
        let text = entry.text().to_string();
        let Some(date) = parse_date(&text, self.today()) else {
            return;
        };
        self.choose(date);
    }

    /// Report a specific date, keeping the time field and any repeat rule.
    ///
    /// Public because it is the picker's whole job: the quick buttons, the
    /// calendar and the text box all funnel through here.
    pub fn choose(&self, date: NaiveDate) {
        self.report(Some(self.build_due(date)));
    }

    /// Report that the task should have no date at all.
    pub fn clear_date(&self) {
        self.report(None);
    }

    fn choose_offset(&self, days: u64) {
        let Some(date) = self.today().checked_add_days(chrono::Days::new(days)) else {
            return;
        };
        self.choose(date);
    }

    fn commit_calendar(&self) {
        if self.imp().loading.get() {
            return;
        }
        let Some(date) = self.selected_date() else {
            return;
        };
        self.choose(date);
    }

    /// The day the calendar is showing.
    fn selected_date(&self) -> Option<NaiveDate> {
        let calendar = self.imp().calendar.borrow().clone()?;
        let selected = calendar.date();
        NaiveDate::from_ymd_opt(
            selected.year(),
            selected.month() as u32,
            selected.day_of_month() as u32,
        )
    }

    /// Show what the typed repeat phrase would mean, without committing.
    fn preview_repeat(&self) {
        if !self.imp().repeat_enabled.get() {
            return;
        }
        let Some(entry) = self.imp().repeat.borrow().clone() else {
            return;
        };
        let text = entry.text().to_string();
        let message = if text.trim().is_empty() {
            String::new()
        } else {
            match parse_recurrence(&text, self.today()) {
                Some(rule) => rule.describe(),
                None => "Not a repeat".to_string(),
            }
        };
        self.set_repeat_feedback(&message);
    }

    fn set_repeat_feedback(&self, text: &str) {
        if let Some(label) = self.imp().repeat_feedback.borrow().as_ref() {
            label.set_label(text);
            label.set_visible(!text.is_empty());
        }
    }

    /// Read the repeat box and report the task with that rule on it. An empty
    /// box means "stop repeating"; a phrase that will not parse commits
    /// nothing rather than quietly dropping the rule the task already had.
    /// Type a phrase into the repeat box and commit it, as pressing Enter in
    /// it does. The other half of the picker's job, alongside [`choose`]:
    /// public for the same reason, and what the widget tests drive.
    ///
    /// [`choose`]: Self::choose
    pub fn commit_repeat_text(&self, text: &str) {
        if let Some(entry) = self.imp().repeat.borrow().as_ref() {
            entry.set_text(text);
        }
        self.commit_repeat();
    }

    /// What the repeat box is showing.
    pub fn repeat_text(&self) -> String {
        self.imp()
            .repeat
            .borrow()
            .as_ref()
            .map(|entry| entry.text().to_string())
            .unwrap_or_default()
    }

    fn commit_repeat(&self) {
        if !self.imp().repeat_enabled.get() {
            return;
        }
        let Some(entry) = self.imp().repeat.borrow().clone() else {
            return;
        };
        let text = entry.text().to_string();
        if text.trim().is_empty() {
            self.set_repeat(None);
            return;
        }
        match parse_recurrence(&text, self.today()) {
            Some(rule) => self.set_repeat(Some(rule)),
            None => self.set_repeat_feedback("Not a repeat"),
        }
    }

    /// Report the task with `rule` on it, keeping its date and time.
    ///
    /// A recurrence lives inside a `Due`, so a rule needs a date to hang on. A
    /// task that has none takes the rule's first occurrence — "every monday"
    /// on a Thursday means the coming Monday, which is the only thing it could
    /// mean and is better than silently landing today.
    fn set_repeat(&self, rule: Option<Recurrence>) {
        let imp = self.imp();
        imp.recurrence.replace(rule.clone());

        // No date and no rule leaves nothing to report: a repeat cannot exist
        // without a date, so the task had none either.
        let date = match (imp.has_date.get(), &rule) {
            (true, _) => self.selected_date().unwrap_or_else(|| self.today()),
            (false, Some(rule)) => rule.first_occurrence(self.today()),
            (false, None) => return self.report(None),
        };
        self.report(Some(self.build_due(date)));
    }

    /// Assemble a due date from a chosen day, the time field, and the rule the
    /// task already had.
    fn build_due(&self, date: NaiveDate) -> Due {
        let time = self
            .imp()
            .time
            .borrow()
            .as_ref()
            .map(|entry| entry.text().to_string())
            .and_then(|text| parse_time(&text));

        Due {
            date,
            time,
            recurrence: self.imp().recurrence.borrow().clone(),
        }
    }

    fn report(&self, due: Option<Due>) {
        if let Some(callback) = self.imp().chosen.borrow().as_ref() {
            callback(due);
        }
        self.popdown();
    }
}

/// How a due date reads on the row that opens this picker.
///
/// Separate from [`format_date`](crate::ui::task_object::format_date) because
/// a row has room to say more: the repeat and the time both belong here, where
/// on a task row they are a separate icon and a suffix.
pub fn describe_due(due: Option<&Due>, today: NaiveDate) -> String {
    let Some(due) = due else {
        return "No date".to_string();
    };
    let mut text = crate::ui::task_object::format_date(due.date, today);
    if let Some(time) = due.time {
        text.push_str(&format!(" at {}", time.format("%H:%M")));
    }
    if let Some(rule) = due.recurrence.as_ref() {
        // The rule itself, not the fact that there is one: "repeats" answers a
        // question nobody asks at the moment they are looking for whether this
        // is the daily one or the Monday one.
        text.push_str(&format!(" · {}", rule.describe()));
    }
    text
}

/// The same, for a deadline, which has no time and never repeats.
pub fn describe_deadline(deadline: Option<NaiveDate>, today: NaiveDate) -> String {
    match deadline {
        Some(date) => crate::ui::task_object::format_date(date, today),
        None => "No deadline".to_string(),
    }
}

/// Whether a time entry holds something usable, for enabling the control.
pub fn is_valid_time(text: &str) -> bool {
    text.trim().is_empty() || parse_time(text).is_some()
}

/// Round a time to the minute. `NaiveTime` carries seconds this app never
/// shows, and keeping them would make two visually identical times unequal.
pub fn to_minute(time: NaiveTime) -> NaiveTime {
    NaiveTime::from_hms_opt(time.hour(), time.minute(), 0).unwrap_or(time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::recurrence::Unit;

    /// Thursday, 30 July 2026.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn no_date_says_so_rather_than_showing_a_blank() {
        assert_eq!(describe_due(None, today()), "No date");
        assert_eq!(describe_deadline(None, today()), "No deadline");
    }

    #[test]
    fn a_due_date_reads_with_its_time_and_repeat() {
        let due = Due::at(today(), NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        assert_eq!(describe_due(Some(&due), today()), "Today at 09:00");

        // The rule itself, so the row distinguishes the weekly one from the
        // daily one without being opened.
        let due = Due::on(date(2026, 8, 3)).repeating(Recurrence::every(1, Unit::Week));
        assert_eq!(describe_due(Some(&due), today()), "Mon · every week");

        let due = Due::on(date(2026, 8, 3)).repeating(Recurrence::every_weekday());
        assert_eq!(describe_due(Some(&due), today()), "Mon · every weekday");
    }

    #[test]
    fn a_deadline_has_no_time_and_no_repeat_to_show() {
        assert_eq!(
            describe_deadline(Some(date(2026, 8, 20)), today()),
            "20 Aug"
        );
    }

    #[test]
    fn an_empty_time_field_is_valid_because_a_date_need_not_have_one() {
        assert!(is_valid_time(""));
        assert!(is_valid_time("   "));
        assert!(is_valid_time("9am"));
        assert!(is_valid_time("17:30"));
        assert!(!is_valid_time("lunchtime"));
        assert!(!is_valid_time("25:00"));
    }

    #[test]
    fn seconds_are_dropped_so_two_equal_looking_times_are_equal() {
        let messy = NaiveTime::from_hms_opt(9, 30, 45).unwrap();
        assert_eq!(to_minute(messy), NaiveTime::from_hms_opt(9, 30, 0).unwrap());
    }
}

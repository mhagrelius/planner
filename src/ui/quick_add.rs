//! Quick add: one entry, parsed as you type.
//!
//! The parser already does the work ([`parse_quick_add`]); this is the part
//! that has to convince the user it did. Two things carry that:
//!
//! **The tokens are highlighted in place.** `friday` turning accent-coloured
//! as you finish typing it is the feedback that stops the syntax feeling like
//! a gamble. It uses the byte spans the parser returns, which is the reason it
//! returns them at all.
//!
//! **The chips say what was understood.** Colour alone would be a poor way to
//! tell a date from a label — it fails for anyone who cannot distinguish the
//! hues, and it fails for everyone when the guess is wrong. So highlighting
//! says only "this was understood" in one uniform colour, and the chips
//! underneath say what it was understood *as*.
//!
//! An `AdwDialog`, so it is a centred dialog on a desktop and a bottom sheet
//! on a narrow screen without this file knowing which.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use gtk::pango;

use chrono::NaiveDate;

use crate::model::parse::quick_add::{QuickAdd, SpanKind};
use crate::model::parse::{parse_quick_add, Vocabulary};
use crate::ui::task_object::format_date;

/// How long to wait after a keystroke before re-parsing.
///
/// Short enough to feel immediate, long enough that a fast typist is not
/// re-parsing and re-rendering chips on every letter of a long sentence.
const PARSE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(120);

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct QuickAddDialog {
        pub entry: RefCell<Option<gtk::Entry>>,
        pub chips: RefCell<Option<adw::WrapBox>>,
        pub hint: RefCell<Option<gtk::Label>>,
        pub add: RefCell<Option<gtk::Button>>,
        pub keep_adding: RefCell<Option<gtk::CheckButton>>,

        pub vocabulary: RefCell<Vocabulary>,
        pub today: Cell<Option<NaiveDate>>,
        pub parsed: RefCell<QuickAdd>,
        pub debounce: RefCell<Option<glib::SourceId>>,
        /// Where the task goes when the line does not say, and what to call it
        /// on the default chip.
        pub default_project: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for QuickAddDialog {
        const NAME: &'static str = "PlannerQuickAddDialog";
        type Type = super::QuickAddDialog;
        type ParentType = adw::Dialog;
    }

    impl ObjectImpl for QuickAddDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            // The timer holds a weak reference, but leaving it running past
            // the dialog is still a source firing into nothing every 120ms.
            if let Some(id) = self.debounce.take() {
                id.remove();
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // The line the user committed. The window parses and files
                    // it — this dialog never touches the store.
                    Signal::builder("submitted")
                        .param_types([str::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for QuickAddDialog {}
    impl AdwDialogImpl for QuickAddDialog {}
}

glib::wrapper! {
    pub struct QuickAddDialog(ObjectSubclass<imp::QuickAddDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for QuickAddDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl QuickAddDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Tell the dialog the names it should recognise, and where a task goes
    /// when the line does not say.
    pub fn prepare(&self, vocabulary: Vocabulary, today: NaiveDate, default_project: &str) {
        let imp = self.imp();
        imp.vocabulary.replace(vocabulary);
        imp.today.set(Some(today));
        imp.default_project.replace(default_project.to_string());
        self.reparse();
    }

    fn build(&self) {
        let imp = self.imp();
        self.set_title("New Task");
        self.set_content_width(460);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let entry = gtk::Entry::builder()
            .placeholder_text("Email Sam #Work @email p2 friday 9am")
            .activates_default(true)
            .hexpand(true)
            .build();
        entry.connect_changed(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.schedule_reparse()
        ));
        body.append(&entry);

        // Where the parse is reported back. Wraps, because a line with a
        // project, a date and three labels will not fit on one.
        let chips = adw::WrapBox::builder()
            .child_spacing(6)
            .line_spacing(6)
            .build();
        let chips_host = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .build();
        chips_host.append(&chips);
        body.append(&chips_host);

        let hint = gtk::Label::builder()
            .label(HINT)
            .xalign(0.0)
            .wrap(true)
            .build();
        hint.add_css_class("caption");
        hint.add_css_class("dimmed");
        body.append(&hint);

        let keep_adding = gtk::CheckButton::builder()
            .label("Keep adding")
            .tooltip_text("Stay open after adding, for a run of tasks")
            .build();

        let cancel = gtk::Button::builder().label("Cancel").build();
        cancel.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.close();
            }
        ));

        let add = gtk::Button::builder()
            .label("Add")
            .sensitive(false)
            .css_classes(["suggested-action"])
            .build();
        add.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.submit()
        ));

        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        buttons.append(&keep_adding);
        let spacer = gtk::Box::builder().hexpand(true).build();
        buttons.append(&spacer);
        buttons.append(&cancel);
        buttons.append(&add);
        body.append(&buttons);

        let toolbar = adw::ToolbarView::builder().content(&body).build();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        self.set_child(Some(&toolbar));

        // Return submits, Escape closes. `activates-default` on the entry
        // needs a default widget to activate.
        self.set_default_widget(Some(&add));
        self.set_focus(Some(&entry));

        imp.entry.replace(Some(entry));
        imp.chips.replace(Some(chips));
        imp.hint.replace(Some(hint));
        imp.add.replace(Some(add));
        imp.keep_adding.replace(Some(keep_adding));

        self.install_shortcuts();
    }

    /// `Ctrl+K` toggles keep-adding, matching the tooltip.
    fn install_shortcuts(&self) {
        let controller = gtk::ShortcutController::new();
        controller.set_scope(gtk::ShortcutScope::Managed);
        controller.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>k"),
            Some(gtk::CallbackAction::new(|widget, _| {
                let dialog = widget
                    .downcast_ref::<QuickAddDialog>()
                    .expect("installed on the dialog");
                if let Some(keep) = dialog.imp().keep_adding.borrow().as_ref() {
                    keep.set_active(!keep.is_active());
                }
                glib::Propagation::Stop
            })),
        ));
        self.add_controller(controller);
    }

    /// Re-parse shortly, replacing any parse already pending.
    fn schedule_reparse(&self) {
        let imp = self.imp();
        if let Some(id) = imp.debounce.take() {
            id.remove();
        }
        let id = glib::timeout_add_local_once(
            PARSE_DEBOUNCE,
            glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move || {
                    dialog.imp().debounce.replace(None);
                    dialog.reparse();
                }
            ),
        );
        imp.debounce.replace(Some(id));
    }

    /// Parse what is in the entry and show what was understood.
    fn reparse(&self) {
        let imp = self.imp();
        let Some(entry) = imp.entry.borrow().clone() else {
            return;
        };
        let Some(today) = imp.today.get() else {
            return;
        };

        let text = entry.text().to_string();
        let parsed = {
            let vocabulary = imp.vocabulary.borrow();
            parse_quick_add(&text, today, &vocabulary)
        };

        entry.set_attributes(&highlight(&parsed));
        self.show_chips(&parsed);

        if let Some(add) = imp.add.borrow().as_ref() {
            // A line of nothing but tokens has no title, and a task with no
            // title is not a task.
            add.set_sensitive(!parsed.title.trim().is_empty());
        }
        imp.parsed.replace(parsed);
    }

    /// Rebuild the row of chips describing the parse.
    fn show_chips(&self, parsed: &QuickAdd) {
        let Some(chips) = self.imp().chips.borrow().clone() else {
            return;
        };
        chips.remove_all();

        let today = self.imp().today.get().unwrap_or_else(crate::ui::today);
        for (icon, text) in describe(parsed, today, &self.imp().default_project.borrow()) {
            chips.append(&build_chip(icon, &text));
        }

        // The hint is only useful before anything has been recognised; once
        // the chips are saying something they say it better.
        if let Some(hint) = self.imp().hint.borrow().as_ref() {
            hint.set_visible(chips.first_child().is_none());
        }
    }

    /// Hand the line to whoever is listening, and either close or clear.
    fn submit(&self) {
        let imp = self.imp();
        let Some(entry) = imp.entry.borrow().clone() else {
            return;
        };
        if imp.parsed.borrow().title.trim().is_empty() {
            return;
        }

        let text = entry.text().to_string();
        self.emit_by_name::<()>("submitted", &[&text]);

        let keep = imp
            .keep_adding
            .borrow()
            .as_ref()
            .is_some_and(|keep| keep.is_active());

        if keep {
            entry.set_text("");
            // A fresh parse clears the chips and disables Add; without it the
            // dialog would sit there claiming to have understood the task it
            // has just filed.
            self.reparse();
            self.set_focus(Some(&entry));
        } else {
            self.close();
        }
    }

    /// What the entry currently holds, for tests.
    pub fn text(&self) -> String {
        self.imp()
            .entry
            .borrow()
            .as_ref()
            .map(|entry| entry.text().to_string())
            .unwrap_or_default()
    }

    /// Set the entry text, for tests and for pre-filling.
    pub fn set_text(&self, text: &str) {
        if let Some(entry) = self.imp().entry.borrow().as_ref() {
            entry.set_text(text);
        }
        // Parse now rather than on the debounce: a caller that just set the
        // text expects to be able to ask what it means.
        self.reparse();
    }

    /// The most recent parse.
    pub fn parsed(&self) -> QuickAdd {
        self.imp().parsed.borrow().clone()
    }

    /// Whether the dialog will stay open after adding.
    pub fn keeps_adding(&self) -> bool {
        self.imp()
            .keep_adding
            .borrow()
            .as_ref()
            .is_some_and(|keep| keep.is_active())
    }

    pub fn set_keeps_adding(&self, keep_adding: bool) {
        if let Some(keep) = self.imp().keep_adding.borrow().as_ref() {
            keep.set_active(keep_adding);
        }
    }

    /// Whether Add would do anything.
    pub fn can_submit(&self) -> bool {
        self.imp()
            .add
            .borrow()
            .as_ref()
            .is_some_and(|add| add.is_sensitive())
    }
}

const HINT: &str = "#project  /section  @label  p1–p4  !30m  — and dates like \
                    “friday 9am”, “in 3 days”, “every other monday”";

/// Pango attributes marking every recognised token.
///
/// One colour for all of them. The kind is on the chips; using hue to encode
/// it here would be information available only to people who can tell the hues
/// apart, in a spot where being wrong is most costly.
pub fn highlight(parsed: &QuickAdd) -> pango::AttrList {
    let attributes = pango::AttrList::new();
    let accent = accent_rgba();

    for span in &parsed.spans {
        let start = span.range.start as u32;
        let end = span.range.end as u32;

        let mut colour = pango::AttrColor::new_foreground(
            (accent.red() * 65535.0) as u16,
            (accent.green() * 65535.0) as u16,
            (accent.blue() * 65535.0) as u16,
        );
        colour.set_start_index(start);
        colour.set_end_index(end);
        attributes.insert(colour);

        let mut weight = pango::AttrInt::new_weight(pango::Weight::Bold);
        weight.set_start_index(start);
        weight.set_end_index(end);
        attributes.insert(weight);
    }
    attributes
}

/// The accent colour, adjusted so it is legible as text on the current scheme.
fn accent_rgba() -> gtk::gdk::RGBA {
    let manager = adw::StyleManager::default();
    manager.accent_color().to_standalone_rgba(manager.is_dark())
}

/// One line per thing the parser understood, as (icon, text).
///
/// Pure, so what the chips claim is a headless test rather than something you
/// check by squinting at a screenshot.
pub fn describe(
    parsed: &QuickAdd,
    today: NaiveDate,
    default_project: &str,
) -> Vec<(&'static str, String)> {
    let mut chips = Vec::new();

    // The project chip shows even when it was not typed, because "where is
    // this going?" is the question quick-add is worst at answering.
    match parsed.project.as_deref() {
        Some(project) => chips.push(("folder-symbolic", project.to_string())),
        None if !default_project.is_empty() => {
            chips.push(("folder-symbolic", default_project.to_string()))
        }
        None => {}
    }
    if let Some(section) = parsed.section.as_deref() {
        chips.push(("view-list-symbolic", section.to_string()));
    }

    if let Some(due) = &parsed.due {
        let mut text = format_date(due.date, today);
        if let Some(time) = due.time {
            text.push(' ');
            text.push_str(&time.format("%H:%M").to_string());
        }
        chips.push(("x-office-calendar-symbolic", text));

        if due.is_recurring() {
            let repeats = parsed
                .spans
                .iter()
                .find(|span| span.kind == SpanKind::Recurrence)
                .map(|_| "Repeats".to_string())
                .unwrap_or_else(|| "Repeats".to_string());
            chips.push(("media-playlist-repeat-symbolic", repeats));
        }
    }

    if let Some(priority) = parsed.priority {
        if priority.is_set() {
            chips.push(("emblem-important-symbolic", priority.label().to_string()));
        }
    }

    for label in &parsed.labels {
        chips.push(("user-bookmarks-symbolic", label.clone()));
    }

    for minutes in &parsed.reminders {
        chips.push(("alarm-symbolic", format!("{} before", duration(*minutes))));
    }

    chips
}

/// "30 minutes", "2 hours", "1 day".
fn duration(minutes: i64) -> String {
    let plural = |count: i64, unit: &str| {
        if count == 1 {
            format!("1 {unit}")
        } else {
            format!("{count} {unit}s")
        }
    };
    match minutes {
        m if m % (60 * 24) == 0 => plural(m / (60 * 24), "day"),
        m if m % 60 == 0 => plural(m / 60, "hour"),
        m => plural(m, "minute"),
    }
}

fn build_chip(icon: &str, text: &str) -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .build();

    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(12);
    body.append(&image);

    let label = gtk::Label::new(Some(text));
    body.append(&label);

    body.add_css_class("parse-chip");
    body.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thursday, 30 July 2026.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
    }

    fn parse(line: &str) -> QuickAdd {
        parse_quick_add(line, today(), &Vocabulary::default())
    }

    fn texts(parsed: &QuickAdd, default_project: &str) -> Vec<String> {
        describe(parsed, today(), default_project)
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    }

    #[test]
    fn a_plain_line_still_says_where_the_task_is_going() {
        let chips = texts(&parse("Buy milk"), "Inbox");
        assert_eq!(chips, vec!["Inbox"]);
    }

    #[test]
    fn a_named_project_replaces_the_default_rather_than_joining_it() {
        let chips = texts(&parse("Buy milk #Home"), "Inbox");
        assert_eq!(chips, vec!["Home"]);
    }

    #[test]
    fn every_understood_token_gets_a_chip() {
        // Today is Thursday, so "friday" is tomorrow — and the chip says
        // "Tomorrow", because that is what the row will say too.
        let parsed = parse("Email Sam #Work @email p1 friday 9am !30m");
        assert_eq!(
            texts(&parsed, "Inbox"),
            vec![
                "Work",
                "Tomorrow 09:00",
                "Urgent",
                "email",
                "30 minutes before"
            ]
        );
    }

    #[test]
    fn a_date_further_out_reads_as_a_weekday_or_a_date() {
        let parsed = parse("Standup next friday");
        assert_eq!(texts(&parsed, "Inbox"), vec!["Inbox", "7 Aug"]);
    }

    #[test]
    fn a_repeat_is_called_out_separately_from_its_first_date() {
        let parsed = parse("Bins every monday");
        assert_eq!(texts(&parsed, "Inbox"), vec!["Inbox", "Mon", "Repeats"]);
    }

    #[test]
    fn an_unset_priority_is_not_worth_a_chip() {
        let parsed = parse("Whatever p4");
        assert_eq!(texts(&parsed, "Inbox"), vec!["Inbox"]);
    }

    #[test]
    fn reminders_are_described_in_the_largest_whole_unit() {
        assert_eq!(duration(30), "30 minutes");
        assert_eq!(duration(1), "1 minute");
        assert_eq!(duration(60), "1 hour");
        assert_eq!(duration(120), "2 hours");
        assert_eq!(duration(1440), "1 day");
        assert_eq!(duration(90), "90 minutes");
    }

    #[test]
    fn nothing_typed_means_nothing_claimed_except_the_destination() {
        let chips = texts(&parse(""), "Inbox");
        assert_eq!(chips, vec!["Inbox"]);
        let chips = texts(&parse(""), "");
        assert!(chips.is_empty());
    }
}

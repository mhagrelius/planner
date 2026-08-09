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
//! **The duplicate check lives here too.** Adding a task is the most frequent
//! thing anyone does in this app, and `window.rs` deliberately chose an undo
//! toast over a confirmation because asking first for something that frequent
//! would be intolerable. Nothing below changes that for the ordinary case: what
//! resembles the task being typed appears *underneath the entry, while it is
//! being typed*, in the same place the chips already report what was
//! understood, and Add stays one keystroke away. Only a near-certain match
//! stops to ask, and it is the one case where the toast is no help — undo
//! removes the task you just added, not the one you already had.
//!
//! An `AdwDialog`, so it is a centred dialog on a desktop and a bottom sheet
//! on a narrow screen without this file knowing which.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use gtk::pango;

use chrono::NaiveDate;

use crate::model::duplicate::{CheckError, Judgements, Verdict};
use crate::model::parse::quick_add::{QuickAdd, SpanKind};
use crate::model::parse::{parse_quick_add, Vocabulary};
use crate::model::similar::Candidate;
use crate::ui::task_object::format_date;

/// How long to wait after a keystroke before re-parsing.
///
/// Short enough to feel immediate, long enough that a fast typist is not
/// re-parsing and re-rendering chips on every letter of a long sentence.
const PARSE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(120);

/// How long to wait after a keystroke before asking the model.
///
/// Much longer than the parse debounce, and for a different reason: the parse
/// is free and this is a network round trip. Waiting until someone has stopped
/// typing for half a second is the difference between one request per task and
/// one per word.
const CHECK_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(600);

/// A title shorter than this is not worth asking about — "Buy" matches half the
/// list and means nothing yet.
const MIN_TITLE_FOR_CHECK: usize = 6;

/// How many similar tasks to show. More than a handful stops being a hint and
/// becomes a second task list.
const MAX_SHOWN: usize = 5;

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

        // --- the duplicate check ---------------------------------------
        pub similar_host: RefCell<Option<gtk::Box>>,
        pub similar_heading: RefCell<Option<gtk::Label>>,
        pub similar_list: RefCell<Option<gtk::ListBox>>,
        /// Shown while the model is being asked, so a pause is a pause and not
        /// a verdict of "nothing found".
        pub checking_row: RefCell<Option<gtk::Box>>,

        /// Reads the store. The dialog is not allowed to hold one — the window
        /// owns that — so it holds the question instead.
        #[allow(clippy::type_complexity)]
        pub candidate_source: RefCell<Option<Box<dyn Fn(&str) -> Vec<Candidate>>>>,
        /// Absent means the local word comparison only, which is the whole
        /// feature for anyone who has not configured a key.
        pub api_key: RefCell<Option<String>>,

        pub candidates: RefCell<Vec<Candidate>>,
        pub judgements: RefCell<Option<Judgements>>,
        pub check_debounce: RefCell<Option<glib::SourceId>>,
        /// Bumped per request. A reply carrying anything else is about a title
        /// that has since been edited, and is dropped.
        pub check_generation: Cell<u64>,
        /// The title the in-flight or completed check was about, so an edit
        /// that lands back on the same words does not ask twice.
        pub checked_title: RefCell<String>,

        /// Set once the user has answered the confirmation for the current
        /// title. Without it, "Add anyway" would re-open the same dialog.
        pub confirmed: Cell<bool>,
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
            if let Some(id) = self.check_debounce.take() {
                id.remove();
            }
            // Any request already in flight answers into a dropped weak
            // reference and stops there; the worker thread finishes on its own.
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
                    // The user picked one of the similar tasks instead of
                    // adding a new one. Carries its id; the window opens it.
                    Signal::builder("open-existing")
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

        // What already looks like this. Under the chips, because it answers the
        // same kind of question they do — "here is what the app made of what
        // you typed" — and above the buttons, because it is meant to be read
        // before Add is pressed rather than after.
        let similar_host = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .visible(false)
            .build();

        let similar_heading = gtk::Label::builder()
            .label("Similar tasks")
            .xalign(0.0)
            .build();
        similar_heading.add_css_class("caption-heading");
        similar_heading.add_css_class("dimmed");
        similar_host.append(&similar_heading);

        let similar_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        similar_list.add_css_class("boxed-list");
        similar_host.append(&similar_list);

        let checking_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .visible(false)
            .build();
        let spinner = adw::Spinner::new();
        spinner.set_size_request(14, 14);
        checking_row.append(&spinner);
        let checking_label = gtk::Label::new(Some("Checking for duplicates…"));
        checking_label.add_css_class("caption");
        checking_label.add_css_class("dimmed");
        checking_row.append(&checking_label);
        similar_host.append(&checking_row);

        body.append(&similar_host);

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
        imp.similar_host.replace(Some(similar_host));
        imp.similar_heading.replace(Some(similar_heading));
        imp.similar_list.replace(Some(similar_list));
        imp.checking_row.replace(Some(checking_row));

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
        let title = parsed.title.trim().to_string();
        imp.parsed.replace(parsed);
        self.refresh_similar(&title);
    }

    /// Recompute what resembles `title` and show it.
    ///
    /// The local half is pure and microsecond-scale, so it runs on every parse
    /// with no debounce of its own. The model is asked separately and later.
    fn refresh_similar(&self, title: &str) {
        let imp = self.imp();

        // A changed title invalidates the previous answer. Clearing the
        // verdicts here is what stops a "same task" badge outliving the words
        // it was about.
        if imp.checked_title.borrow().as_str() != title {
            imp.judgements.replace(None);
            imp.confirmed.set(false);
        }

        let candidates = match imp.candidate_source.borrow().as_ref() {
            Some(source) if title.chars().count() >= 2 => source(title),
            _ => Vec::new(),
        };
        imp.candidates.replace(candidates);

        self.show_similar();
        self.schedule_check(title);
    }

    /// Rebuild the list of similar tasks from the current candidates and
    /// whatever the model has said about them so far.
    fn show_similar(&self) {
        let imp = self.imp();
        let (Some(host), Some(list)) = (
            imp.similar_host.borrow().clone(),
            imp.similar_list.borrow().clone(),
        ) else {
            return;
        };

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        let shown = self.shown();
        let judgements = imp.judgements.borrow();

        host.set_visible(!shown.is_empty());
        if let Some(heading) = imp.similar_heading.borrow().as_ref() {
            heading.set_visible(!shown.is_empty());
        }

        for candidate in &shown {
            let judgement = judgements
                .as_ref()
                .and_then(|judged| judged.for_id(candidate.id.as_str()));
            list.append(&self.build_similar_row(candidate, judgement));
        }
    }

    /// The candidates that belong on screen.
    ///
    /// The gathered set is deliberately wider than this — it is gathered at
    /// [`RECALL_FLOOR`](crate::model::similar::RECALL_FLOOR) so the model gets
    /// a chance to recognise a pair the words alone would not justify showing.
    /// Which of those actually appear is decided here:
    ///
    /// - judged **different**: never. That is the app showing its working.
    /// - judged **same** or **related**: always, whatever the word score was.
    ///   This is the entire point of asking.
    /// - **unjudged**: only above the display floor, which is the behaviour
    ///   with no key, no network, or a reply still in flight.
    fn shown(&self) -> Vec<Candidate> {
        let imp = self.imp();
        let candidates = imp.candidates.borrow();
        let judgements = imp.judgements.borrow();

        candidates
            .iter()
            .filter(|candidate| {
                match judgements
                    .as_ref()
                    .and_then(|judged| judged.for_id(candidate.id.as_str()))
                {
                    Some(judgement) => judgement.verdict != Verdict::Different,
                    None => candidate.score >= crate::model::similar::FLOOR,
                }
            })
            .take(MAX_SHOWN)
            .cloned()
            .collect()
    }

    /// One row: what the task is called, where it lives, and — once the model
    /// has answered — what it made of the pair.
    fn build_similar_row(
        &self,
        candidate: &Candidate,
        judgement: Option<&crate::model::duplicate::Judgement>,
    ) -> gtk::Widget {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&candidate.title))
            .activatable(true)
            .build();

        // The subtitle carries the model's reason when there is one, because
        // "why does it think these are the same?" is the question the person
        // is being asked to answer and they cannot without it.
        let subtitle = match judgement {
            Some(judged) if !judged.reason.is_empty() => {
                format!("{} · {}", candidate.context, judged.reason)
            }
            _ => candidate.context.clone(),
        };
        row.set_subtitle(&glib::markup_escape_text(&subtitle));

        let icon = gtk::Image::from_icon_name(if candidate.checked {
            "object-select-symbolic"
        } else {
            "view-list-symbolic"
        });
        row.add_prefix(&icon);

        // A badge only where there is something to say. An unjudged near-miss
        // speaks for itself by being in the list at all.
        //
        // The two blocking states share `warning` rather than separating into
        // amber and red: red is for failure and lost data, and adding a second
        // copy of a task is neither. What distinguishes them is already in the
        // words — "Same task" carries a reason underneath, "Near-identical" is
        // the app's own guess with nothing to show for it.
        let badge = match judgement.map(|judged| judged.verdict) {
            Some(Verdict::Same) => Some(("Same task", "warning")),
            Some(Verdict::Related) => Some(("Related", "dimmed")),
            Some(Verdict::Different) => None,
            None if candidate.is_strong() => Some(("Near-identical", "warning")),
            None => None,
        };
        if let Some((text, style)) = badge {
            let label = gtk::Label::new(Some(text));
            label.add_css_class("caption");
            label.add_css_class(style);
            row.add_suffix(&label);
        }

        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_tooltip_text(Some("Open this task instead"));

        let id = candidate.id.as_str().to_string();
        row.connect_activated(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                dialog.emit_by_name::<()>("open-existing", &[&id]);
                dialog.close();
            }
        ));

        row.upcast()
    }

    /// Ask the model shortly, if there is anything worth asking about.
    fn schedule_check(&self, title: &str) {
        let imp = self.imp();
        if let Some(id) = imp.check_debounce.take() {
            id.remove();
        }

        let Some(key) = imp.api_key.borrow().clone() else {
            return;
        };
        // Nothing to compare against, nothing to ask. The local pass having
        // found nothing is itself the answer: a title sharing no words with
        // anything on the list is not a duplicate of it.
        if imp.candidates.borrow().is_empty()
            || title.chars().count() < MIN_TITLE_FOR_CHECK
            || imp.checked_title.borrow().as_str() == title
        {
            return;
        }

        let title = title.to_string();
        let id = glib::timeout_add_local_once(
            CHECK_DEBOUNCE,
            glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move || {
                    dialog.imp().check_debounce.replace(None);
                    dialog.start_check(key, title);
                }
            ),
        );
        imp.check_debounce.replace(Some(id));
    }

    fn start_check(&self, key: String, title: String) {
        let imp = self.imp();
        let candidates = imp.candidates.borrow().clone();
        if candidates.is_empty() {
            return;
        }

        let generation = imp.check_generation.get().wrapping_add(1);
        imp.check_generation.set(generation);
        imp.checked_title.replace(title.clone());
        self.set_checking(true);

        crate::ui::duplicate_check::spawn(
            key,
            title,
            candidates,
            generation,
            glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |generation, outcome| dialog.finish_check(generation, outcome)
            ),
        );
    }

    /// A reply landed. Main thread, by construction.
    fn finish_check(&self, generation: u64, outcome: Result<Judgements, CheckError>) {
        let imp = self.imp();
        // A reply about a title the user has moved on from. Dropped in silence:
        // there is nothing for anyone to do about it.
        if generation != imp.check_generation.get() {
            return;
        }
        self.set_checking(false);

        match outcome {
            Ok(judgements) => {
                imp.judgements.replace(Some(judgements));
                self.show_similar();
            }
            // Offline, no key, a bad key, a decline. All of them mean the same
            // thing here — the local candidates stay exactly as they are and
            // nothing is said about it. Someone adding a task is not in a
            // position to fix an API key, and a banner over the entry would be
            // the app making its problem into theirs.
            Err(error) => {
                imp.judgements.replace(None);
                eprintln!("planner: duplicate check unavailable: {error}");
            }
        }
    }

    fn set_checking(&self, checking: bool) {
        if let Some(row) = self.imp().checking_row.borrow().as_ref() {
            row.set_visible(checking);
        }
        if let Some(host) = self.imp().similar_host.borrow().as_ref() {
            if checking {
                host.set_visible(true);
            }
        }
    }

    /// The candidates this would stop to ask about, if Add were pressed now.
    ///
    /// The model's verdict wins where there is one, because it saw the words
    /// and the local pass only counted them. Where there is none — no key, no
    /// network, a reply still in flight — a near-identical local match stands
    /// in, so the feature degrades to something rather than nothing.
    fn blocking(&self) -> Vec<Candidate> {
        let imp = self.imp();
        let candidates = imp.candidates.borrow();
        match imp.judgements.borrow().as_ref() {
            Some(judged) => {
                let blocking = judged.blocking();
                candidates
                    .iter()
                    .filter(|candidate| blocking.contains(&candidate.id.as_str()))
                    .cloned()
                    .collect()
            }
            None => candidates
                .iter()
                .filter(|candidate| candidate.is_strong())
                .cloned()
                .collect(),
        }
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
    ///
    /// Everything above this point is advisory — the list under the entry says
    /// what it found and Add still adds. This is the one place that stops, and
    /// only for a match strong enough that adding is almost certainly a
    /// mistake. The undo toast that normally covers a wrong add cannot: it
    /// deletes the new task, not the duplicate already on the list.
    fn submit(&self) {
        let imp = self.imp();
        let Some(entry) = imp.entry.borrow().clone() else {
            return;
        };
        if imp.parsed.borrow().title.trim().is_empty() {
            return;
        }

        if !imp.confirmed.get() {
            let blocking = self.blocking();
            if !blocking.is_empty() {
                self.confirm_duplicate(blocking);
                return;
            }
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
            imp.confirmed.set(false);
            imp.checked_title.replace(String::new());
            imp.judgements.replace(None);
            self.reparse();
            self.set_focus(Some(&entry));
        } else {
            self.close();
        }
    }

    /// Ask before adding something that already exists.
    ///
    /// Cancel first and the specific verb last, per the HIG. "Add Anyway" is
    /// suggested rather than destructive: the user typed this on purpose and
    /// the app is second-guessing them, so the affirmative is the one that
    /// should be easy to hit.
    fn confirm_duplicate(&self, blocking: Vec<Candidate>) {
        let title = self.imp().parsed.borrow().title.trim().to_string();

        let body = match blocking.as_slice() {
            [only] => format!(
                "“{}” is already on your list, in {}.\n\nAdd “{}” as well?",
                only.title,
                if only.context.is_empty() {
                    "your tasks".to_string()
                } else {
                    only.context.clone()
                },
                title,
            ),
            many => {
                let names = many
                    .iter()
                    .map(|candidate| format!("• {}", candidate.title))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("These are already on your list:\n\n{names}\n\nAdd “{title}” as well?")
            }
        };

        let alert = adw::AlertDialog::new(Some("This looks like a duplicate"), Some(&body));
        alert.add_response("cancel", "Cancel");
        alert.add_response("open", "Open Existing");
        alert.add_response("add", "Add Anyway");
        alert.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");

        let first = blocking
            .first()
            .map(|candidate| candidate.id.as_str().to_string())
            .unwrap_or_default();

        alert.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |_, response| match response {
                    "add" => {
                        // Sticky for this title: having answered once, pressing
                        // Add again must add rather than ask the same question.
                        dialog.imp().confirmed.set(true);
                        dialog.submit();
                    }
                    "open" => {
                        dialog.emit_by_name::<()>("open-existing", &[&first]);
                        dialog.close();
                    }
                    // Cancel leaves the entry exactly as it was, so the obvious
                    // next move — editing the title into something distinct —
                    // costs nothing.
                    _ => {}
                }
            ),
        );

        alert.present(Some(self));
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

    /// Teach the dialog how to find tasks resembling a title.
    ///
    /// A closure rather than a `Store`, so the rule that this file never
    /// touches one survives the feature. The window owns the store and hands
    /// down the question.
    pub fn set_candidate_source(&self, source: impl Fn(&str) -> Vec<Candidate> + 'static) {
        self.imp().candidate_source.replace(Some(Box::new(source)));
        self.reparse();
    }

    /// The key that turns on the semantic half. `None` leaves the local word
    /// comparison, which is the whole feature until someone configures one.
    pub fn set_api_key(&self, key: Option<String>) {
        self.imp()
            .api_key
            .replace(key.filter(|key| !key.trim().is_empty()));
    }

    /// The similar tasks currently on screen, for tests.
    ///
    /// What is displayed, not what was gathered — the gathered set is wider on
    /// purpose and most of it never reaches a person. See [`Self::shown`].
    pub fn similar(&self) -> Vec<Candidate> {
        self.shown()
    }

    /// Whether pressing Add right now would stop to ask, for tests.
    pub fn would_confirm(&self) -> bool {
        !self.imp().confirmed.get() && !self.blocking().is_empty()
    }

    /// Pretend the model answered, for tests. The socket is not exercised
    /// here; [`crate::model::duplicate`] covers the wire format headlessly.
    pub fn apply_judgements(&self, judgements: Judgements) {
        self.imp().judgements.replace(Some(judgements));
        self.show_similar();
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

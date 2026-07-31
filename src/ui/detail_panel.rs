//! Everything about one task.
//!
//! **Edits are reported as a typed [`Edit`], through a Rust callback.**
//! Everywhere else in this app a widget reports what the user did with a
//! GObject signal, and that is right when the payload is a string and a bool.
//! It stops being right here: half these edits carry a `Due`, an `Option<
//! NaiveDate>` or a `Priority`, none of which is a `glib::Value` without being
//! boxed into one, and the boxing would buy nothing — both ends of this wire
//! are Rust, in the same crate. Flattening `Due` into a string to fit through
//! a signal would mean parsing it back on the other side, which is a
//! serialisation format invented to avoid admitting the callback was simpler.
//!
//! The panel still never touches the store. It says what happened; the
//! application decides what it means and writes it down.
//!
//! **The panel is rebuilt from the store after every edit.** It does not trust
//! its own widgets to be the truth: setting a due date on a repeating task can
//! change the rule, and completing a subtask changes the parent's count, so
//! the answer to "what does this task look like now" always comes back from
//! the store rather than from what the panel just did.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use chrono::NaiveDate;

use crate::model::due::Due;
use crate::model::store::Store;
use crate::model::task::Task;
use crate::model::{Priority, TaskId};
use crate::ui::date_picker::{describe_deadline, describe_due, DatePicker};

/// Something the user did to the task on show.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Title(String),
    Description(String),
    Priority(Priority),
    Due(Option<Due>),
    Deadline(Option<NaiveDate>),
    /// A label added or removed, by name. Names rather than IDs because a
    /// label typed into the box may not exist yet.
    Label {
        name: String,
        on: bool,
    },
    Pinned(bool),
    Delete,
    /// A quick-add line to file as a subtask of this task.
    AddSubtask(String),
    ToggleSubtask(TaskId, bool),
    /// Show a different task — clicking through to a subtask.
    Open(TaskId),
}

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    type Callback = Box<dyn Fn(&Edit)>;

    #[derive(Default)]
    pub struct DetailPanel {
        pub title: RefCell<Option<adw::EntryRow>>,
        pub description: RefCell<Option<gtk::TextView>>,
        pub schedule: RefCell<Option<adw::ActionRow>>,
        pub deadline: RefCell<Option<adw::ActionRow>>,
        pub priority: RefCell<Option<adw::ComboRow>>,
        pub priority_swatch: RefCell<Option<gtk::Box>>,
        pub labels: RefCell<Option<adw::ActionRow>>,
        pub label_box: RefCell<Option<adw::WrapBox>>,
        pub subtasks: RefCell<Option<adw::PreferencesGroup>>,
        pub subtask_rows: RefCell<Vec<gtk::Widget>>,
        pub new_subtask: RefCell<Option<adw::EntryRow>>,
        pub pin: RefCell<Option<gtk::ToggleButton>>,
        pub content: RefCell<Option<gtk::Widget>>,
        pub placeholder: RefCell<Option<gtk::Widget>>,
        pub stack: RefCell<Option<gtk::Stack>>,

        pub due_picker: RefCell<Option<DatePicker>>,
        pub deadline_picker: RefCell<Option<DatePicker>>,

        pub task: RefCell<Option<TaskId>>,
        pub today: Cell<Option<NaiveDate>>,
        pub edited: RefCell<Option<Callback>>,
        /// Set while `show` writes widget state, so the resulting `changed`
        /// and `toggled` are not reported back as user edits. Every widget
        /// here is written to programmatically, so without this every refresh
        /// would echo itself back as an edit and loop.
        pub loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DetailPanel {
        const NAME: &'static str = "PlannerDetailPanel";
        type Type = super::DetailPanel;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for DetailPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for DetailPanel {}
    impl BoxImpl for DetailPanel {}
}

glib::wrapper! {
    pub struct DetailPanel(ObjectSubclass<imp::DetailPanel>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for DetailPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailPanel {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build()
    }

    /// Called for every edit the user makes.
    pub fn connect_edited(&self, callback: impl Fn(&Edit) + 'static) {
        self.imp().edited.replace(Some(Box::new(callback)));
    }

    fn report(&self, edit: Edit) {
        if self.imp().loading.get() {
            return;
        }
        // The callback comes straight back through `show`, which writes every
        // widget here. That is safe because `show` never touches `edited` —
        // the only borrow this re-entrancy could collide with. Anything that
        // did would have to take the callback out first, because a panic
        // inside a GTK handler crosses `extern "C"` and aborts.
        if let Some(callback) = self.imp().edited.borrow().as_ref() {
            callback(&edit);
        }
    }

    /// The task on show, if any.
    pub fn task_id(&self) -> Option<TaskId> {
        self.imp().task.borrow().clone()
    }

    /// Stop showing anything.
    pub fn clear(&self) {
        self.imp().task.replace(None);
        if let Some(stack) = self.imp().stack.borrow().as_ref() {
            stack.set_visible_child_name("empty");
        }
    }

    /// Show a task, reading everything from the store.
    pub fn show(&self, id: &TaskId, store: &Store, today: NaiveDate) {
        let Some(task) = store.task(id) else {
            self.clear();
            return;
        };
        let imp = self.imp();
        imp.loading.set(true);
        imp.task.replace(Some(id.clone()));
        imp.today.set(Some(today));

        if let Some(stack) = imp.stack.borrow().as_ref() {
            stack.set_visible_child_name("task");
        }
        // Only write a text field that is actually out of date. `show` runs
        // after every keystroke — the title edit that triggered it came from
        // this very entry — and setting the text of an entry moves the cursor
        // to the start, so writing it unconditionally would make the title
        // impossible to type in.
        if let Some(row) = imp.title.borrow().as_ref() {
            if row.text() != task.content {
                row.set_text(&task.content);
            }
        }
        if let Some(view) = imp.description.borrow().as_ref() {
            let buffer = view.buffer();
            let current = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            if current != task.description {
                buffer.set_text(&task.description);
            }
        }
        if let Some(row) = imp.schedule.borrow().as_ref() {
            row.set_subtitle(&describe_due(task.due.as_ref(), today));
        }
        if let Some(row) = imp.deadline.borrow().as_ref() {
            row.set_subtitle(&describe_deadline(task.deadline, today));
        }
        if let Some(row) = imp.priority.borrow().as_ref() {
            row.set_selected(task.priority.rank() as u32);
        }
        if let Some(swatch) = imp.priority_swatch.borrow().as_ref() {
            for class in ["priority-1", "priority-2", "priority-3"] {
                swatch.remove_css_class(class);
            }
            // Unset priority has no colour to show, and an empty ring in the
            // prefix would read as a fourth level rather than as "none".
            match task.priority.css_class() {
                Some(class) => {
                    swatch.add_css_class(class);
                    swatch.set_visible(true);
                }
                None => swatch.set_visible(false),
            }
        }
        if let Some(toggle) = imp.pin.borrow().as_ref() {
            toggle.set_active(task.pinned);
        }

        self.show_labels(task, store);
        self.show_subtasks(task, store);

        if let Some(picker) = imp.due_picker.borrow().as_ref() {
            picker.load(task.due.as_ref(), today);
        }
        if let Some(picker) = imp.deadline_picker.borrow().as_ref() {
            picker.load(task.deadline.map(Due::on).as_ref(), today);
        }

        imp.loading.set(false);
    }

    fn build(&self) {
        let imp = self.imp();

        let header = adw::HeaderBar::builder()
            .show_title(false)
            .css_classes(["flat"])
            .build();

        let pin = gtk::ToggleButton::builder()
            .icon_name("view-pin-symbolic")
            .tooltip_text("Pin")
            .build();
        pin.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |toggle| panel.report(Edit::Pinned(toggle.is_active()))
        ));
        header.pack_end(&pin);

        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete Task")
            .css_classes(["flat"])
            .build();
        delete.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.report(Edit::Delete)
        ));
        header.pack_end(&delete);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(12)
            .margin_bottom(18)
            .margin_start(12)
            .margin_end(12)
            .build();

        // --- title and description --------------------------------------
        let title_group = adw::PreferencesGroup::new();
        let title = adw::EntryRow::builder().title("Task").build();
        title.connect_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |row| panel.report(Edit::Title(row.text().to_string()))
        ));
        title_group.add(&title);
        body.append(&title_group);

        let description_group = adw::PreferencesGroup::builder()
            .title("Description")
            .build();
        let description = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .height_request(90)
            .css_classes(["description-view"])
            .build();
        description.buffer().connect_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |buffer| {
                let text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string();
                panel.report(Edit::Description(text));
            }
        ));
        let description_frame = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&description)
            .css_classes(["card"])
            .build();
        description_group.add(&description_frame);
        body.append(&description_group);

        // --- properties -------------------------------------------------
        let properties = adw::PreferencesGroup::builder().title("Details").build();

        let (schedule, due_picker) = self.build_date_row("Schedule", "x-office-calendar-symbolic");
        due_picker.connect_chosen(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |due| panel.report(Edit::Due(due))
        ));
        properties.add(&schedule);

        let (deadline, deadline_picker) = self.build_date_row("Deadline", "alarm-symbolic");
        deadline_picker.connect_chosen(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |due| panel.report(Edit::Deadline(due.map(|due| due.date)))
        ));
        properties.add(&deadline);

        let priority_names: Vec<&str> = Priority::ALL.iter().map(|p| p.label()).collect();
        let priority = adw::ComboRow::builder()
            .title("Priority")
            .model(&gtk::StringList::new(&priority_names))
            .build();
        // The list teaches red = P1 by tinting the checkbox. Showing "High" as
        // plain text here drops the colour at the one place you go to change
        // it, so the row carries the same ring in its prefix.
        let priority_swatch = gtk::Box::builder()
            .valign(gtk::Align::Center)
            .css_classes(["priority-swatch"])
            .build();
        priority.add_prefix(&priority_swatch);
        priority.connect_selected_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |row| {
                let selected = Priority::ALL
                    .get(row.selected() as usize)
                    .copied()
                    .unwrap_or_default();
                panel.report(Edit::Priority(selected));
            }
        ));
        properties.add(&priority);

        // Schedule and Deadline carry prefix icons, so Labels needs one too or
        // the group's titles sit on two different left edges.
        let labels = adw::ActionRow::builder().title("Labels").build();
        labels.add_prefix(&gtk::Image::from_icon_name("user-bookmarks-symbolic"));
        let label_box = adw::WrapBox::builder()
            .child_spacing(4)
            .line_spacing(4)
            .valign(gtk::Align::Center)
            .build();
        labels.add_suffix(&label_box);
        let label_button = gtk::MenuButton::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Label")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        label_button.set_popover(Some(&self.build_label_popover()));
        labels.add_suffix(&label_button);
        properties.add(&labels);

        body.append(&properties);

        // --- subtasks ---------------------------------------------------
        let subtasks = adw::PreferencesGroup::builder().title("Subtasks").build();
        let new_subtask = adw::EntryRow::builder().title("Add a subtask").build();
        new_subtask.connect_apply(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |row| {
                let line = row.text().to_string();
                if !line.trim().is_empty() {
                    panel.report(Edit::AddSubtask(line));
                    row.set_text("");
                }
            }
        ));
        new_subtask.set_show_apply_button(true);
        subtasks.add(&new_subtask);
        body.append(&subtasks);

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build();

        let placeholder = adw::StatusPage::builder()
            .icon_name("document-edit-symbolic")
            .title("No task selected")
            .description("Open a task to see its details.")
            .vexpand(true)
            .build();

        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&scroller, Some("task"));
        stack.add_named(&placeholder, Some("empty"));
        stack.set_visible_child_name("empty");

        let toolbar = adw::ToolbarView::builder()
            .top_bar_style(adw::ToolbarStyle::Flat)
            .content(&stack)
            .build();
        toolbar.add_top_bar(&header);
        self.append(&toolbar);

        imp.title.replace(Some(title));
        imp.description.replace(Some(description));
        imp.schedule.replace(Some(schedule));
        imp.deadline.replace(Some(deadline));
        imp.priority.replace(Some(priority));
        imp.priority_swatch.replace(Some(priority_swatch));
        imp.labels.replace(Some(labels));
        imp.label_box.replace(Some(label_box));
        imp.subtasks.replace(Some(subtasks));
        imp.new_subtask.replace(Some(new_subtask));
        imp.pin.replace(Some(pin));
        imp.due_picker.replace(Some(due_picker));
        imp.deadline_picker.replace(Some(deadline_picker));
        imp.stack.replace(Some(stack.clone()));
        imp.content.replace(Some(scroller.upcast()));
        imp.placeholder.replace(Some(placeholder.upcast()));
    }

    /// A row whose suffix button opens a date picker.
    fn build_date_row(&self, title: &str, icon: &str) -> (adw::ActionRow, DatePicker) {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle("No date")
            .build();

        let image = gtk::Image::from_icon_name(icon);
        row.add_prefix(&image);

        let picker = DatePicker::new();
        let button = gtk::MenuButton::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text(format!("Change {}", title.to_lowercase()))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .popover(&picker)
            .build();
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));

        (row, picker)
    }

    /// The popover for adding a label: type a name, or pick an existing one.
    fn build_label_popover(&self) -> gtk::Popover {
        let popover = gtk::Popover::new();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();

        let entry = gtk::Entry::builder().placeholder_text("Label name").build();
        entry.connect_activate(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[weak]
            popover,
            move |entry| {
                let name = entry.text().trim().to_string();
                if !name.is_empty() {
                    panel.report(Edit::Label { name, on: true });
                    entry.set_text("");
                    popover.popdown();
                }
            }
        ));
        body.append(&entry);
        popover.set_child(Some(&body));
        popover
    }

    /// Rebuild the label chips, each with a button that takes it off again.
    fn show_labels(&self, task: &Task, store: &Store) {
        let Some(chips) = self.imp().label_box.borrow().clone() else {
            return;
        };
        chips.remove_all();

        for label in task.labels.iter().filter_map(|id| store.label(id)) {
            let chip = gtk::Button::builder()
                .label(&label.name)
                .tooltip_text(format!("Remove “{}”", label.name))
                .css_classes(["label-chip", "flat"])
                .build();
            let name = label.name.clone();
            chip.connect_clicked(glib::clone!(
                #[weak(rename_to = panel)]
                self,
                move |_| {
                    panel.report(Edit::Label {
                        name: name.clone(),
                        on: false,
                    })
                }
            ));
            chips.append(&chip);
        }
    }

    /// Rebuild the subtask rows.
    fn show_subtasks(&self, task: &Task, store: &Store) {
        let Some(group) = self.imp().subtasks.borrow().clone() else {
            return;
        };
        for row in self.imp().subtask_rows.take() {
            group.remove(&row);
        }

        let mut rows = Vec::new();
        for child in store.subtasks(&task.id) {
            let row = adw::ActionRow::builder().title(&child.content).build();

            let check = gtk::CheckButton::builder()
                .active(child.checked)
                .valign(gtk::Align::Center)
                .build();
            let id = child.id.clone();
            check.connect_toggled(glib::clone!(
                #[weak(rename_to = panel)]
                self,
                #[strong]
                id,
                move |check| {
                    panel.report(Edit::ToggleSubtask(id.clone(), check.is_active()));
                }
            ));
            row.add_prefix(&check);
            row.set_activatable_widget(Some(&check));

            let open = gtk::Button::builder()
                .icon_name("go-next-symbolic")
                .tooltip_text("Open")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let id = child.id.clone();
            open.connect_clicked(glib::clone!(
                #[weak(rename_to = panel)]
                self,
                move |_| panel.report(Edit::Open(id.clone()))
            ));
            row.add_suffix(&open);

            if child.checked {
                row.add_css_class("dimmed");
            }

            group.add(&row);
            rows.push(row.upcast::<gtk::Widget>());
        }

        // Move the "add" row back to the bottom. `PreferencesGroup::add`
        // appends, so without this the box you type into sits above the list
        // it adds to, and a task with several subtasks pushes it off-screen.
        if let Some(entry) = self.imp().new_subtask.borrow().as_ref() {
            group.remove(entry);
            group.add(entry);
        }
        self.imp().subtask_rows.replace(rows);
    }

    // --- accessors, for tests -------------------------------------------

    /// The title as the panel currently shows it.
    pub fn title_text(&self) -> String {
        self.imp()
            .title
            .borrow()
            .as_ref()
            .map(|row| row.text().to_string())
            .unwrap_or_default()
    }

    /// The description as the panel currently shows it.
    pub fn description_text(&self) -> String {
        self.imp()
            .description
            .borrow()
            .as_ref()
            .map(|view| {
                let buffer = view.buffer();
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string()
            })
            .unwrap_or_default()
    }

    /// What the schedule row says.
    pub fn schedule_text(&self) -> String {
        self.imp()
            .schedule
            .borrow()
            .as_ref()
            .map(|row| row.subtitle().unwrap_or_default().to_string())
            .unwrap_or_default()
    }

    /// What the deadline row says.
    pub fn deadline_text(&self) -> String {
        self.imp()
            .deadline
            .borrow()
            .as_ref()
            .map(|row| row.subtitle().unwrap_or_default().to_string())
            .unwrap_or_default()
    }

    /// The priority the combo is showing.
    pub fn priority(&self) -> Priority {
        self.imp()
            .priority
            .borrow()
            .as_ref()
            .and_then(|row| Priority::ALL.get(row.selected() as usize).copied())
            .unwrap_or_default()
    }

    /// Whether the pin toggle is on.
    pub fn is_pinned(&self) -> bool {
        self.imp()
            .pin
            .borrow()
            .as_ref()
            .is_some_and(|toggle| toggle.is_active())
    }

    /// How many subtask rows are showing.
    pub fn subtask_count(&self) -> usize {
        self.imp().subtask_rows.borrow().len()
    }

    /// Whether the panel is showing a task rather than the placeholder.
    pub fn is_showing_task(&self) -> bool {
        self.imp()
            .stack
            .borrow()
            .as_ref()
            .and_then(|stack| stack.visible_child_name())
            .is_some_and(|name| name == "task")
    }
}

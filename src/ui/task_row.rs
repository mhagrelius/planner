//! One row in a task list.
//!
//! The row renders and reports; it never persists. Ticking the checkbox emits
//! `toggled` and nothing else — the application decides what completing a task
//! means, which is the only way "completing a recurring task reschedules it"
//! can be true in every list at once.
//!
//! Rows are recycled by the list view, so everything `bind` sets up `unbind`
//! must take down. A binding left in place keeps the *old* task alive and
//! keeps writing its changes into a row now showing something else.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::ui::task_object::TaskObject;

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::cell::RefCell;
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct TaskRow {
        pub check: RefCell<Option<gtk::CheckButton>>,
        pub title: RefCell<Option<gtk::Label>>,
        pub details: RefCell<Option<gtk::Box>>,
        pub due: RefCell<Option<gtk::Label>>,
        pub deadline: RefCell<Option<gtk::Label>>,
        pub recurring: RefCell<Option<gtk::Image>>,
        pub labels: RefCell<Option<adw::WrapBox>>,
        pub subtasks: RefCell<Option<gtk::Label>>,

        pub bindings: RefCell<Vec<glib::Binding>>,
        /// Handler watching the bound item's `checked`, disconnected on
        /// unbind. A class cannot be a binding target, so this is the one
        /// piece of the row driven by a handler rather than by a binding.
        pub checked_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub item: RefCell<Option<TaskObject>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TaskRow {
        const NAME: &'static str = "PlannerTaskRow";
        type Type = super::TaskRow;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for TaskRow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // The task's ID, and whether it is now ticked.
                    Signal::builder("toggled")
                        .param_types([str::static_type(), bool::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for TaskRow {}
    impl BoxImpl for TaskRow {}
}

glib::wrapper! {
    pub struct TaskRow(ObjectSubclass<imp::TaskRow>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for TaskRow {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRow {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Horizontal)
            .build()
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("task-row");

        // The checkbox and the body are centred together, as one block, rather
        // than each in its own right. Rows are held to a minimum height so the
        // list keeps an even rhythm past tasks with nothing on their second
        // line, and centring the two separately would spend that slack twice:
        // the title would drift down the row while the checkbox stayed put.
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        self.append(&content);

        let check = gtk::CheckButton::builder()
            .valign(gtk::Align::Start)
            .build();
        check.connect_toggled(glib::clone!(
            #[weak(rename_to = row)]
            self,
            move |check| row.report_toggle(check.is_active())
        ));
        content.append(&check);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();

        let title = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        title.add_css_class("task-title");
        body.append(&title);

        // The second line only exists when there is something on it, so a
        // plain task stays one line tall. Hidden rather than merely empty:
        // an empty box still costs the spacing above it.
        let details = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();

        let due = gtk::Label::new(None);
        due.add_css_class("due-chip");
        details.append(&due);

        // Symbolic icons are drawn on a 16px grid; scaling one to 12 lands its
        // strokes between pixels and it stops reading as a glyph at all.
        let recurring = gtk::Image::from_icon_name("media-playlist-repeat-symbolic");
        recurring.set_pixel_size(16);
        recurring.add_css_class("dimmed");
        // The text comes from the task, bound in `bind`.
        details.append(&recurring);

        let deadline = gtk::Label::new(None);
        deadline.add_css_class("deadline-chip");
        deadline.add_css_class("dimmed");
        details.append(&deadline);

        let subtasks = gtk::Label::new(None);
        subtasks.add_css_class("caption");
        subtasks.add_css_class("dimmed");
        details.append(&subtasks);

        // One chip per label, not one chip listing them all: two labels are
        // two things, and a single pill reading "errand, home" makes the
        // comma look like part of a name.
        let labels = adw::WrapBox::builder()
            .child_spacing(4)
            .line_spacing(4)
            .valign(gtk::Align::Center)
            .build();
        details.append(&labels);

        body.append(&details);
        content.append(&body);

        self.install_drag_source();

        imp.check.replace(Some(check));
        imp.title.replace(Some(title));
        imp.details.replace(Some(details));
        imp.due.replace(Some(due));
        imp.deadline.replace(Some(deadline));
        imp.recurring.replace(Some(recurring));
        imp.labels.replace(Some(labels));
        imp.subtasks.replace(Some(subtasks));
    }

    /// Let the row be dragged, carrying its task's ID.
    ///
    /// The ID rather than the row: a drop lands in a different list, whose
    /// job is to look the task up and move it. Handing over a widget would
    /// mean two lists sharing one, and handing over the `TaskObject` would
    /// mean the destination holding a projection built for somewhere else.
    fn install_drag_source(&self) {
        let source = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::MOVE)
            .build();

        source.connect_prepare(glib::clone!(
            #[weak(rename_to = row)]
            self,
            #[upgrade_or]
            None,
            move |_, _, _| {
                let item = row.imp().item.borrow().clone()?;
                Some(gtk::gdk::ContentProvider::for_value(&item.id().to_value()))
            }
        ));

        // A drag needs to look like it picked the row up, and the row it left
        // behind needs to look like a gap rather than a duplicate.
        source.connect_drag_begin(glib::clone!(
            #[weak(rename_to = row)]
            self,
            move |source, _| {
                let paintable = gtk::WidgetPaintable::new(Some(&row));
                source.set_icon(Some(&paintable), 0, 0);
                row.add_css_class("dragging");
            }
        ));
        source.connect_drag_end(glib::clone!(
            #[weak(rename_to = row)]
            self,
            move |_, _, _| row.remove_css_class("dragging")
        ));
        source.connect_drag_cancel(glib::clone!(
            #[weak(rename_to = row)]
            self,
            #[upgrade_or]
            false,
            move |_, _, _| {
                row.remove_css_class("dragging");
                false
            }
        ));

        self.add_controller(source);
    }

    /// Decide whether a `toggled` came from the user, and report it if so.
    ///
    /// The checkbox is driven from the item by a one-way binding, so it fires
    /// `toggled` for two quite different reasons: the user clicked it, or the
    /// model changed underneath it. Telling them apart matters — reporting the
    /// second kind sends "the user completed this task" back to the
    /// application every time a refresh touches the row, which is a loop.
    ///
    /// The state itself distinguishes them. A click moves the checkbox while
    /// the item still holds the old value, so the two disagree; a model change
    /// arrives *through* the item, so by the time the checkbox catches up they
    /// already agree.
    fn report_toggle(&self, active: bool) {
        let Some(item) = self.imp().item.borrow().clone() else {
            return;
        };
        if item.checked() == active {
            return;
        }
        self.emit_by_name::<()>("toggled", &[&item.id(), &active]);
    }

    /// Show a task.
    pub fn bind(&self, item: &TaskObject) {
        let imp = self.imp();
        imp.item.replace(Some(item.clone()));

        let check = imp.check.borrow().clone().expect("built");
        let title = imp.title.borrow().clone().expect("built");
        let details = imp.details.borrow().clone().expect("built");
        let due = imp.due.borrow().clone().expect("built");
        let deadline = imp.deadline.borrow().clone().expect("built");
        let recurring = imp.recurring.borrow().clone().expect("built");
        let labels = imp.labels.borrow().clone().expect("built");
        let subtasks = imp.subtasks.borrow().clone().expect("built");

        let bindings = vec![
            item.bind_property("content", &title, "label")
                .sync_create()
                .build(),
            item.bind_property("checked", &check, "active")
                .sync_create()
                .build(),
            item.bind_property("has-details", &details, "visible")
                .sync_create()
                .build(),
            item.bind_property("due-label", &due, "label")
                .sync_create()
                .build(),
            item.bind_property("has-due", &due, "visible")
                .sync_create()
                .build(),
            item.bind_property("recurring", &recurring, "visible")
                .sync_create()
                .build(),
            item.bind_property("repeat-label", &recurring, "tooltip-text")
                .sync_create()
                .build(),
            item.bind_property("deadline-label", &deadline, "label")
                .sync_create()
                .build(),
            item.bind_property("has-deadline", &deadline, "visible")
                .sync_create()
                .build(),
            item.bind_property("has-labels", &labels, "visible")
                .sync_create()
                .build(),
            item.bind_property("subtasks", &subtasks, "label")
                .sync_create()
                .build(),
            item.bind_property("has-subtasks", &subtasks, "visible")
                .sync_create()
                .build(),
        ];

        // A chip per label. Widgets cannot be bound to a property any more
        // than a class can, so these are built here and torn down on unbind.
        labels.remove_all();
        for name in item.labels().iter() {
            let chip = gtk::Label::new(Some(name));
            chip.add_css_class("label-chip");
            labels.append(&chip);
        }

        // Classes cannot be bound to a property, so they are applied here and
        // cleared on unbind alongside everything else.
        apply_class(&due, item.due_class());
        // Priority colours the checkbox rather than a separate marker. A dot
        // off on its own at the end of the row is a second thing to look at
        // saying something about the first; tinting the control you are about
        // to click says it in the place you are already looking.
        apply_class(&check, item.priority_class());
        set_class(&deadline, "past", item.deadline_past());
        set_class(self, "completed", item.checked());

        let handler = item.connect_checked_notify(glib::clone!(
            #[weak(rename_to = row)]
            self,
            move |item| set_class(&row, "completed", item.checked())
        ));
        imp.checked_handler.replace(Some(handler));

        imp.bindings.replace(bindings);
    }

    /// Stop showing a task.
    pub fn unbind(&self) {
        let imp = self.imp();
        for binding in imp.bindings.take() {
            binding.unbind();
        }
        // Both ends have to go: the handler is on the *item*, which outlives
        // this row, so leaving it connected would keep a recycled row being
        // driven by a task it no longer shows.
        if let (Some(handler), Some(item)) = (imp.checked_handler.take(), imp.item.borrow().clone())
        {
            item.disconnect(handler);
        }
        imp.item.replace(None);
        self.remove_css_class("completed");
        self.clear_drop_hint();

        // Chips belong to the task that was here, not to the row.
        if let Some(labels) = imp.labels.borrow().as_ref() {
            labels.remove_all();
        }

        if let Some(due) = imp.due.borrow().as_ref() {
            clear_state_classes(due);
        }
        if let Some(check) = imp.check.borrow().as_ref() {
            clear_state_classes(check);
        }
        if let Some(deadline) = imp.deadline.borrow().as_ref() {
            clear_state_classes(deadline);
        }
    }

    /// The task this row is showing, if any.
    pub fn item(&self) -> Option<TaskObject> {
        self.imp().item.borrow().clone()
    }
}

impl TaskRow {
    /// Show where a drop would land: above the row, or below it.
    pub fn set_drop_hint(&self, before: bool) {
        self.remove_css_class(if before { "drop-below" } else { "drop-above" });
        self.add_css_class(if before { "drop-above" } else { "drop-below" });
    }

    /// Stop showing a drop position.
    pub fn clear_drop_hint(&self) {
        self.remove_css_class("drop-above");
        self.remove_css_class("drop-below");
    }
}

/// Every class this module applies conditionally. Anything not in this list is
/// structural and must survive being unbound.
const STATE_CLASSES: [&str; 6] = [
    "overdue",
    "today",
    "past",
    "priority-1",
    "priority-2",
    "priority-3",
];

fn apply_class(widget: &impl IsA<gtk::Widget>, class: String) {
    clear_state_classes(widget);
    if !class.is_empty() {
        widget.as_ref().add_css_class(&class);
    }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, present: bool) {
    if present {
        widget.as_ref().add_css_class(class);
    } else {
        widget.as_ref().remove_css_class(class);
    }
}

fn clear_state_classes(widget: &impl IsA<gtk::Widget>) {
    for class in STATE_CLASSES {
        widget.as_ref().remove_css_class(class);
    }
}

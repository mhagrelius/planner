//! A list of tasks: `ListStore` of [`TaskObject`], `ListView` of [`TaskRow`].
//!
//! Stickies got away with one window per record and needed none of this. A
//! planner cannot: the rows have to be recycled, so the item is a GObject and
//! the row binds to it.
//!
//! **Refreshing updates in place.** Clearing the store and refilling it would
//! be four lines shorter and would throw away the scroll position, the
//! selection, and the row that is halfway through an animation every time a
//! checkbox is ticked. Matching on ID and updating the objects that survive
//! costs a `HashMap` and keeps all three.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::model::store::Store;
use crate::model::task::Task;
use crate::model::{ProjectId, SectionId};
use crate::ui::task_object::TaskObject;
use crate::ui::task_row::TaskRow;

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct TaskList {
        pub store: RefCell<Option<gtk::gio::ListStore>>,
        pub view: RefCell<Option<gtk::ListView>>,
        pub empty: RefCell<Option<adw::StatusPage>>,
        pub stack: RefCell<Option<gtk::Stack>>,

        pub header: RefCell<Option<gtk::Box>>,
        pub header_title: RefCell<Option<gtk::Label>>,
        pub header_count: RefCell<Option<gtk::Label>>,
        pub header_menu: RefCell<Option<gtk::MenuButton>>,
        /// Which list this is, for a drop to know where it landed. `None`
        /// means a filter view — somewhere tasks are shown but do not live,
        /// so nothing can be dropped into it.
        pub group: RefCell<Option<(ProjectId, Option<SectionId>)>>,
        /// The row currently showing a drop hint, so it can be cleared when
        /// the pointer moves on. Without it a hint is left behind on every
        /// row the drag passed over.
        pub hinted: RefCell<Option<TaskRow>>,
        pub plain: RefCell<Option<gtk::NoSelection>>,
        pub multi: RefCell<Option<gtk::MultiSelection>>,
        pub selecting: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TaskList {
        const NAME: &'static str = "PlannerTaskList";
        type Type = super::TaskList;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for TaskList {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("task-toggled")
                        .param_types([str::static_type(), bool::static_type()])
                        .build(),
                    Signal::builder("task-activated")
                        .param_types([str::static_type()])
                        .build(),
                    // A task was dropped into this list at this position.
                    Signal::builder("task-dropped")
                        .param_types([str::static_type(), u32::static_type()])
                        .build(),
                ]
            })
        }
    }

    impl WidgetImpl for TaskList {}
    impl BoxImpl for TaskList {}
}

glib::wrapper! {
    pub struct TaskList(ObjectSubclass<imp::TaskList>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for TaskList {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskList {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build()
    }

    fn build(&self) {
        let imp = self.imp();

        let model = gtk::gio::ListStore::new::<TaskObject>();
        let factory = gtk::SignalListItemFactory::new();

        factory.connect_setup(glib::clone!(
            #[weak(rename_to = list)]
            self,
            move |_, object| {
                let item = object.downcast_ref::<gtk::ListItem>().expect("a list item");
                let row = TaskRow::new();
                row.connect_closure(
                    "toggled",
                    false,
                    glib::closure_local!(
                        #[watch]
                        list,
                        move |_: TaskRow, id: &str, checked: bool| {
                            list.emit_by_name::<()>("task-toggled", &[&id, &checked]);
                        }
                    ),
                );
                item.set_child(Some(&row));
            }
        ));

        factory.connect_bind(|_, object| {
            let item = object.downcast_ref::<gtk::ListItem>().expect("a list item");
            let row = item.child().and_downcast::<TaskRow>().expect("a task row");
            let task = item.item().and_downcast::<TaskObject>().expect("a task");
            row.bind(&task);
        });

        factory.connect_unbind(|_, object| {
            let item = object.downcast_ref::<gtk::ListItem>().expect("a list item");
            let row = item.child().and_downcast::<TaskRow>().expect("a task row");
            row.unbind();
        });

        // Two selection models, swapped rather than rebuilt: the list view
        // keeps its factory and its scroll position when selection mode goes
        // on and off, which it would not if the whole view were replaced.
        let plain = gtk::NoSelection::new(Some(model.clone()));
        let multi = gtk::MultiSelection::new(Some(model.clone()));
        let view = gtk::ListView::builder()
            .model(&plain)
            .factory(&factory)
            .single_click_activate(false)
            .build();
        view.add_css_class("navigation-sidebar");

        view.connect_activate(glib::clone!(
            #[weak(rename_to = list)]
            self,
            move |view, position| {
                let Some(item) = view
                    .model()
                    .and_then(|model| model.item(position))
                    .and_downcast::<TaskObject>()
                else {
                    return;
                };
                list.emit_by_name::<()>("task-activated", &[&item.id()]);
            }
        ));

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&view)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("object-select-symbolic")
            .title("Nothing to do")
            .description("Tasks matching this view will appear here.")
            .vexpand(true)
            .build();

        // Not homogeneous: a `GtkStack` sizes to its tallest child by
        // default, and the empty-state page is tall. A lane showing three
        // rows would otherwise reserve the height of an empty one, leaving a
        // chasm between every section in the list view.
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vhomogeneous(false)
            .hhomogeneous(false)
            .vexpand(true)
            .build();
        stack.add_named(&scroller, Some("tasks"));
        stack.add_named(&empty, Some("empty"));

        // A section header. Hidden unless this list is one section of several,
        // so a plain filter view is not given a redundant title above the
        // title already in the header bar.
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .visible(false)
            .build();
        let header_title = gtk::Label::builder().xalign(0.0).hexpand(true).build();
        header_title.add_css_class("heading");
        header.append(&header_title);
        let header_count = gtk::Label::new(None);
        header_count.add_css_class("dimmed");
        header_count.add_css_class("numeric");
        header.append(&header_count);
        // Empty until the header is given a menu: the unsectioned lane and a
        // filter view have nothing to put in one.
        let header_menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("Section Options")
            .css_classes(["flat"])
            .visible(false)
            .build();
        header.append(&header_menu);
        header.add_css_class("section-header");

        self.append(&header);
        self.append(&stack);
        self.install_drop_target();

        imp.header.replace(Some(header));
        imp.header_title.replace(Some(header_title));
        imp.header_count.replace(Some(header_count));
        imp.header_menu.replace(Some(header_menu));
        imp.plain.replace(Some(plain));
        imp.multi.replace(Some(multi));
        imp.store.replace(Some(model));
        imp.view.replace(Some(view));
        imp.empty.replace(Some(empty));
        imp.stack.replace(Some(stack));
    }

    /// Show these tasks, keeping the objects for tasks that were already here.
    pub fn set_tasks(&self, tasks: &[&Task], store: &Store, today: NaiveDate) {
        let model = self.imp().store.borrow().clone().expect("built");

        let existing: HashMap<String, TaskObject> = (0..model.n_items())
            .filter_map(|index| model.item(index).and_downcast::<TaskObject>())
            .map(|object| (object.id(), object))
            .collect();

        let objects: Vec<TaskObject> = tasks
            .iter()
            .map(|task| match existing.get(task.id.as_str()) {
                Some(object) => {
                    object.update(task, store, today);
                    object.clone()
                }
                None => TaskObject::from_task(task, store, today),
            })
            .collect();

        // One splice rather than a remove-and-insert per row: the view emits
        // a single `items-changed` and recycles what it can.
        model.splice(0, model.n_items(), &objects);

        self.set_empty(objects.is_empty());
        self.update_header_count();
    }

    fn set_empty(&self, empty: bool) {
        if let Some(stack) = self.imp().stack.borrow().as_ref() {
            stack.set_visible_child_name(if empty { "empty" } else { "tasks" });
        }
    }

    /// Change what the empty state says. Every view has a different reason to
    /// be empty and "Nothing to do" is only right for some of them.
    pub fn set_empty_state(&self, title: &str, description: &str, icon: &str) {
        if let Some(page) = self.imp().empty.borrow().as_ref() {
            page.set_title(title);
            page.set_description(Some(description));
            page.set_icon_name(Some(icon));
        }
    }

    /// Lay the list out as a page section or as a board column.
    ///
    /// A column is a fixed width and fills the height; a page section is full
    /// width and only as tall as its rows, so several stack down the page and
    /// the outer scroller does the scrolling rather than each list doing its
    /// own.
    pub fn apply_style(&self, style: crate::model::ViewStyle) {
        use crate::model::ViewStyle;

        let scroller = self
            .imp()
            .view
            .borrow()
            .as_ref()
            .and_then(|view| view.parent())
            .and_downcast::<gtk::ScrolledWindow>();

        match style {
            ViewStyle::Board => {
                self.set_width_request(300);
                self.set_hexpand(false);
                self.set_vexpand(true);
                self.add_css_class("board-column");
                if let Some(scroller) = scroller {
                    scroller.set_vexpand(true);
                    scroller.set_propagate_natural_height(false);
                    scroller.set_vscrollbar_policy(gtk::PolicyType::Automatic);
                }
                if let Some(empty) = self.imp().empty.borrow().as_ref() {
                    empty.set_vexpand(true);
                }
            }
            ViewStyle::List => {
                self.set_width_request(-1);
                self.set_hexpand(true);
                self.set_vexpand(false);
                self.remove_css_class("board-column");
                if let Some(scroller) = scroller {
                    // A lane down the page is only as tall as its rows; the
                    // project view's own scroller does the scrolling.
                    scroller.set_vexpand(false);
                    scroller.set_propagate_natural_height(true);
                    scroller.set_vscrollbar_policy(gtk::PolicyType::Never);
                }
                if let Some(empty) = self.imp().empty.borrow().as_ref() {
                    empty.set_vexpand(false);
                }
            }
        }
        if let Some(stack) = self.imp().stack.borrow().as_ref() {
            stack.set_vexpand(matches!(style, ViewStyle::Board));
        }
    }

    /// Say which list this is, so a drop knows where it landed.
    pub fn set_group(&self, project: ProjectId, section: Option<SectionId>) {
        self.imp().group.replace(Some((project, section)));
    }

    /// Which list this is, if it is one tasks can live in.
    pub fn group(&self) -> Option<(ProjectId, Option<SectionId>)> {
        self.imp().group.borrow().clone()
    }

    /// Show a section header above the list.
    pub fn set_header(&self, title: Option<&str>) {
        let imp = self.imp();
        if let Some(header) = imp.header.borrow().as_ref() {
            header.set_visible(title.is_some());
        }
        if let (Some(label), Some(title)) = (imp.header_title.borrow().as_ref(), title) {
            label.set_label(title);
        }
    }

    /// Put a menu on the section header. `None` takes it away again.
    pub fn set_header_menu(&self, menu: Option<&gtk::gio::MenuModel>) {
        if let Some(button) = self.imp().header_menu.borrow().as_ref() {
            button.set_menu_model(menu);
            button.set_visible(menu.is_some());
        }
    }

    /// The actions on the header menu, for tests.
    pub fn header_actions(&self) -> Vec<String> {
        let Some(menu) = self
            .imp()
            .header_menu
            .borrow()
            .as_ref()
            .and_then(|button| button.menu_model())
        else {
            return Vec::new();
        };
        (0..menu.n_items())
            .filter_map(|item| {
                menu.item_attribute_value(item, "action", Some(glib::VariantTy::STRING))
                    .and_then(|action| action.str().map(str::to_string))
            })
            .collect()
    }

    fn update_header_count(&self) {
        if let Some(label) = self.imp().header_count.borrow().as_ref() {
            let count = self.len();
            label.set_label(&if count == 0 {
                String::new()
            } else {
                count.to_string()
            });
        }
    }

    /// Accept dropped tasks, working out where they landed.
    ///
    /// The drop target is on the list rather than on each row because a row is
    /// recycled: a controller added in the factory's `setup` would outlive the
    /// item it was added for. Working from the pointer position means one
    /// controller, and it is also the only way to drop into the empty space
    /// below the last row — which is the whole of an empty section.
    fn install_drop_target(&self) {
        let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);

        target.connect_motion(glib::clone!(
            #[weak(rename_to = list)]
            self,
            #[upgrade_or]
            gtk::gdk::DragAction::empty(),
            move |_, _, y| {
                if list.group().is_none() {
                    return gtk::gdk::DragAction::empty();
                }
                list.show_drop_hint(y);
                gtk::gdk::DragAction::MOVE
            }
        ));

        target.connect_leave(glib::clone!(
            #[weak(rename_to = list)]
            self,
            move |_| list.clear_drop_hint()
        ));

        target.connect_drop(glib::clone!(
            #[weak(rename_to = list)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, y| {
                list.clear_drop_hint();
                if list.group().is_none() {
                    return false;
                }
                let Ok(id) = value.get::<String>() else {
                    return false;
                };
                let index = list.drop_index(y, &id);
                list.emit_by_name::<()>("task-dropped", &[&id, &index]);
                true
            }
        ));

        self.add_controller(target);
    }

    /// Where in this list a drop at height `y` would land.
    ///
    /// Above the midpoint of a row means before it, below means after. The
    /// dragged task is discounted when it is already in this list, so the
    /// index is one into the list *without* it — which is what
    /// [`Store::move_task`](crate::model::store::Store::move_task) expects.
    fn drop_index(&self, y: f64, dragged: &str) -> u32 {
        insertion_index(&self.row_geometry(dragged), y)
    }

    /// Draw the line showing where a drop would land.
    fn show_drop_hint(&self, y: f64) {
        self.clear_drop_hint();

        let Some((position, before)) = hint_position(&self.row_geometry(""), y) else {
            return;
        };
        if let Some(row) = self.row_at(position as u32) {
            row.set_drop_hint(before);
            self.imp().hinted.replace(Some(row));
        }
    }

    /// Where each realised row sits, and whether it is the one being dragged.
    fn row_geometry(&self, dragged: &str) -> Vec<Row> {
        (0..self.len())
            .filter_map(|position| {
                let row = self.row_at(position)?;
                Some(Row {
                    top: self.row_top(&row),
                    height: row.height() as f64,
                    is_dragged: self.item(position).is_some_and(|item| item.id() == dragged),
                })
            })
            .collect()
    }

    fn clear_drop_hint(&self) {
        if let Some(row) = self.imp().hinted.take() {
            row.clear_drop_hint();
        }
    }

    /// The row widget showing item `position`, if it is realised.
    fn row_at(&self, position: u32) -> Option<TaskRow> {
        let view = self.imp().view.borrow().clone()?;
        let mut child = view.first_child();
        let mut index = 0;
        while let Some(current) = child {
            if let Some(row) = current.first_child().and_downcast::<TaskRow>() {
                if index == position {
                    return Some(row);
                }
                index += 1;
            }
            child = current.next_sibling();
        }
        None
    }

    /// A row's top edge, in this list's coordinates.
    fn row_top(&self, row: &TaskRow) -> f64 {
        row.compute_point(self, &gtk::graphene::Point::new(0.0, 0.0))
            .map(|point| point.y() as f64)
            .unwrap_or(0.0)
    }

    /// Turn selection mode on or off.
    pub fn set_selecting(&self, selecting: bool) {
        let imp = self.imp();
        imp.selecting.set(selecting);
        let Some(view) = imp.view.borrow().clone() else {
            return;
        };
        if selecting {
            if let Some(multi) = imp.multi.borrow().as_ref() {
                multi.unselect_all();
                view.set_model(Some(multi));
            }
        } else if let Some(plain) = imp.plain.borrow().as_ref() {
            view.set_model(Some(plain));
        }
    }

    /// Whether selection mode is on.
    pub fn is_selecting(&self) -> bool {
        self.imp().selecting.get()
    }

    /// The tasks currently selected.
    pub fn selected(&self) -> Vec<crate::model::TaskId> {
        if !self.is_selecting() {
            return Vec::new();
        }
        let Some(multi) = self.imp().multi.borrow().clone() else {
            return Vec::new();
        };
        (0..multi.n_items())
            .filter(|position| multi.is_selected(*position))
            .filter_map(|position| self.item(position))
            .map(|item| item.task_id())
            .collect()
    }

    /// Called whenever the selection changes.
    pub fn connect_selection_changed(&self, changed: impl Fn() + 'static) {
        if let Some(multi) = self.imp().multi.borrow().as_ref() {
            multi.connect_selection_changed(move |_, _, _| changed());
        }
    }

    /// The task object at a position, as the view holds it.
    pub fn item(&self, index: u32) -> Option<TaskObject> {
        self.imp()
            .store
            .borrow()
            .as_ref()
            .and_then(|model| model.item(index))
            .and_downcast::<TaskObject>()
    }

    /// How many tasks are showing.
    pub fn len(&self) -> u32 {
        self.imp()
            .store
            .borrow()
            .as_ref()
            .map_or(0, |model| model.n_items())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One row's place in a list, for working out where a drop lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    pub top: f64,
    pub height: f64,
    /// Whether this is the row being dragged.
    pub is_dragged: bool,
}

/// Which position in the list a drop at height `y` means.
///
/// Above a row's midpoint means before it, below means after. The dragged row
/// is not counted, because the index is a position in the list *without* it —
/// which is what [`Store::move_task`](crate::model::store::Store::move_task)
/// takes, and is the difference between dragging a task to the bottom and
/// dragging it to one short of the bottom.
pub fn insertion_index(rows: &[Row], y: f64) -> u32 {
    let mut index = 0;
    for row in rows {
        if y < row.top + row.height / 2.0 {
            return index;
        }
        if !row.is_dragged {
            index += 1;
        }
    }
    index
}

/// Which row to draw the drop line on, and whether it goes above it.
///
/// `None` when there are no rows: an empty section has nothing to draw a line
/// against, and the drop still works.
pub fn hint_position(rows: &[Row], y: f64) -> Option<(usize, bool)> {
    for (position, row) in rows.iter().enumerate() {
        if y < row.top + row.height {
            return Some((position, y < row.top + row.height / 2.0));
        }
    }
    // Past the last row: the line goes under it.
    rows.len().checked_sub(1).map(|last| (last, false))
}

#[cfg(test)]
mod arithmetic_tests {
    use super::*;

    /// Three rows, 40px each, stacked from the top. The middle one is being
    /// dragged in the cases that say so.
    fn rows(dragged: Option<usize>) -> Vec<Row> {
        (0..3)
            .map(|index| Row {
                top: index as f64 * 40.0,
                height: 40.0,
                is_dragged: dragged == Some(index),
            })
            .collect()
    }

    #[test]
    fn a_drop_above_the_first_midpoint_goes_to_the_top() {
        assert_eq!(insertion_index(&rows(None), 0.0), 0);
        assert_eq!(insertion_index(&rows(None), 19.0), 0);
    }

    #[test]
    fn a_drop_below_a_midpoint_goes_after_that_row() {
        assert_eq!(insertion_index(&rows(None), 21.0), 1);
        assert_eq!(insertion_index(&rows(None), 61.0), 2);
    }

    #[test]
    fn a_drop_past_the_last_row_goes_to_the_end() {
        assert_eq!(insertion_index(&rows(None), 500.0), 3);
    }

    #[test]
    fn the_dragged_row_does_not_count_towards_the_position() {
        // Dragging the middle row to the bottom. Counting it would give 2 —
        // which puts it back where it started rather than at the end.
        let rows = rows(Some(1));
        assert_eq!(insertion_index(&rows, 500.0), 2);
        // And dropping it just below the last row's midpoint is the same
        // place: the end of a two-item list.
        assert_eq!(insertion_index(&rows, 101.0), 2);
    }

    #[test]
    fn dragging_the_first_row_downwards_lands_where_it_looks_like_it_will() {
        let rows = rows(Some(0));
        // Below the second row's midpoint: after it, in a list of two.
        assert_eq!(insertion_index(&rows, 61.0), 1);
        assert_eq!(insertion_index(&rows, 500.0), 2);
    }

    #[test]
    fn an_empty_list_takes_a_drop_at_position_zero() {
        assert_eq!(insertion_index(&[], 42.0), 0);
        assert_eq!(hint_position(&[], 42.0), None);
    }

    #[test]
    fn the_hint_sits_above_or_below_the_row_the_pointer_is_over() {
        assert_eq!(hint_position(&rows(None), 5.0), Some((0, true)));
        assert_eq!(hint_position(&rows(None), 35.0), Some((0, false)));
        assert_eq!(hint_position(&rows(None), 45.0), Some((1, true)));
    }

    #[test]
    fn past_the_end_the_hint_goes_under_the_last_row() {
        assert_eq!(hint_position(&rows(None), 500.0), Some((2, false)));
    }
}

//! A project, as a list of sections or as a board of them.
//!
//! Both layouts are the same widgets: one [`TaskList`] per section, laid out
//! down the page or across it.
//!
//! GTK 4.12 can section a single `ListView` — a `GtkSortListModel` with a
//! section sorter, plus a header factory — and that keeps one recycling list
//! for the whole project. But a section with no tasks in it has no rows, so it
//! has no header, so it is not on screen; and a section you cannot see is a
//! section you cannot drag anything into. Empty sections are exactly what you
//! have when you have just made one. So: a list per section, which shows an
//! empty section, gives every drop target its own destination without working
//! any of it out from coordinates, and makes the board a change of
//! orientation rather than a second implementation.
//!
//! The cost is several `ListView`s instead of one. Each still recycles its own
//! rows, and a project with more sections than fit on screen is a different
//! problem from a project with more tasks than fit in memory.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use chrono::NaiveDate;

use crate::model::store::Store;
use crate::model::{ProjectId, SectionId, ViewStyle};
use crate::ui::task_list::TaskList;

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct ProjectView {
        pub scroller: RefCell<Option<gtk::ScrolledWindow>>,
        pub lanes: RefCell<Option<gtk::Box>>,
        /// One list per lane, in display order, with the section it stands
        /// for. The first is always the unsectioned one.
        pub lists: RefCell<Vec<(Option<SectionId>, TaskList)>>,
        pub project: RefCell<Option<ProjectId>>,
        pub style: Cell<ViewStyle>,
        pub selecting: Cell<bool>,
        pub selection_changed: RefCell<Option<Box<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectView {
        const NAME: &'static str = "PlannerProjectView";
        type Type = super::ProjectView;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for ProjectView {
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
                    // Task ID, destination section (empty for none), position.
                    Signal::builder("task-moved")
                        .param_types([str::static_type(), str::static_type(), u32::static_type()])
                        .build(),
                    Signal::builder("section-requested").build(),
                ]
            })
        }
    }

    impl WidgetImpl for ProjectView {}
    impl BoxImpl for ProjectView {}
}

glib::wrapper! {
    pub struct ProjectView(ObjectSubclass<imp::ProjectView>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for ProjectView {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectView {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build()
    }

    fn build(&self) {
        let lanes = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(6)
            .margin_bottom(18)
            .margin_start(6)
            .margin_end(6)
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&lanes)
            .build();
        self.append(&scroller);

        self.imp().scroller.replace(Some(scroller));
        self.imp().lanes.replace(Some(lanes));
    }

    /// Rebuild for a project.
    pub fn show_project(
        &self,
        project: &ProjectId,
        style: ViewStyle,
        store: &Store,
        today: NaiveDate,
    ) {
        let imp = self.imp();
        let Some(lanes) = imp.lanes.borrow().clone() else {
            return;
        };
        if store.project(project).is_none() {
            return;
        }

        let sections: Vec<(Option<SectionId>, String)> = std::iter::once((None, String::new()))
            .chain(
                store
                    .sections_in(project)
                    .into_iter()
                    .map(|section| (Some(section.id.clone()), section.name.clone())),
            )
            .collect();

        // Rebuild the lanes only when the sections themselves changed.
        // Ticking a checkbox must not throw away every list and its scroll
        // position, and it fires this path like everything else does.
        let same = {
            let existing = imp.lists.borrow();
            existing.len() == sections.len()
                && existing
                    .iter()
                    .zip(&sections)
                    .all(|((id, _), (wanted, _))| id == wanted)
        };

        if !same || imp.project.borrow().as_ref() != Some(project) {
            while let Some(child) = lanes.first_child() {
                lanes.remove(&child);
            }
            let mut lists = Vec::new();
            for (section, name) in &sections {
                let list = self.build_lane(project, section.clone(), name, style);
                lanes.append(&list);
                lists.push((section.clone(), list));
            }
            imp.lists.replace(lists);
            imp.project.replace(Some(project.clone()));
        }

        self.set_style(style);

        for (section, list) in imp.lists.borrow().iter() {
            let tasks = store.tasks_in(project, section.as_ref());
            list.set_tasks(&tasks, store, today);
        }
    }

    /// One lane: a list with its own header, drop target and signals.
    fn build_lane(
        &self,
        project: &ProjectId,
        section: Option<SectionId>,
        name: &str,
        style: ViewStyle,
    ) -> TaskList {
        let list = TaskList::new();
        list.set_group(project.clone(), section.clone());

        // The unsectioned lane has no name. In a list it needs no header
        // either — its tasks simply start at the top, which is what a project
        // with no sections should look like. On a board it needs a column
        // heading like every other column, or it is an anonymous first pile.
        let header = match (&section, style) {
            (Some(_), _) => Some(name.to_string()),
            (None, ViewStyle::Board) => Some("No section".to_string()),
            (None, ViewStyle::List) => None,
        };
        list.set_header(header.as_deref());
        list.set_header_menu(section_menu(section.as_ref()).as_ref());
        list.set_empty_state(
            "Nothing here",
            "Drag a task in, or add one.",
            "list-add-symbolic",
        );

        list.connect_closure(
            "task-toggled",
            false,
            glib::closure_local!(
                #[watch(rename_to = view)]
                self,
                move |_: TaskList, id: &str, checked: bool| {
                    view.emit_by_name::<()>("task-toggled", &[&id, &checked]);
                }
            ),
        );
        list.connect_closure(
            "task-activated",
            false,
            glib::closure_local!(
                #[watch(rename_to = view)]
                self,
                move |_: TaskList, id: &str| {
                    view.emit_by_name::<()>("task-activated", &[&id]);
                }
            ),
        );
        list.connect_closure(
            "task-dropped",
            false,
            glib::closure_local!(
                #[watch(rename_to = view)]
                self,
                move |list: TaskList, id: &str, index: u32| {
                    let section = list
                        .group()
                        .and_then(|(_, section)| section)
                        .map(|id| id.to_string())
                        .unwrap_or_default();
                    view.emit_by_name::<()>("task-moved", &[&id, &section, &index]);
                }
            ),
        );

        list.connect_selection_changed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || view.report_selection_changed()
        ));

        // A lane built while selection mode is on joins it, rather than being
        // the one column you cannot select anything in.
        list.set_selecting(self.imp().selecting.get());
        list.apply_style(style);
        list
    }

    /// Switch between down-the-page and across-the-page.
    fn set_style(&self, style: ViewStyle) {
        let imp = self.imp();
        let Some(lanes) = imp.lanes.borrow().clone() else {
            return;
        };
        let Some(scroller) = imp.scroller.borrow().clone() else {
            return;
        };
        imp.style.set(style);

        match style {
            ViewStyle::List => {
                lanes.set_orientation(gtk::Orientation::Vertical);
                scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
                scroller.set_vscrollbar_policy(gtk::PolicyType::Automatic);
            }
            ViewStyle::Board => {
                lanes.set_orientation(gtk::Orientation::Horizontal);
                scroller.set_hscrollbar_policy(gtk::PolicyType::Automatic);
                scroller.set_vscrollbar_policy(gtk::PolicyType::Never);
            }
        }

        for (_, list) in imp.lists.borrow().iter() {
            list.apply_style(style);
        }
    }

    /// Turn selection mode on or off in every lane.
    pub fn set_selecting(&self, selecting: bool) {
        self.imp().selecting.set(selecting);
        for (_, list) in self.imp().lists.borrow().iter() {
            list.set_selecting(selecting);
        }
    }

    /// Called whenever the selection changes in any lane.
    pub fn connect_selection_changed(&self, changed: impl Fn() + 'static) {
        self.imp()
            .selection_changed
            .replace(Some(Box::new(changed)));
    }

    fn report_selection_changed(&self) {
        if let Some(changed) = self.imp().selection_changed.borrow().as_ref() {
            changed();
        }
    }

    /// Every task selected, across every lane.
    pub fn selected(&self) -> Vec<crate::model::TaskId> {
        self.imp()
            .lists
            .borrow()
            .iter()
            .flat_map(|(_, list)| list.selected())
            .collect()
    }

    /// How many lanes are showing, for tests.
    pub fn lane_count(&self) -> usize {
        self.imp().lists.borrow().len()
    }

    /// The list for a section, for tests.
    pub fn lane(&self, section: Option<&SectionId>) -> Option<TaskList> {
        self.imp()
            .lists
            .borrow()
            .iter()
            .find(|(id, _)| id.as_ref() == section)
            .map(|(_, list)| list.clone())
    }

    /// The layout currently in use.
    pub fn style(&self) -> ViewStyle {
        self.imp().style.get()
    }
}

/// The menu on a section's header, aimed at that section.
///
/// The target is set as a variant rather than written into a detailed action
/// name: an ID is opaque, and `win.rename-section::…` would have to escape
/// whatever ends up in one.
fn section_menu(section: Option<&SectionId>) -> Option<gtk::gio::MenuModel> {
    let id = section?.to_string();
    let menu = gtk::gio::Menu::new();
    for (label, action) in [
        ("Rename Section…", "win.rename-section"),
        ("Delete Section…", "win.delete-section"),
    ] {
        let item = gtk::gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some(action), Some(&id.to_variant()));
        menu.append_item(&item);
    }
    Some(menu.upcast())
}

//! The window: two split views, a breakpoint, and the task list between them.
//!
//! The structure is Planify's, because it is the right one. The outer
//! `AdwOverlaySplitView` holds the navigation sidebar; a second one, packed at
//! the `End`, holds the task detail panel at 360px; one `AdwBreakpoint` at
//! `675sp` collapses both. Note that neither is an `AdwNavigationSplitView` —
//! a detail panel is a utility pane that should overlay when there is no room,
//! not a page you navigate into and have to come back from.
//!
//! The window owns no data. It asks the application what to show and tells it
//! what the user did.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::model::store::LoadOutcome;
use crate::model::{SectionId, TaskId};
use crate::ui::application::PlannerApplication;
use crate::ui::detail_panel::{DetailPanel, Edit};
use crate::ui::project_view::ProjectView;
use crate::ui::quick_add::QuickAddDialog;
use crate::ui::quick_find::QuickFindDialog;
use crate::ui::sidebar::Sidebar;
use crate::ui::task_list::TaskList;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct PlannerWindow {
        pub sidebar: RefCell<Option<Sidebar>>,
        pub tasks: RefCell<Option<TaskList>>,
        pub detail: RefCell<Option<DetailPanel>>,
        pub project_view: RefCell<Option<ProjectView>>,
        /// Swaps between the flat list a filter view shows and the sectioned
        /// project view.
        pub content_stack: RefCell<Option<gtk::Stack>>,
        pub style_toggle: RefCell<Option<adw::ToggleGroup>>,
        pub project_menu: RefCell<Option<gtk::MenuButton>>,
        pub filter_button: RefCell<Option<gtk::Button>>,
        pub select_toggle: RefCell<Option<gtk::ToggleButton>>,
        pub last_view: RefCell<Option<String>>,
        pub action_bar: RefCell<Option<gtk::ActionBar>>,
        pub selection_label: RefCell<Option<gtk::Label>>,
        pub title: RefCell<Option<adw::WindowTitle>>,
        pub toasts: RefCell<Option<adw::ToastOverlay>>,
        pub banner: RefCell<Option<adw::Banner>>,
        pub outer_split: RefCell<Option<adw::OverlaySplitView>>,
        pub detail_split: RefCell<Option<adw::OverlaySplitView>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PlannerWindow {
        const NAME: &'static str = "PlannerWindow";
        type Type = super::PlannerWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for PlannerWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_default_size(1000, 700);
            obj.set_title(Some("Planner"));
            obj.build();
            obj.install_actions();
            obj.refresh();
            obj.report_load_outcome();
        }
    }

    impl WidgetImpl for PlannerWindow {}
    impl WindowImpl for PlannerWindow {
        fn close_request(&self) -> glib::Propagation {
            // The tick may be up to two seconds from firing; closing must not
            // be one of the ways to lose the last edit.
            if let Some(app) = self.obj().planner_application() {
                app.save_now();
            }
            self.parent_close_request()
        }
    }
    impl ApplicationWindowImpl for PlannerWindow {}
    impl AdwApplicationWindowImpl for PlannerWindow {}
}

glib::wrapper! {
    pub struct PlannerWindow(ObjectSubclass<imp::PlannerWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::gio::ActionGroup, gtk::gio::ActionMap, gtk::Accessible,
                    gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::Root,
                    gtk::ShortcutManager;
}

/// "1 task" or "4 tasks".
fn count_of(count: usize) -> String {
    if count == 1 {
        "1 task".to_string()
    } else {
        format!("{count} tasks")
    }
}

/// What the filter editor tells you about the query language.
const FILTER_HINT: &str = "Combine with & | ! ( ) — for example \
                           `p1 & due before: next week`, `@errand | @town`, \
                           `##Work & !subtask`.";

impl PlannerWindow {
    pub fn new(app: &PlannerApplication) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    fn planner_application(&self) -> Option<PlannerApplication> {
        self.application().and_downcast::<PlannerApplication>()
    }

    fn build(&self) {
        let imp = self.imp();

        // --- the sidebar side -------------------------------------------
        let sidebar = Sidebar::new();
        let sidebar_header = adw::HeaderBar::builder()
            .show_title(false)
            .css_classes(["flat"])
            .build();

        let menu = gtk::gio::Menu::new();
        let create = gtk::gio::Menu::new();
        create.append(Some("New Project…"), Some("win.new-project"));
        create.append(Some("New Filter…"), Some("win.new-filter"));
        menu.append_section(None, &create);
        let app_items = gtk::gio::Menu::new();
        app_items.append(Some("Quick Find"), Some("win.find"));
        app_items.append(Some("About Planner"), Some("app.about"));
        app_items.append(Some("Quit"), Some("app.quit"));
        menu.append_section(None, &app_items);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Main Menu")
            .menu_model(&menu)
            .build();
        sidebar_header.pack_end(&menu_button);

        let new_project_button = gtk::Button::builder()
            .icon_name("folder-new-symbolic")
            .tooltip_text("New Project")
            .action_name("win.new-project")
            .build();
        sidebar_header.pack_start(&new_project_button);

        let sidebar_pane = adw::ToolbarView::builder()
            .top_bar_style(adw::ToolbarStyle::Flat)
            .content(&sidebar)
            .build();
        sidebar_pane.add_top_bar(&sidebar_header);

        // --- the content side -------------------------------------------
        let title = adw::WindowTitle::new("Today", "");
        let content_header = adw::HeaderBar::builder()
            .title_widget(&title)
            .css_classes(["flat"])
            .build();

        let sidebar_toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle Sidebar")
            .active(true)
            .build();
        content_header.pack_start(&sidebar_toggle);

        let new_task = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("New Task")
            .action_name("win.new-task")
            .build();
        content_header.pack_end(&new_task);

        let project_menu_model = gtk::gio::Menu::new();
        let structure = gtk::gio::Menu::new();
        structure.append(Some("Add Section…"), Some("win.add-section"));
        structure.append(Some("Add Subproject…"), Some("win.new-subproject"));
        project_menu_model.append_section(None, &structure);
        let project_items = gtk::gio::Menu::new();
        project_items.append(Some("Rename Project…"), Some("win.rename-project"));
        project_items.append(Some("Delete Project…"), Some("win.delete-project"));
        project_menu_model.append_section(None, &project_items);
        let project_menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("Project Options")
            .menu_model(&project_menu_model)
            .visible(false)
            .build();
        content_header.pack_end(&project_menu);

        let filter_button = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Edit Filter")
            .action_name("win.edit-filter")
            .visible(false)
            .build();
        content_header.pack_end(&filter_button);

        // Only meaningful in a project: a filter view has no sections to make
        // columns out of. Hidden rather than insensitive, because a control
        // that is never usable in this view is not a control this view has.
        let style_toggle = adw::ToggleGroup::builder().visible(false).build();
        style_toggle.add(
            adw::Toggle::builder()
                .name("list")
                .icon_name("view-list-symbolic")
                .tooltip("List")
                .build(),
        );
        style_toggle.add(
            adw::Toggle::builder()
                .name("board")
                .icon_name("view-grid-symbolic")
                .tooltip("Board")
                .build(),
        );
        style_toggle.connect_active_name_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |group| {
                let style = match group.active_name().as_deref() {
                    Some("board") => crate::model::ViewStyle::Board,
                    _ => crate::model::ViewStyle::List,
                };
                window.on_style_changed(style);
            }
        ));
        content_header.pack_end(&style_toggle);

        let tasks = TaskList::new();
        tasks.connect_closure(
            "task-toggled",
            false,
            glib::closure_local!(
                #[watch(rename_to = window)]
                self,
                move |_: TaskList, id: &str, checked: bool| {
                    window.on_task_toggled(id, checked);
                }
            ),
        );

        tasks.connect_closure(
            "task-activated",
            false,
            glib::closure_local!(
                #[watch(rename_to = window)]
                self,
                move |_: TaskList, id: &str| {
                    window.open_task(&TaskId::from_raw(id));
                }
            ),
        );

        let project_view = ProjectView::new();
        project_view.connect_closure(
            "task-toggled",
            false,
            glib::closure_local!(
                #[watch(rename_to = window)]
                self,
                move |_: ProjectView, id: &str, checked: bool| {
                    window.on_task_toggled(id, checked);
                }
            ),
        );
        project_view.connect_closure(
            "task-activated",
            false,
            glib::closure_local!(
                #[watch(rename_to = window)]
                self,
                move |_: ProjectView, id: &str| {
                    window.open_task(&TaskId::from_raw(id));
                }
            ),
        );
        project_view.connect_selection_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.update_selection_label()
        ));
        project_view.connect_closure(
            "task-moved",
            false,
            glib::closure_local!(
                #[watch(rename_to = window)]
                self,
                move |_: ProjectView, id: &str, section: &str, index: u32| {
                    window.on_task_moved(id, section, index);
                }
            ),
        );

        let select_toggle = gtk::ToggleButton::builder()
            .icon_name("selection-mode-symbolic")
            .tooltip_text("Select Tasks")
            .build();
        select_toggle.connect_toggled(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |toggle| window.set_selecting(toggle.is_active())
        ));
        content_header.pack_end(&select_toggle);

        tasks.connect_selection_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.update_selection_label()
        ));

        let content_stack = gtk::Stack::builder().vexpand(true).build();
        content_stack.add_named(&tasks, Some("flat"));
        content_stack.add_named(&project_view, Some("project"));

        let banner = adw::Banner::builder().revealed(false).build();

        let content_body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        content_body.append(&banner);
        content_body.append(&content_stack);

        let content_pane = adw::ToolbarView::builder()
            .top_bar_style(adw::ToolbarStyle::Flat)
            .content(&content_body)
            .build();
        content_pane.add_top_bar(&content_header);

        let (action_bar, selection_label) = self.build_action_bar();
        content_pane.add_bottom_bar(&action_bar);

        // --- the detail panel, packed at the end ------------------------
        let detail = DetailPanel::new();
        detail.connect_edited(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |edit| window.on_detail_edited(edit)
        ));
        let detail_pane = &detail;

        let detail_split = adw::OverlaySplitView::builder()
            .sidebar_position(gtk::PackType::End)
            .min_sidebar_width(360.0)
            .max_sidebar_width(360.0)
            .collapsed(true)
            .show_sidebar(false)
            .content(&content_pane)
            .sidebar(detail_pane)
            .build();

        let outer_split = adw::OverlaySplitView::builder()
            .min_sidebar_width(240.0)
            .max_sidebar_width(300.0)
            .sidebar(&sidebar_pane)
            .content(&detail_split)
            .build();

        sidebar_toggle
            .bind_property("active", &outer_split, "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();

        sidebar.connect_closure(
            "view-selected",
            false,
            glib::closure_local!(
                #[watch(rename_to = window)]
                self,
                move |_: Sidebar, _id: &str| window.refresh_content()
            ),
        );

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&outer_split));
        self.set_content(Some(&toasts));

        // Narrow: the navigation sidebar overlays instead of sitting beside
        // the list. Set on the window, never by swapping widget trees.
        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            675.0,
            adw::LengthUnit::Sp,
        ));
        breakpoint.add_setter(&outer_split, "collapsed", Some(&true.into()));
        breakpoint.add_setter(&sidebar_toggle, "active", Some(&false.into()));
        self.add_breakpoint(breakpoint);

        imp.detail.replace(Some(detail));
        imp.project_view.replace(Some(project_view));
        imp.content_stack.replace(Some(content_stack));
        imp.style_toggle.replace(Some(style_toggle));
        imp.project_menu.replace(Some(project_menu));
        imp.filter_button.replace(Some(filter_button));
        imp.select_toggle.replace(Some(select_toggle));
        imp.action_bar.replace(Some(action_bar));
        imp.selection_label.replace(Some(selection_label));
        imp.sidebar.replace(Some(sidebar));
        imp.tasks.replace(Some(tasks));
        imp.title.replace(Some(title));
        imp.toasts.replace(Some(toasts));
        imp.banner.replace(Some(banner));
        imp.outer_split.replace(Some(outer_split));
        imp.detail_split.replace(Some(detail_split));
    }

    fn install_actions(&self) {
        let actions = gtk::gio::SimpleActionGroup::new();

        let new_task = gtk::gio::SimpleAction::new("new-task", None);
        new_task.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.open_quick_add()
        ));
        actions.add_action(&new_task);

        let find = gtk::gio::SimpleAction::new("find", None);
        find.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.open_quick_find()
        ));
        actions.add_action(&find);

        let new_filter = gtk::gio::SimpleAction::new("new-filter", None);
        new_filter.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.edit_filter(None)
        ));
        actions.add_action(&new_filter);

        let edit_filter = gtk::gio::SimpleAction::new("edit-filter", None);
        edit_filter.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let existing = window
                    .imp()
                    .sidebar
                    .borrow()
                    .as_ref()
                    .and_then(|sidebar| sidebar.selected_view())
                    .and_then(|view| view.filter_id());
                window.edit_filter(existing);
            }
        ));
        actions.add_action(&edit_filter);

        let new_project = gtk::gio::SimpleAction::new("new-project", None);
        new_project.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.prompt_for_project(None)
        ));
        actions.add_action(&new_project);

        let new_subproject = gtk::gio::SimpleAction::new("new-subproject", None);
        new_subproject.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let parent = window.current_project();
                window.prompt_for_project(parent);
            }
        ));
        actions.add_action(&new_subproject);

        let rename_project = gtk::gio::SimpleAction::new("rename-project", None);
        rename_project.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.prompt_to_rename_project()
        ));
        actions.add_action(&rename_project);

        let delete_project = gtk::gio::SimpleAction::new("delete-project", None);
        delete_project.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.confirm_delete_project()
        ));
        actions.add_action(&delete_project);

        let add_section = gtk::gio::SimpleAction::new("add-section", None);
        add_section.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.prompt_for_section()
        ));
        actions.add_action(&add_section);

        // Both take the section they act on: the menu they come from is on a
        // section header, not on the window, so there is no "current" one.
        let rename_section =
            gtk::gio::SimpleAction::new("rename-section", Some(glib::VariantTy::STRING));
        rename_section.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, target| {
                if let Some(id) = target.and_then(|target| target.str()) {
                    window.prompt_to_rename_section(&SectionId::from_raw(id));
                }
            }
        ));
        actions.add_action(&rename_section);

        let delete_section =
            gtk::gio::SimpleAction::new("delete-section", Some(glib::VariantTy::STRING));
        delete_section.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, target| {
                if let Some(id) = target.and_then(|target| target.str()) {
                    window.delete_section(&SectionId::from_raw(id));
                }
            }
        ));
        actions.add_action(&delete_section);

        let toggle_sidebar = gtk::gio::SimpleAction::new("toggle-sidebar", None);
        toggle_sidebar.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(split) = window.imp().outer_split.borrow().as_ref() {
                    split.set_show_sidebar(!split.shows_sidebar());
                }
            }
        ));
        actions.add_action(&toggle_sidebar);

        self.insert_action_group("win", Some(&actions));
    }

    /// Rebuild the sidebar and the list. Called whenever the store changes.
    pub fn refresh(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let today = crate::ui::today();

        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            app.with_store(|store| sidebar.refresh(store, today));
        }
        self.refresh_content();
        self.refresh_detail();
        self.set_save_error(app.save_error());
    }

    /// Re-read the open task, if there is one.
    fn refresh_detail(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let Some(detail) = self.imp().detail.borrow().clone() else {
            return;
        };
        let Some(id) = detail.task_id() else {
            return;
        };

        let today = crate::ui::today();
        let exists = app.with_store(|store| {
            let exists = store.task(&id).is_some();
            if exists {
                detail.show(&id, store, today);
            }
            exists
        });
        // The task went away — deleted here, or completed out of a filter that
        // the panel is now the only thing still showing it in.
        if !exists {
            self.close_detail();
        }
    }

    /// Rebuild just the task list, for when the selected view changes.
    fn refresh_content(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let imp = self.imp();
        let (Some(sidebar), Some(tasks)) =
            (imp.sidebar.borrow().clone(), imp.tasks.borrow().clone())
        else {
            return;
        };
        let Some(view) = sidebar.selected_view() else {
            return;
        };
        let today = crate::ui::today();

        if let Some(title) = imp.title.borrow().as_ref() {
            title.set_title(&view.title);
        }

        // A selection belongs to the list it was made in. Carrying it to the
        // next view would mean a bulk delete acting on tasks that are no
        // longer on screen.
        if let Some(toggle) = imp.select_toggle.borrow().as_ref() {
            if toggle.is_active() && imp.last_view.borrow().as_deref() != Some(view.id.as_str()) {
                toggle.set_active(false);
            }
        }
        imp.last_view.replace(Some(view.id.clone()));

        // A project shows its sections; a filter shows one flat list. The
        // controls that only make sense for one of them follow.
        let project = view
            .project_id()
            .filter(|id| app.with_store(|store| store.project(id).is_some()));
        let is_project = project.is_some();

        if let Some(toggle) = imp.style_toggle.borrow().as_ref() {
            toggle.set_visible(is_project);
        }
        if let Some(menu) = imp.project_menu.borrow().as_ref() {
            menu.set_visible(is_project);
        }
        if let Some(button) = imp.filter_button.borrow().as_ref() {
            button.set_visible(view.filter_id().is_some());
        }

        let count = match &project {
            Some(id) => {
                let Some(project_view) = imp.project_view.borrow().clone() else {
                    return;
                };
                let style = app.with_store(|store| {
                    store
                        .project(id)
                        .map(|project| project.view_style)
                        .unwrap_or_default()
                });
                if let Some(toggle) = imp.style_toggle.borrow().as_ref() {
                    toggle.set_active_name(Some(match style {
                        crate::model::ViewStyle::Board => "board",
                        crate::model::ViewStyle::List => "list",
                    }));
                }
                app.with_store(|store| project_view.show_project(id, style, store, today));
                if let Some(stack) = imp.content_stack.borrow().as_ref() {
                    stack.set_visible_child_name("project");
                }
                app.with_store(|store| store.progress(id))
            }
            None => {
                tasks.set_empty_state(
                    view.empty_title,
                    view.empty_description,
                    "object-select-symbolic",
                );
                app.with_store(|store| {
                    let matching = store.query(&view.query(), today);
                    tasks.set_tasks(&matching, store, today);
                });
                if let Some(stack) = imp.content_stack.borrow().as_ref() {
                    stack.set_visible_child_name("flat");
                }
                (0, tasks.len() as usize)
            }
        };

        if let Some(title) = imp.title.borrow().as_ref() {
            let (done, total) = count;
            title.set_subtitle(&match (is_project, total) {
                (_, 0) => String::new(),
                // A project says how far through it is; a filter has no
                // "through" to be — a task leaves Today by being done.
                (true, total) => format!("{done} of {total} done"),
                (false, 1) => "1 task".to_string(),
                (false, total) => format!("{total} tasks"),
            });
        }
    }

    fn on_style_changed(&self, style: crate::model::ViewStyle) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let Some(project) = self
            .imp()
            .sidebar
            .borrow()
            .as_ref()
            .and_then(|sidebar| sidebar.selected_view())
            .and_then(|view| view.project_id())
        else {
            return;
        };
        app.set_view_style(&project, style);
    }

    /// A task was dragged to a new position, possibly in another section.
    fn on_task_moved(&self, id: &str, section: &str, index: u32) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let Some(project) = self
            .imp()
            .sidebar
            .borrow()
            .as_ref()
            .and_then(|sidebar| sidebar.selected_view())
            .and_then(|view| view.project_id())
        else {
            return;
        };

        let section = (!section.is_empty()).then(|| crate::model::SectionId::from_raw(section));
        app.move_task(&TaskId::from_raw(id), &project, section.as_ref(), index);
    }

    /// The bar of bulk actions, shown only in selection mode.
    fn build_action_bar(&self) -> (gtk::ActionBar, gtk::Label) {
        let bar = gtk::ActionBar::builder().revealed(false).build();

        let label = gtk::Label::new(Some("No tasks selected"));
        label.add_css_class("dimmed");
        bar.pack_start(&label);

        let complete = gtk::Button::builder()
            .icon_name("object-select-symbolic")
            .tooltip_text("Complete")
            .build();
        complete.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.bulk_complete()
        ));
        bar.pack_end(&complete);

        let schedule = gtk::MenuButton::builder()
            .icon_name("x-office-calendar-symbolic")
            .tooltip_text("Schedule")
            .build();
        schedule.set_popover(Some(&self.build_bulk_date_popover()));
        bar.pack_end(&schedule);

        let priority = gtk::MenuButton::builder()
            .icon_name("emblem-important-symbolic")
            .tooltip_text("Priority")
            .build();
        priority.set_popover(Some(&self.build_bulk_priority_popover()));
        bar.pack_end(&priority);

        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete")
            .css_classes(["destructive-action"])
            .build();
        delete.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.bulk_delete()
        ));
        bar.pack_end(&delete);

        (bar, label)
    }

    fn build_bulk_priority_popover(&self) -> gtk::Popover {
        let popover = gtk::Popover::new();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();

        for priority in crate::model::Priority::ALL {
            let button = gtk::Button::builder()
                .label(priority.label())
                .css_classes(["flat"])
                .build();
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                popover,
                move |_| {
                    let selected = window.selected_tasks();
                    if let Some(app) = window.planner_application() {
                        app.set_priority_all(&selected, priority);
                    }
                    popover.popdown();
                    window.report_bulk(selected.len(), "changed");
                }
            ));
            body.append(&button);
        }
        popover.set_child(Some(&body));
        popover
    }

    fn build_bulk_date_popover(&self) -> gtk::Popover {
        let popover = gtk::Popover::new();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();

        for (label, offset) in [
            ("Today", Some(0u64)),
            ("Tomorrow", Some(1)),
            ("Next week", Some(7)),
            ("No date", None),
        ] {
            let button = gtk::Button::builder()
                .label(label)
                .css_classes(["flat"])
                .build();
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                popover,
                move |_| {
                    let date = offset.and_then(|days| {
                        crate::ui::today().checked_add_days(chrono::Days::new(days))
                    });
                    // `None` here means two different things — "no date" and
                    // "that date does not exist" — but the second cannot
                    // happen for offsets of a week from today.
                    let selected = window.selected_tasks();
                    if let Some(app) = window.planner_application() {
                        app.set_due_all(&selected, date);
                    }
                    popover.popdown();
                    window.report_bulk(selected.len(), "rescheduled");
                }
            ));
            body.append(&button);
        }
        popover.set_child(Some(&body));
        popover
    }

    /// Turn selection mode on or off across whichever view is showing.
    fn set_selecting(&self, selecting: bool) {
        let imp = self.imp();
        if let Some(tasks) = imp.tasks.borrow().as_ref() {
            tasks.set_selecting(selecting);
        }
        if let Some(view) = imp.project_view.borrow().as_ref() {
            view.set_selecting(selecting);
        }
        if let Some(bar) = imp.action_bar.borrow().as_ref() {
            bar.set_revealed(selecting);
        }
        self.update_selection_label();
    }

    /// Every task selected, across every lane.
    fn selected_tasks(&self) -> Vec<TaskId> {
        let imp = self.imp();
        let mut selected = imp
            .tasks
            .borrow()
            .as_ref()
            .map(|tasks| tasks.selected())
            .unwrap_or_default();
        if let Some(view) = imp.project_view.borrow().as_ref() {
            selected.extend(view.selected());
        }
        selected
    }

    fn update_selection_label(&self) {
        let count = self.selected_tasks().len();
        if let Some(label) = self.imp().selection_label.borrow().as_ref() {
            label.set_label(&match count {
                0 => "No tasks selected".to_string(),
                1 => "1 task selected".to_string(),
                n => format!("{n} tasks selected"),
            });
        }
    }

    fn bulk_complete(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let selected = self.selected_tasks();
        if selected.is_empty() {
            return;
        }
        let completed = app.complete_all(&selected);

        let toast = adw::Toast::builder()
            .title(format!("Completed {}", count_of(completed.len())))
            .button_label("Undo")
            .build();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if let Some(app) = window.planner_application() {
                    app.uncomplete_all(&completed);
                }
            }
        ));
        self.add_toast(toast);
    }

    fn bulk_delete(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let selected = self.selected_tasks();
        if selected.is_empty() {
            return;
        }
        let count = selected.len();
        let removed = app.delete_all(&selected);

        let toast = adw::Toast::builder()
            .title(format!("Deleted {}", count_of(count)))
            .button_label("Undo")
            .build();
        let removed = std::cell::RefCell::new(Some(removed));
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                let Some(app) = window.planner_application() else {
                    return;
                };
                if let Some(tasks) = removed.borrow_mut().take() {
                    app.restore_tasks(tasks);
                }
            }
        ));
        self.add_toast(toast);
    }

    /// Say what a bulk action did. No undo: changing a priority back is one
    /// more bulk action, and an undo button that only sometimes appears is
    /// harder to rely on than one that never does.
    fn report_bulk(&self, count: usize, verb: &str) {
        if count > 0 {
            self.toast(&format!("{} {verb}", count_of(count)));
        }
    }

    fn add_toast(&self, toast: adw::Toast) {
        if let Some(toasts) = self.imp().toasts.borrow().as_ref() {
            toasts.add_toast(toast);
        }
    }

    /// Create or edit a saved filter.
    fn edit_filter(&self, existing: Option<crate::model::FilterId>) {
        use crate::model::query::Query;
        use crate::model::SavedFilter;

        let Some(app) = self.planner_application() else {
            return;
        };
        let current = existing
            .as_ref()
            .and_then(|id| app.with_store(|store| store.filter(id).cloned()));

        let dialog = adw::AlertDialog::new(
            Some(if current.is_some() {
                "Edit Filter"
            } else {
                "New Filter"
            }),
            None,
        );

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();

        let name = gtk::Entry::builder()
            .placeholder_text("Name")
            .text(
                current
                    .as_ref()
                    .map(|f| f.name.as_str())
                    .unwrap_or_default(),
            )
            .build();
        body.append(&name);

        let query = gtk::Entry::builder()
            .placeholder_text("p1 & due before: next week")
            .text(
                current
                    .as_ref()
                    .map(|f| f.query.as_str())
                    .unwrap_or_default(),
            )
            .build();
        body.append(&query);

        // A query that will not parse is reported as you type, with the
        // offending text quoted. Saving a broken filter and finding out later
        // that a sidebar entry is permanently empty is a much worse way to
        // learn the same thing.
        let feedback = gtk::Label::builder().xalign(0.0).wrap(true).build();
        feedback.add_css_class("caption");
        body.append(&feedback);

        let hint = gtk::Label::builder()
            .label(FILTER_HINT)
            .xalign(0.0)
            .wrap(true)
            .build();
        hint.add_css_class("caption");
        hint.add_css_class("dimmed");
        body.append(&hint);

        let validate = glib::clone!(
            #[weak]
            query,
            #[weak]
            feedback,
            #[weak]
            dialog,
            move || {
                let text = query.text().to_string();
                if text.trim().is_empty() {
                    feedback.set_label("");
                    feedback.remove_css_class("error");
                    dialog.set_response_enabled("save", false);
                    return;
                }
                match Query::parse(&text) {
                    Ok(_) => {
                        feedback.set_label("");
                        feedback.remove_css_class("error");
                        dialog.set_response_enabled("save", true);
                    }
                    Err(error) => {
                        feedback.set_label(&error.message);
                        feedback.add_css_class("error");
                        dialog.set_response_enabled("save", false);
                    }
                }
            }
        );
        query.connect_changed(glib::clone!(
            #[strong]
            validate,
            move |_| validate()
        ));

        dialog.set_extra_child(Some(&body));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("save", "Save");
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        if current.is_some() {
            dialog.add_response("delete", "Delete");
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        }
        validate();

        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                name,
                #[weak]
                query,
                move |_, response| {
                    let Some(app) = window.planner_application() else {
                        return;
                    };
                    match response {
                        "delete" => {
                            if let Some(id) = &existing {
                                app.remove_filter(id);
                            }
                        }
                        "save" => {
                            let name = name.text().trim().to_string();
                            let text = query.text().trim().to_string();
                            if name.is_empty() || Query::parse(&text).is_err() {
                                return;
                            }
                            let mut filter = match &current {
                                Some(filter) => filter.clone(),
                                None => SavedFilter::new(
                                    "",
                                    "",
                                    app.with_store(|store| store.next_filter_color()),
                                ),
                            };
                            filter.name = name;
                            filter.query = text;
                            app.put_filter(filter);
                        }
                        _ => {}
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    /// Open Quick Find.
    fn open_quick_find(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };

        let dialog = QuickFindDialog::new();
        dialog.connect_search(glib::clone!(
            #[weak(rename_to = window)]
            dialog,
            #[weak]
            app,
            move || app.with_store(|store| window.refresh(store))
        ));
        dialog.connect_chosen(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |hit| window.go_to(hit)
        ));
        dialog.present(Some(self));
    }

    /// Go wherever Quick Find was pointed.
    fn go_to(&self, hit: &crate::model::search::Hit) {
        use crate::model::search::Hit;

        match hit {
            Hit::Task { id, .. } => {
                // Open the task rather than navigating to whichever view might
                // contain it: a completed task is in none of them, and the
                // panel can show anything.
                self.open_task(id);
            }
            Hit::Project { id, .. } => self.select_view(&format!("project:{id}")),
            Hit::Label { name, .. } => {
                self.toast(&format!("Label “{name}” — filter by @{name} to see it"));
            }
        }
    }

    /// Select a sidebar view by ID, if it is there.
    fn select_view(&self, id: &str) {
        if let Some(sidebar) = self.imp().sidebar.borrow().as_ref() {
            sidebar.select(id);
        }
    }

    /// Ask for a name, and do something with it.
    ///
    /// One dialog for projects, sections and renames: they differ only in the
    /// words, and three near-identical copies is three places for the Enter
    /// key to behave slightly differently.
    fn prompt_for_name(
        &self,
        title: &str,
        placeholder: &str,
        initial: &str,
        verb: &str,
        accept: impl Fn(&Self, &str) + 'static,
    ) {
        let dialog = adw::AlertDialog::new(Some(title), None);
        let entry = gtk::Entry::builder()
            .placeholder_text(placeholder)
            .text(initial)
            .activates_default(true)
            .build();
        dialog.set_extra_child(Some(&entry));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("accept", verb);
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("accept"));
        dialog.set_close_response("cancel");

        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                entry,
                move |_, response| {
                    if response != "accept" {
                        return;
                    }
                    let name = entry.text().trim().to_string();
                    if !name.is_empty() {
                        accept(&window, &name);
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    /// The project currently on show, if the selected view is one.
    fn current_project(&self) -> Option<crate::model::ProjectId> {
        self.imp()
            .sidebar
            .borrow()
            .as_ref()
            .and_then(|sidebar| sidebar.selected_view())
            .and_then(|view| view.project_id())
    }

    fn prompt_for_section(&self) {
        let Some(project) = self.current_project() else {
            return;
        };
        self.prompt_for_name(
            "New Section",
            "Section name",
            "",
            "Add",
            move |window, name| {
                if let Some(app) = window.planner_application() {
                    app.add_section(&project, name);
                }
            },
        );
    }

    fn prompt_to_rename_section(&self, section: &SectionId) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let Some(current) =
            app.with_store(|store| store.section(section).map(|(_, s)| s.name.clone()))
        else {
            return;
        };
        let section = section.clone();
        self.prompt_for_name(
            "Rename Section",
            "Section name",
            &current,
            "Rename",
            move |window, name| {
                if let Some(app) = window.planner_application() {
                    app.rename_section(&section, name);
                }
            },
        );
    }

    /// Delete a section, with undo.
    ///
    /// A toast rather than the confirmation a project gets: the tasks stay in
    /// the project, so the only thing lost is the grouping, and putting it
    /// back is one click.
    fn delete_section(&self, section: &SectionId) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let Some(name) =
            app.with_store(|store| store.section(section).map(|(_, s)| s.name.clone()))
        else {
            return;
        };
        let Some(removed) = app.remove_section(section) else {
            return;
        };

        let toast = adw::Toast::builder()
            .title(format!("Deleted “{name}”"))
            .button_label("Undo")
            .build();
        let removed = std::cell::RefCell::new(Some(removed));
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                let Some(app) = window.planner_application() else {
                    return;
                };
                if let Some(removed) = removed.borrow_mut().take() {
                    app.restore_section(removed);
                }
            }
        ));
        self.add_toast(toast);
    }

    fn prompt_for_project(&self, parent: Option<crate::model::ProjectId>) {
        let title = if parent.is_some() {
            "New Subproject"
        } else {
            "New Project"
        };
        self.prompt_for_name(title, "Project name", "", "Add", move |window, name| {
            let Some(app) = window.planner_application() else {
                return;
            };
            let id = app.add_project(name, parent.as_ref());
            // Go to it. Making a project and being left looking at Today is
            // one click short of what "new project" means.
            window.select_view(&format!("project:{id}"));
        });
    }

    fn prompt_to_rename_project(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let Some(project) = self.current_project() else {
            return;
        };
        let current = app.with_store(|store| {
            store
                .project(&project)
                .map(|project| project.name.clone())
                .unwrap_or_default()
        });
        self.prompt_for_name(
            "Rename Project",
            "Project name",
            &current,
            "Rename",
            move |window, name| {
                if let Some(app) = window.planner_application() {
                    app.rename_project(&project, name);
                }
            },
        );
    }

    /// Delete the project on show, after asking.
    ///
    /// A confirmation rather than a toast with undo, unlike everywhere else:
    /// this takes the subprojects and every task in all of them, and the
    /// number is worth reading before it happens rather than after.
    fn confirm_delete_project(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let Some(project) = self.current_project() else {
            return;
        };
        let (name, tasks) = app.with_store(|store| {
            let name = store
                .project(&project)
                .map(|project| project.name.clone())
                .unwrap_or_default();
            let doomed = store.project_and_descendants(&project);
            let tasks = store
                .tasks()
                .iter()
                .filter(|task| doomed.contains(&task.project_id))
                .count();
            (name, tasks)
        });

        let dialog = adw::AlertDialog::new(
            Some(&format!("Delete “{name}”?")),
            Some(&match tasks {
                0 => "This project is empty.".to_string(),
                1 => "Its 1 task will be deleted too.".to_string(),
                n => format!("Its {n} tasks will be deleted too."),
            }),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, response| {
                    if response != "delete" {
                        return;
                    }
                    let Some(app) = window.planner_application() else {
                        return;
                    };
                    let Some(removed) = app.remove_project(&project) else {
                        return;
                    };
                    window.select_view("today");

                    let toast = adw::Toast::builder()
                        .title(format!("Deleted “{name}”"))
                        .button_label("Undo")
                        .build();
                    let removed = std::cell::RefCell::new(Some(removed));
                    toast.connect_button_clicked(glib::clone!(
                        #[weak]
                        window,
                        move |_| {
                            let Some(app) = window.planner_application() else {
                                return;
                            };
                            if let Some(removed) = removed.borrow_mut().take() {
                                app.restore_project(removed);
                            }
                        }
                    ));
                    window.add_toast(toast);
                }
            ),
        );
        dialog.present(Some(self));
    }

    /// Open quick add, aimed at whatever the sidebar is showing.
    fn open_quick_add(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };

        let default_project = self
            .imp()
            .sidebar
            .borrow()
            .as_ref()
            .and_then(|sidebar| sidebar.selected_view())
            .and_then(|view| view.project_id())
            .filter(|id| app.with_store(|store| store.project(id).is_some()))
            .unwrap_or_else(crate::model::ProjectId::inbox);

        let name = app.with_store(|store| {
            store
                .project(&default_project)
                .map(|project| project.name.clone())
                .unwrap_or_default()
        });
        let vocabulary = app.with_store(|store| store.vocabulary());

        let dialog = QuickAddDialog::new();
        dialog.prepare(vocabulary, crate::ui::today(), &name);
        dialog.connect_closure(
            "submitted",
            false,
            glib::closure_local!(
                #[watch(rename_to = window)]
                self,
                #[strong]
                default_project,
                move |_: QuickAddDialog, line: &str| {
                    window.on_quick_add_submitted(line, &default_project);
                }
            ),
        );
        dialog.present(Some(self));
    }

    fn on_quick_add_submitted(&self, line: &str, default_project: &crate::model::ProjectId) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let id = app.add_quick_add(line, default_project, None);

        let title = app.with_store(|store| {
            store
                .task(&id)
                .map(|task| task.content.clone())
                .unwrap_or_default()
        });

        // Undo rather than a confirmation: adding is cheap to reverse and
        // asking first for something this frequent would be intolerable.
        let toast = adw::Toast::builder()
            .title(format!("Added “{title}”"))
            .button_label("Undo")
            .build();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            id,
            move |_| {
                if let Some(app) = window.planner_application() {
                    app.delete_task(&id);
                }
            }
        ));
        if let Some(toasts) = self.imp().toasts.borrow().as_ref() {
            toasts.add_toast(toast);
        }
    }

    /// Show a task in the detail panel and slide the panel in.
    pub fn open_task(&self, id: &TaskId) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let imp = self.imp();
        let Some(detail) = imp.detail.borrow().clone() else {
            return;
        };

        app.with_store(|store| detail.show(id, store, crate::ui::today()));
        if let Some(split) = imp.detail_split.borrow().as_ref() {
            split.set_show_sidebar(true);
        }
    }

    /// Close the detail panel.
    fn close_detail(&self) {
        let imp = self.imp();
        if let Some(detail) = imp.detail.borrow().as_ref() {
            detail.clear();
        }
        if let Some(split) = imp.detail_split.borrow().as_ref() {
            split.set_show_sidebar(false);
        }
    }

    fn on_detail_edited(&self, edit: &Edit) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let Some(detail) = self.imp().detail.borrow().clone() else {
            return;
        };
        let Some(id) = detail.task_id() else {
            return;
        };

        match edit {
            Edit::Open(child) => {
                self.open_task(child);
                return;
            }
            Edit::Delete => {
                let title = app.with_store(|store| {
                    store
                        .task(&id)
                        .map(|task| task.content.clone())
                        .unwrap_or_default()
                });
                let removed = app.delete_task(&id);
                self.close_detail();

                // Undo rather than a confirmation dialog. Deleting a task is
                // cheap to reverse and common enough that a modal every time
                // would be worse than the mistake it prevents.
                let toast = adw::Toast::builder()
                    .title(format!("Deleted “{title}”"))
                    .button_label("Undo")
                    .build();
                let removed = std::cell::RefCell::new(Some(removed));
                toast.connect_button_clicked(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |_| {
                        let Some(app) = window.planner_application() else {
                            return;
                        };
                        if let Some(tasks) = removed.borrow_mut().take() {
                            app.restore_tasks(tasks);
                        }
                    }
                ));
                if let Some(toasts) = self.imp().toasts.borrow().as_ref() {
                    toasts.add_toast(toast);
                }
                return;
            }
            _ => {}
        }

        // `apply_edit` refreshes the window, and that reads the task back into
        // the panel — which matters because an edit can change more than it
        // was asked to: a date set on a repeating task carries the rule with
        // it, and completing a subtask changes the parent's count.
        app.apply_edit(&id, edit);
    }

    fn on_task_toggled(&self, id: &str, checked: bool) {
        let Some(app) = self.planner_application() else {
            return;
        };
        let id = TaskId::from_raw(id);

        if checked {
            let outcome = app.with_store(|store| store.task(&id).map(|task| task.due.clone()));
            app.complete_task(&id);
            // Say where a repeating task went. Ticking one off and watching it
            // stay in the list is otherwise indistinguishable from a bug.
            let rescheduled = app.with_store(|store| {
                store
                    .task(&id)
                    .filter(|task| !task.checked)
                    .and_then(|task| task.due.as_ref())
                    .map(|due| crate::ui::task_object::format_date(due.date, crate::ui::today()))
            });
            match rescheduled {
                Some(next) => self.toast(&format!("Repeats — next on {next}")),
                None => self.toast("Task completed"),
            }
            let _ = outcome;
        } else {
            app.uncomplete_task(&id);
        }
    }

    /// Show a message that does not need a response.
    pub fn toast(&self, message: &str) {
        if let Some(toasts) = self.imp().toasts.borrow().as_ref() {
            toasts.add_toast(adw::Toast::new(message));
        }
    }

    /// Show or clear the "not saving" banner.
    ///
    /// A banner rather than a toast, and it stays up until the save succeeds:
    /// this is an ongoing condition, and a toast about lost work is exactly
    /// the toast you miss because you are typing.
    pub fn set_save_error(&self, error: Option<String>) {
        let Some(banner) = self.imp().banner.borrow().clone() else {
            return;
        };
        match error {
            Some(message) => {
                banner.set_title(&message);
                banner.set_revealed(true);
            }
            None => banner.set_revealed(false),
        }
    }

    /// Tell the user if the file had to be recovered on the way in.
    fn report_load_outcome(&self) {
        let Some(app) = self.planner_application() else {
            return;
        };
        match app.take_load_outcome() {
            Some(LoadOutcome::Recovered { backup, .. }) => {
                let name = backup
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| backup.display().to_string());
                self.set_save_error(Some(format!(
                    "The planner file could not be read and was set aside as {name}. \
                     Starting with an empty list."
                )));
            }
            Some(LoadOutcome::ReadOnly { version }) => {
                self.set_save_error(Some(format!(
                    "This file was written by a newer version of Planner (v{version}). \
                     Changes will not be saved."
                )));
            }
            _ => {}
        }
    }
}

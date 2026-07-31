//! The navigation sidebar: the built-in views, then the project tree.
//!
//! A `GtkListBox` with `.navigation-sidebar` rather than `AdwSidebar` (1.9).
//! `AdwSidebar` is a flat list of items in sections, and projects nest —
//! there is nowhere in it to put a subproject or a disclosure triangle. When
//! the project tree is a tree, a list box with an indent is the honest fit.
//!
//! Every entry here is a [`Query`], including the built-in ones. Today is not
//! a special case with its own code path; it is `due: today | overdue`.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::model::query::Query;
use crate::model::store::Store;
use crate::model::Color;

/// One thing you can click in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// Stable identifier, used to remember what was selected across a refresh.
    pub id: String,
    pub title: String,
    pub icon: &'static str,
    /// The filter this view shows.
    pub query: String,
    /// The dot colour, for projects. Built-in views use their icon instead.
    pub color: Option<Color>,
    /// How far to indent, for subprojects.
    pub depth: u32,
    pub empty_title: &'static str,
    pub empty_description: &'static str,
}

impl View {
    /// The project this view *is*, if it is a project rather than a filter.
    ///
    /// What a new task typed in this view should default to. A filter view
    /// has no answer — "Today" is not somewhere a task can live — so those
    /// give `None` and the caller falls back to the Inbox.
    pub fn project_id(&self) -> Option<crate::model::ProjectId> {
        self.id
            .strip_prefix("project:")
            .map(crate::model::ProjectId::from_raw)
    }

    /// The saved filter this view is, if it is one.
    pub fn filter_id(&self) -> Option<crate::model::FilterId> {
        self.id
            .strip_prefix("filter:")
            .map(crate::model::FilterId::from_raw)
    }

    /// The parsed query.
    ///
    /// Built-in queries are compile-time constants and are expected to parse;
    /// a project's is built from its own name, escaped, so it cannot fail
    /// either. A failure here means a bug in this file, not bad user input.
    /// The parsed query, or an empty result if it will not parse.
    ///
    /// A built-in query is a compile-time constant and a project's is built
    /// from its own escaped name, so a failure in either is a bug here — hence
    /// the debug assertion. A *saved* filter is user-written and can be broken
    /// at any time, which is not a bug and must not be an assertion; it shows
    /// as an empty view, and the editor reports where it went wrong.
    pub fn query(&self) -> Query {
        Query::parse(&self.query).unwrap_or_else(|error| {
            debug_assert!(
                self.filter_id().is_some(),
                "built-in query {:?} is broken: {error}",
                self.query
            );
            Query::parse("no date & !no date").expect("a query matching nothing")
        })
    }
}

/// The views that always exist, in sidebar order.
pub fn builtin_views() -> Vec<View> {
    vec![
        View {
            id: "inbox".into(),
            title: "Inbox".into(),
            icon: "mail-unread-symbolic",
            query: "#Inbox".into(),
            color: None,
            depth: 0,
            empty_title: "Inbox zero",
            empty_description: "New tasks with no project land here.",
        },
        View {
            id: "today".into(),
            title: "Today".into(),
            icon: "view-continuous-symbolic",
            query: "due: today | overdue".into(),
            color: None,
            depth: 0,
            empty_title: "Nothing due today",
            empty_description: "Enjoy it.",
        },
        View {
            id: "upcoming".into(),
            title: "Upcoming".into(),
            icon: "x-office-calendar-symbolic",
            query: "due after: today".into(),
            color: None,
            depth: 0,
            empty_title: "Nothing scheduled",
            empty_description: "Tasks with a future date will appear here.",
        },
        View {
            id: "pinned".into(),
            title: "Pinned".into(),
            icon: "view-pin-symbolic",
            query: "pinned".into(),
            color: None,
            depth: 0,
            empty_title: "Nothing pinned",
            empty_description: "Pin a task to keep it in reach.",
        },
        View {
            id: "completed".into(),
            title: "Completed".into(),
            icon: "object-select-symbolic",
            query: "completed".into(),
            color: None,
            depth: 0,
            empty_title: "Nothing completed yet",
            empty_description: "Finished tasks are kept here.",
        },
    ]
}

/// The user's saved filters, in order.
pub fn filter_views(store: &Store) -> Vec<View> {
    store
        .filters_ordered()
        .into_iter()
        .map(|filter| View {
            id: format!("filter:{}", filter.id),
            title: filter.name.clone(),
            icon: "edit-find-symbolic",
            query: filter.query.clone(),
            color: Some(filter.color),
            depth: 0,
            empty_title: "Nothing matches",
            empty_description: "No tasks match this filter right now.",
        })
        .collect()
}

/// The project tree, flattened depth-first with each project's depth.
pub fn project_views(store: &Store) -> Vec<View> {
    fn walk(
        store: &Store,
        parent: Option<&crate::model::ProjectId>,
        depth: u32,
        out: &mut Vec<View>,
    ) {
        let projects: Vec<_> = store
            .projects_ordered()
            .into_iter()
            .filter(|project| project.parent_id.as_ref() == parent && !project.is_inbox())
            .map(|project| (project.id.clone(), project.name.clone(), project.color))
            .collect();

        for (id, name, color) in projects {
            out.push(View {
                id: format!("project:{id}"),
                title: name.clone(),
                icon: "folder-symbolic",
                query: format!("#{}", escape(&name)),
                color: Some(color),
                depth,
                empty_title: "No tasks yet",
                empty_description: "Add one with the button above.",
            });
            walk(store, Some(&id), depth + 1, out);
        }
    }

    let mut views = Vec::new();
    walk(store, None, 0, &mut views);
    views
}

/// Escape a name so it survives the query tokenizer.
///
/// A project genuinely called "R&D" has to become `#R\&D`, or the `&` reads as
/// a conjunction and the view silently shows nothing.
fn escape(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for character in name.chars() {
        if matches!(character, '&' | '|' | '!' | ',' | '(' | ')' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::cell::RefCell;
    use std::sync::OnceLock;

    #[derive(Default)]
    pub struct Sidebar {
        pub list: RefCell<Option<gtk::ListBox>>,
        pub views: RefCell<Vec<View>>,
        /// What each list-box row stands for: an index into `views`, or
        /// `None` for a heading. Built as the rows are, so any arrangement of
        /// headings works without anyone counting offsets.
        pub rows: RefCell<Vec<Option<usize>>>,
        pub selected: RefCell<String>,
        /// Set while rows are rebuilt, so re-selecting the current row does
        /// not report a navigation the user did not make.
        pub rebuilding: std::cell::Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "PlannerSidebar";
        type Type = super::Sidebar;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for Sidebar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![Signal::builder("view-selected")
                    .param_types([str::static_type()])
                    .build()]
            })
        }
    }

    impl WidgetImpl for Sidebar {}
    impl BoxImpl for Sidebar {}
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build()
    }

    fn build(&self) {
        let list = gtk::ListBox::new();
        list.add_css_class("navigation-sidebar");
        list.set_selection_mode(gtk::SelectionMode::Single);

        list.connect_row_selected(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |_, row| {
                if sidebar.imp().rebuilding.get() {
                    return;
                }
                let Some(row) = row else { return };
                let index = row.index();
                if index < 0 {
                    return;
                }
                let id = sidebar
                    .imp()
                    .rows
                    .borrow()
                    .get(index as usize)
                    .copied()
                    .flatten()
                    .and_then(|position| {
                        sidebar
                            .imp()
                            .views
                            .borrow()
                            .get(position)
                            .map(|view| view.id.clone())
                    });
                if let Some(id) = id {
                    sidebar.imp().selected.replace(id.clone());
                    sidebar.emit_by_name::<()>("view-selected", &[&id]);
                }
            }
        ));

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        self.append(&scroller);

        self.imp().list.replace(Some(list));
    }

    /// Rebuild the sidebar from the store, keeping the selection if it still
    /// exists and falling back to Today if it does not.
    pub fn refresh(&self, store: &Store, today: chrono::NaiveDate) {
        let imp = self.imp();
        let list = imp.list.borrow().clone().expect("built");

        let mut views = builtin_views();
        views.extend(filter_views(store));
        views.extend(project_views(store));

        let wanted = imp.selected.borrow().clone();
        let selected = views
            .iter()
            .position(|view| view.id == wanted)
            .or_else(|| views.iter().position(|view| view.id == "today"))
            .unwrap_or(0);

        imp.rebuilding.set(true);
        while let Some(row) = list.first_child() {
            list.remove(&row);
        }

        // Headings go before the first view of each group. They are rows as
        // far as the list box is concerned, so `rows` records which ones stand
        // for a view and which do not.
        let built_in = builtin_views().len();
        let filters = filter_views(store).len();
        let mut row_map: Vec<Option<usize>> = Vec::new();
        let mut selected_row = 0;

        for (index, view) in views.iter().enumerate() {
            if index == built_in && filters > 0 {
                list.append(&build_heading("Filters"));
                row_map.push(None);
            }
            if index == built_in + filters {
                list.append(&build_heading("Projects"));
                row_map.push(None);
            }
            if index == selected {
                selected_row = row_map.len();
            }
            list.append(&build_row(view, store, today));
            row_map.push(Some(index));
        }

        if let Some(row) = list.row_at_index(selected_row as i32) {
            list.select_row(Some(&row));
        }
        imp.rows.replace(row_map);
        imp.rebuilding.set(false);

        if let Some(view) = views.get(selected) {
            imp.selected.replace(view.id.clone());
        }
        imp.views.replace(views);
    }

    /// Select a view by ID, if it is still there.
    pub fn select(&self, id: &str) {
        let imp = self.imp();
        let position = imp.views.borrow().iter().position(|view| view.id == id);
        let Some(position) = position else {
            return;
        };
        let Some(list) = imp.list.borrow().clone() else {
            return;
        };
        let row_index = imp
            .rows
            .borrow()
            .iter()
            .position(|entry| *entry == Some(position));
        if let Some(row) = row_index.and_then(|index| list.row_at_index(index as i32)) {
            list.select_row(Some(&row));
        }
    }

    /// The view currently selected.
    pub fn selected_view(&self) -> Option<View> {
        let id = self.imp().selected.borrow().clone();
        self.imp()
            .views
            .borrow()
            .iter()
            .find(|view| view.id == id)
            .cloned()
    }
}

/// A non-selectable heading between the built-in views and the projects.
fn build_heading(text: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::builder().label(text).xalign(0.0).build();
    label.add_css_class("sidebar-heading");
    label.add_css_class("dimmed");

    gtk::ListBoxRow::builder()
        .child(&label)
        .selectable(false)
        .activatable(false)
        .build()
}

/// One sidebar row: icon or colour dot, title, and a count of open tasks.
fn build_row(view: &View, store: &Store, today: chrono::NaiveDate) -> gtk::ListBoxRow {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_start(6 + view.depth as i32 * 16)
        .build();

    match view.color {
        Some(color) => {
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.set_valign(gtk::Align::Center);
            dot.add_css_class("accent-dot");
            dot.add_css_class(&color.css_class());
            body.append(&dot);
        }
        None => {
            let icon = gtk::Image::from_icon_name(view.icon);
            body.append(&icon);
        }
    }

    let title = gtk::Label::builder()
        .label(&view.title)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    body.append(&title);

    // Completed is a count of things already done; showing it beside the open
    // counts would read as work outstanding.
    if view.id != "completed" {
        let count = store.query(&view.query(), today).len();
        if count > 0 {
            let badge = gtk::Label::builder().label(count.to_string()).build();
            badge.add_css_class("count-badge");
            badge.add_css_class("dimmed");
            badge.add_css_class("numeric");
            body.append(&badge);
        }
    }

    gtk::ListBoxRow::builder().child(&body).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_query_parses() {
        for view in builtin_views() {
            Query::parse(&view.query).unwrap_or_else(|error| panic!("{}: {error}", view.title));
        }
    }

    #[test]
    fn only_a_project_view_names_a_project() {
        for view in builtin_views() {
            assert_eq!(view.project_id(), None, "{} is a filter", view.title);
        }

        let view = View {
            id: "project:abc".into(),
            ..builtin_views().remove(0)
        };
        assert_eq!(
            view.project_id(),
            Some(crate::model::ProjectId::from_raw("abc"))
        );
    }

    #[test]
    fn a_project_name_with_an_operator_in_it_is_escaped() {
        assert_eq!(escape("R&D"), r"R\&D");
        assert_eq!(escape("Either|Or"), r"Either\|Or");
        assert_eq!(escape("Plain"), "Plain");
        // And the escaped form parses back to the original name.
        let query = Query::parse(&format!("#{}", escape("R&D"))).expect("parses");
        assert_eq!(
            query.lists,
            vec![crate::model::query::Filter::Term(
                crate::model::query::Term::Project {
                    name: "R&D".into(),
                    include_subprojects: false
                }
            )]
        );
    }
}

//! Quick Find: type a few letters, go to the thing.
//!
//! Search over tasks, projects and labels at once, because when you know what
//! you are looking for you do not first want to decide what kind of thing it
//! is. The ranking lives in [`crate::model::search`] and is tested there; this
//! is the dialog around it.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::model::search::{search, Hit};
use crate::model::store::Store;

/// How many results to offer. More than fits on screen is a list you scroll
/// rather than read, which is not what this is for.
const LIMIT: usize = 12;

mod imp {
    use super::*;
    use std::cell::RefCell;

    type Callback = Box<dyn Fn(&Hit)>;

    #[derive(Default)]
    pub struct QuickFindDialog {
        pub entry: RefCell<Option<gtk::SearchEntry>>,
        pub results: RefCell<Option<gtk::ListBox>>,
        pub empty: RefCell<Option<gtk::Label>>,
        pub hits: RefCell<Vec<Hit>>,
        pub chosen: RefCell<Option<Callback>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for QuickFindDialog {
        const NAME: &'static str = "PlannerQuickFindDialog";
        type Type = super::QuickFindDialog;
        type ParentType = adw::Dialog;
    }

    impl ObjectImpl for QuickFindDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for QuickFindDialog {}
    impl AdwDialogImpl for QuickFindDialog {}
}

glib::wrapper! {
    pub struct QuickFindDialog(ObjectSubclass<imp::QuickFindDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for QuickFindDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl QuickFindDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Called with whatever the user picked.
    pub fn connect_chosen(&self, callback: impl Fn(&Hit) + 'static) {
        self.imp().chosen.replace(Some(Box::new(callback)));
    }

    fn build(&self) {
        let imp = self.imp();
        self.set_title("Quick Find");
        self.set_content_width(460);
        self.set_content_height(420);

        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Search tasks, projects and labels")
            .build();
        entry.connect_activate(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.choose(0)
        ));

        let results = gtk::ListBox::new();
        results.add_css_class("navigation-sidebar");
        results.set_selection_mode(gtk::SelectionMode::Single);
        results.connect_row_activated(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_, row| {
                let index = row.index();
                if index >= 0 {
                    dialog.choose(index as usize);
                }
            }
        ));

        // Down from the entry moves into the results, so the whole thing works
        // without the mouse.
        entry.connect_next_match(glib::clone!(
            #[weak]
            results,
            move |_| {
                if let Some(row) = results.row_at_index(0) {
                    row.grab_focus();
                }
            }
        ));

        let empty = gtk::Label::builder()
            .label("Type to search")
            .vexpand(true)
            .build();
        empty.add_css_class("dimmed");

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&results)
            .build();

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        body.append(&entry);
        body.append(&empty);
        body.append(&scroller);

        let toolbar = adw::ToolbarView::builder().content(&body).build();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        self.set_child(Some(&toolbar));
        self.set_focus(Some(&entry));

        imp.entry.replace(Some(entry));
        imp.results.replace(Some(results));
        imp.empty.replace(Some(empty));

        self.show_results(&[]);
    }

    /// Search the store for whatever is typed, and show it.
    ///
    /// Driven by the window rather than held here, so the dialog never keeps
    /// a store reference across the time it is open.
    pub fn refresh(&self, store: &Store) {
        let query = self
            .imp()
            .entry
            .borrow()
            .as_ref()
            .map(|entry| entry.text().to_string())
            .unwrap_or_default();
        let hits = search(store, &query, LIMIT);
        self.show_results(&hits);
    }

    /// Re-search whenever the text changes.
    pub fn connect_search(&self, search: impl Fn() + 'static) {
        if let Some(entry) = self.imp().entry.borrow().as_ref() {
            entry.connect_search_changed(move |_| search());
        }
    }

    fn show_results(&self, hits: &[Hit]) {
        let imp = self.imp();
        let Some(list) = imp.results.borrow().clone() else {
            return;
        };
        while let Some(row) = list.first_child() {
            list.remove(&row);
        }

        for hit in hits {
            let row = adw::ActionRow::builder()
                .title(hit.title())
                .subtitle(hit.context())
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name(hit.icon()));
            list.append(&row);
        }

        if let Some(row) = list.row_at_index(0) {
            list.select_row(Some(&row));
        }
        if let Some(empty) = imp.empty.borrow().as_ref() {
            let typed = imp
                .entry
                .borrow()
                .as_ref()
                .is_some_and(|entry| !entry.text().trim().is_empty());
            empty.set_visible(hits.is_empty());
            empty.set_label(if typed {
                "No matches"
            } else {
                "Type to search"
            });
        }
        list.set_visible(!hits.is_empty());

        imp.hits.replace(hits.to_vec());
    }

    fn choose(&self, index: usize) {
        let hit = self.imp().hits.borrow().get(index).cloned();
        let Some(hit) = hit else {
            return;
        };
        if let Some(callback) = self.imp().chosen.borrow().as_ref() {
            callback(&hit);
        }
        self.close();
    }

    /// Set the query, for tests.
    pub fn set_query(&self, text: &str) {
        if let Some(entry) = self.imp().entry.borrow().as_ref() {
            entry.set_text(text);
        }
    }

    /// The results currently on offer, for tests.
    pub fn hits(&self) -> Vec<Hit> {
        self.imp().hits.borrow().clone()
    }
}

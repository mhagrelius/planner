//! The GTK 4/libadwaita half.
//!
//! Everything here knows about widgets; nothing here knows the rules. The
//! division of labour is the one Stickies uses and it has earned it:
//! [`PlannerApplication`] owns the [`Store`](crate::model::Store) and is the
//! only thing that mutates it, widgets emit signals saying what the user did,
//! and state is pushed back down by method call. One place to lose data means
//! one place to get it right.

pub mod application;
pub mod date_picker;
pub mod detail_panel;
pub mod project_view;
pub mod quick_add;
pub mod quick_find;
pub mod sidebar;
pub mod task_list;
pub mod task_object;
pub mod task_row;
pub mod window;

pub use application::PlannerApplication;
pub use quick_add::QuickAddDialog;
pub use task_object::TaskObject;
pub use window::PlannerWindow;

/// The stylesheet, compiled into the binary.
pub const STYLE: &str = include_str!("style.css");

/// The few rules whose answer changes on a dark background.
pub const STYLE_DARK: &str = include_str!("style-dark.css");

/// Install the stylesheet on a display, once.
pub fn load_stylesheet(display: &gtk::gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // CSS has `@media (prefers-color-scheme: dark)`, but that asks the *system*
    // preference, which is not the question. A user who has forced dark in this
    // app — or a preview run under `ForceDark` — is on a dark background while
    // the system still says light, and the query answers light. libadwaita has
    // the real answer on `AdwStyleManager` and ships its own light and dark
    // sheets as separate providers; this follows it with the handful of rules
    // that need to know.
    let dark_provider = gtk::CssProvider::new();
    dark_provider.load_from_string(STYLE_DARK);

    let manager = adw::StyleManager::default();
    let apply = {
        let display = display.clone();
        move |manager: &adw::StyleManager| {
            // Removing a provider that is not installed is a no-op, so this
            // needs no record of which way round it currently is.
            gtk::style_context_remove_provider_for_display(&display, &dark_provider);
            if manager.is_dark() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &dark_provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
        }
    };

    apply(&manager);
    manager.connect_dark_notify(apply);
}

/// Today, in the local timezone.
///
/// The one place the UI half reads the clock. Everything below it takes the
/// date as an argument, which is what makes the model testable in February.
pub fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

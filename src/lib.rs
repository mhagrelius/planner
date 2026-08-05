//! A task planner for the GNOME desktop.
//!
//! The crate splits into two halves. Everything under [`model`] is plain Rust:
//! no GTK, no display, no main loop, and therefore unit-testable anywhere. The
//! `ui` half wraps that in GTK 4/libadwaita widgets and is the only place that
//! knows a window exists.
//!
//! The split is not decoration. Almost every bug worth having in a planner
//! lives in date arithmetic, recurrence rules, filter evaluation and
//! natural-language parsing — all of which are pure functions over plain data,
//! and none of which should need a Wayland socket to test.

/// The half that does not draw, which lives in its own crate so that
/// `planner-server` can link it without libadwaita.
///
/// Re-exported under the name it had when it was a directory, so that
/// `crate::model::Task` still means what it always did.
pub use planner_core as model;

pub mod ui;

/// The application ID, used for D-Bus, the desktop file, and GSettings.
pub const APP_ID: &str = "us.hagreli.Planner";

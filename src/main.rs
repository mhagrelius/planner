//! Entry point. Everything worth reading is in [`planner::ui::PlannerApplication`].

use gtk::prelude::*;
use planner::ui::PlannerApplication;

fn main() -> gtk::glib::ExitCode {
    PlannerApplication::new().run()
}

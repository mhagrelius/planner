//! The planner's data and every rule that operates on it.
//!
//! Nothing in here links against GTK, so `cargo test` exercises it with no X
//! server, no Wayland socket and no `gtk::init()`. Nothing in here reads the
//! clock either: any function whose answer depends on today's date takes it as
//! an argument, which is the difference between a test and a bug report filed
//! next February.
//!
//! It is a crate of its own rather than a module because `planner-server` needs
//! these types — the two halves must agree on what a task is, and a second
//! definition of that is a bug waiting for someone to change one of them — and
//! a container has no business linking libadwaita to get them. The GTK shell
//! re-exports the whole thing as `planner::model`, so call sites there read as
//! they did when this was a directory.

pub mod agent;
pub mod color;
pub mod due;
pub mod id;
pub mod order;
pub mod parse;
pub mod priority;
pub mod project;
pub mod query;
pub mod recurrence;
pub mod schedule;
pub mod search;
pub mod store;
pub mod sync;
pub mod task;
pub mod tombstone;

pub use color::Color;
pub use due::Due;
pub use id::{FilterId, LabelId, ProjectId, ReminderId, SectionId, TaskId};
pub use order::Order;
pub use priority::Priority;
pub use project::{Label, Project, SavedFilter, Section, SortBy, ViewStyle};
pub use query::Query;
pub use recurrence::Recurrence;
pub use schedule::Schedule;
pub use search::{search, Hit};
pub use store::{LoadOutcome, SaveError, Store};
pub use task::{Completion, Reminder, Task, Trigger};
pub use tombstone::{RecordKind, Tombstone};

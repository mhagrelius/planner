//! Turning what a person typed into structured data.
//!
//! Quick-add and the filter query language both live downstream of this. All
//! of it is pure functions over `&str` with `today` passed in, which is what
//! makes a bug report of the form "typing X on a Tuesday in February gives Y"
//! reproducible as a test rather than as a wait until February.

pub mod date;
pub mod quick_add;
pub mod recurrence;

pub use date::{parse_date, parse_time};
pub use quick_add::{parse_quick_add, QuickAdd, Vocabulary};
pub use recurrence::parse_recurrence;

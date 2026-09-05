//! Identifiers.
//!
//! Every record type gets its own ID type rather than sharing a `String`.
//! A task carries a project ID, a section ID and a parent *task* ID at the same
//! time; making those three interchangeable invites exactly the kind of bug
//! that type-checks, runs, and silently files a task under the wrong thing.
//! The newtypes cost nothing at runtime and the compiler catches the mix-up.
//!
//! IDs are opaque strings, generated locally and never reused. They are stable
//! for the life of a record, which is what a sync source would need later.

use std::time::{SystemTime, UNIX_EPOCH};

/// Generate an ID unique within this store.
///
/// Records are only ever created by the single running instance
/// (`GtkApplication` enforces uniqueness), so a monotonic counter combined with
/// the creation timestamp is sufficient — no need to pull in a UUID
/// dependency. The timestamp prefix also makes IDs sort roughly by age, which
/// is occasionally handy when reading the JSON by hand.
fn generate() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{seq:x}")
}

/// Define an ID newtype: a `String` that only compares against its own kind.
macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord,
            serde::Serialize, serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Mint a fresh, unused ID.
            pub fn new() -> Self {
                Self(generate())
            }

            /// Wrap an existing string, for reserved IDs and for tests.
            pub fn from_raw(raw: impl Into<String>) -> Self {
                Self(raw.into())
            }

            /// The underlying string, for display and serialisation.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// Identifies a [`Task`](crate::Task).
    TaskId
);
id_type!(
    /// Identifies a [`Project`](crate::Project).
    ProjectId
);
id_type!(
    /// Identifies a [`Section`](crate::Section) within a project.
    SectionId
);
id_type!(
    /// Identifies a [`Label`](crate::Label).
    LabelId
);
id_type!(
    /// Identifies a [`Reminder`](crate::Reminder) on a task.
    ReminderId
);
id_type!(
    /// Identifies a [`SavedFilter`](crate::SavedFilter).
    FilterId
);

impl ProjectId {
    /// The Inbox, where a task with no stated project lands.
    ///
    /// Reserved rather than generated, so the Inbox survives an export and
    /// re-import, and so a task can name it before the store has been read.
    pub fn inbox() -> Self {
        Self::from_raw("inbox")
    }

    /// Whether this is the reserved Inbox ID.
    pub fn is_inbox(&self) -> bool {
        self.0 == "inbox"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generated_ids_are_unique() {
        let ids: HashSet<_> = (0..10_000).map(|_| TaskId::new()).collect();
        assert_eq!(ids.len(), 10_000);
    }

    #[test]
    fn an_id_round_trips_through_json_as_a_bare_string() {
        let id = TaskId::from_raw("abc");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"abc\"");
        assert_eq!(serde_json::from_str::<TaskId>(&json).unwrap(), id);
    }

    #[test]
    fn the_inbox_id_is_stable_across_runs() {
        assert_eq!(ProjectId::inbox(), ProjectId::inbox());
        assert!(ProjectId::inbox().is_inbox());
        assert!(!ProjectId::new().is_inbox());
    }
}

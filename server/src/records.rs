//! The records, in Postgres.
//!
//! The server stores and arbitrates. It does not evaluate a filter query,
//! compute a recurrence, parse a quick-add line or decide what is due today —
//! all of that is the client's, where it already is and where it is already
//! tested. A server that starts answering "what is in Today" is a second
//! planner that can disagree with the first.
//!
//! So there is one table and three questions: what have you got, take this,
//! and this one is gone.
//!
//! **A write whose version is not newer than the stored one is refused.** That
//! single rule is why this is a database rather than a directory of JSON
//! files: the refusal has to be atomic with the write, or two machines
//! syncing at once read the same version, both decide theirs is newer, and
//! both write.

use chrono::{DateTime, Utc};
use planner_core::tombstone::RecordKind;
use serde::{Deserialize, Serialize};

/// What the server holds for one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: RecordKind,
    pub id: String,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
    /// Absent in a snapshot listing, present when a record is fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// The name a kind goes by in the database.
///
/// Spelled out rather than derived from the serde representation, so that
/// renaming a variant for the wire cannot silently orphan every row already
/// written under the old name.
pub fn kind_name(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Task => "task",
        RecordKind::Project => "project",
        RecordKind::Section => "section",
        RecordKind::Label => "label",
        RecordKind::Filter => "filter",
    }
}

/// The kind a database row belongs to.
pub fn kind_from_name(name: &str) -> Option<RecordKind> {
    match name {
        "task" => Some(RecordKind::Task),
        "project" => Some(RecordKind::Project),
        "section" => Some(RecordKind::Section),
        "label" => Some(RecordKind::Label),
        "filter" => Some(RecordKind::Filter),
        _ => None,
    }
}

/// Store this record if it is newer than what is there.
///
/// The `WHERE` on the conflict clause is the whole point: Postgres evaluates
/// it while holding the row, so two clients arriving together are serialised
/// and the older one is told `Stale` rather than overwriting the newer.
pub const UPSERT: &str = "
    INSERT INTO records (kind, id, updated_at, deleted_at, body)
    VALUES ($1, $2, $3, NULL, $4)
    ON CONFLICT (kind, id) DO UPDATE
        SET updated_at = excluded.updated_at,
            deleted_at = NULL,
            body       = excluded.body
        WHERE records.updated_at < excluded.updated_at
    RETURNING 1
";

/// Mark a record deleted, if the deletion is newer than what is there.
///
/// The body goes: keeping the contents of a deleted task on a NAS is not
/// something anybody asked for, and the row itself is all a pass needs — a
/// machine that has been switched off has to tell "deleted" from "never
/// seen", and only a row can say the first.
pub const DELETE: &str = "
    INSERT INTO records (kind, id, updated_at, deleted_at, body)
    VALUES ($1, $2, $3, $3, NULL)
    ON CONFLICT (kind, id) DO UPDATE
        SET updated_at = excluded.updated_at,
            deleted_at = excluded.deleted_at,
            body       = NULL
        WHERE records.updated_at < excluded.updated_at
    RETURNING 1
";

/// Everything the server holds, without the bodies.
///
/// A snapshot is what a pass compares against, and it needs the versions
/// rather than the contents — sending every task's text so the client can
/// discover that nothing changed is the one thing a sync must not do.
pub const SNAPSHOT: &str = "
    SELECT kind, id, updated_at, deleted_at
    FROM records
    ORDER BY kind, id
";

/// The bodies of specific records, for the pull half of a pass.
///
/// One kind at a time — five queries at the very worst — because a composite
/// `(kind, id) = ANY(…)` needs an array of anonymous records that the client
/// library has no clean way to build. Grouping by kind costs a loop and reads
/// like the index it uses.
pub const FETCH: &str = "
    SELECT kind, id, updated_at, deleted_at, body
    FROM records
    WHERE kind = $1 AND id = ANY($2)
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_name_and_survives_the_round_trip() {
        // A kind added to the core without a name here would be stored under a
        // name the reader does not recognise, and its records would go quiet.
        for kind in [
            RecordKind::Task,
            RecordKind::Project,
            RecordKind::Section,
            RecordKind::Label,
            RecordKind::Filter,
        ] {
            assert_eq!(kind_from_name(kind_name(kind)), Some(kind), "{kind:?}");
        }
    }

    #[test]
    fn an_unknown_kind_is_none_rather_than_a_guess() {
        assert_eq!(kind_from_name("sprocket"), None);
    }

    #[test]
    fn both_writes_refuse_to_go_backwards() {
        // The rule the whole server exists for. Asserted on the SQL because
        // the alternative is noticing it went missing on a NAS.
        for statement in [UPSERT, DELETE] {
            assert!(
                statement.contains("WHERE records.updated_at < excluded.updated_at"),
                "a write without the version check would let an older copy win"
            );
        }
    }

    #[test]
    fn a_snapshot_does_not_select_the_bodies() {
        assert!(
            !SNAPSHOT.contains("body"),
            "a snapshot that carries every task's text is not a snapshot"
        );
    }

    #[test]
    fn an_entry_omits_what_it_has_not_got() {
        let entry = Entry {
            kind: RecordKind::Task,
            id: "t1".into(),
            updated_at: DateTime::from_timestamp(0, 0).unwrap(),
            deleted_at: None,
            body: None,
        };
        let json = serde_json::to_string(&entry).expect("serialise");
        assert!(!json.contains("deleted_at"), "{json}");
        assert!(!json.contains("body"), "{json}");
    }
}

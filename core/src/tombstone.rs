//! A record that used to exist.
//!
//! Deleting locally is easy: take the record out and the app stops showing it.
//! Deleting across machines is not, because "I do not have this record" and "I
//! deleted this record" look identical from the outside. Without a note saying
//! which, a task deleted here is a task the other machine still has and will
//! helpfully send back on the next pass, for ever.
//!
//! So a deletion leaves a marker. It carries the id and the moment, and
//! nothing else — the record's *contents* are the undo toast's problem, and
//! [`RemovedProject`](crate::store::RemovedProject) and its neighbours already
//! carry those. Keeping the two separate is what lets every read path in the
//! store stay exactly as it was: a deleted record is genuinely gone from the
//! list it lived in, so nothing has to remember to filter it out.
//!
//! **Putting a record back clears its marker.** An undo that left the marker
//! behind would delete the record again on the next sync, which is the worst
//! version of this: the app says the undo worked, and it does, until a pass
//! runs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which list the deleted record came from.
///
/// Ids are unique across kinds in practice, but sync applies deletions per
/// kind and an untyped id would make that a search of five lists hoping for
/// exactly one hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecordKind {
    Task,
    Project,
    Section,
    Label,
    Filter,
}

/// A record that was deleted, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub kind: RecordKind,
    pub id: String,
    pub deleted_at: DateTime<Utc>,
}

/// How long a marker is kept.
///
/// It only has to outlive the longest a machine can be away and still be
/// syncing rather than starting again. Ninety days is a laptop left in a
/// drawer over a season; a machine gone longer than that gets the record back,
/// which is the recoverable mistake of the two.
pub const RETENTION_DAYS: i64 = 90;

/// Drop markers old enough that no machine could still need them.
///
/// Without this the file grows for ever with a line per task anyone has ever
/// ticked off and deleted.
pub fn purge_expired(tombstones: &mut Vec<Tombstone>, now: DateTime<Utc>) {
    let cutoff = now - chrono::Duration::days(RETENTION_DAYS);
    tombstones.retain(|tombstone| tombstone.deleted_at > cutoff);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(days_ago: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap() - chrono::Duration::days(days_ago)
    }

    fn marker(id: &str, days_ago: i64) -> Tombstone {
        Tombstone {
            kind: RecordKind::Task,
            id: id.to_string(),
            deleted_at: at(days_ago),
        }
    }

    #[test]
    fn a_marker_past_its_retention_is_dropped() {
        let mut tombstones = vec![marker("old", RETENTION_DAYS + 1), marker("recent", 1)];
        purge_expired(&mut tombstones, at(0));

        let kept: Vec<&str> = tombstones.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(kept, vec!["recent"]);
    }

    #[test]
    fn a_marker_on_the_boundary_is_kept() {
        // The recoverable mistake is keeping one too long, not one too few.
        let mut tombstones = vec![marker("borderline", RETENTION_DAYS - 1)];
        purge_expired(&mut tombstones, at(0));
        assert_eq!(tombstones.len(), 1);
    }

    #[test]
    fn a_marker_round_trips_through_json() {
        let marker = marker("t1", 0);
        let json = serde_json::to_string(&marker).expect("serialise");
        assert!(json.contains(r#""kind":"task""#), "{json}");
        assert_eq!(
            serde_json::from_str::<Tombstone>(&json).expect("deserialise"),
            marker
        );
    }
}

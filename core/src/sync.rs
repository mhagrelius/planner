//! Deciding what to do when a task list exists in more than one place.
//!
//! A pure function. Take what this machine holds now, what the server holds
//! now, and what the two agreed on last time, and return work. A task edited
//! in Planner, a project deleted on the Mac, and a machine that was switched
//! off for a week are the same input.
//!
//! # Three snapshots, not two
//!
//! Comparing local against remote can tell you they differ but never which of
//! them moved, and "differs" is not enough to act on — pushing when you should
//! have pulled is how a sync loses work. So a third snapshot is kept: what the
//! two sides agreed on at the end of the last pass. Local against base says
//! whether *this* machine changed something; remote against base says whether
//! another one did. Only when both did is there anything to resolve.
//!
//! # Per record, and small
//!
//! Planner is one JSON document, but syncing it whole would mean two machines
//! both open is last-writer-wins over everything, silently. So the unit is the
//! record: a task, a project, a section, a label, a saved filter. Each has a
//! stable id and an `updated_at`, and each moves on its own.
//!
//! A version here is a timestamp, not a hash of the contents. Records are
//! small and every mutation goes through `touch`, so the timestamp already
//! answers "did this change" — and unlike a hash it also answers "which one
//! came second", which is the question the resolution rules ask.
//!
//! # The two rules that are rules rather than questions
//!
//! **A deletion never wins over an edit.** If one machine deleted a task and
//! another edited it, the task survives. Losing work is the one unrecoverable
//! failure here, and the recoverable mistake is always the one to make: a task
//! that comes back is deleted again in a second, a task that is gone is gone.
//!
//! **Everything else is last-writer-wins, per record.** This is where Planner
//! parts company with a notebook. Brain writes a conflicting note into the
//! vault beside the original because a note is prose somebody wrote and losing
//! a paragraph is unrecoverable. A task record is a dozen scalar fields, and
//! the worst case here is a priority set twice in the same minute on two
//! machines — a re-do, not a loss. Conflict copies would cost a duplicate task
//! in the list for every ordinary double edit, which is worse than the thing
//! they prevent.
//!
//! The honest cost, so it is not a surprise: two machines editing the same
//! task's `description` in one pass keeps one of them, and adding a different
//! label to the same task on each keeps one set rather than the union. If
//! either turns out to bite, field-level merge is the next step — measured
//! first.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::store::Store;
use crate::tombstone::RecordKind;

/// Which record a snapshot entry is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub kind: RecordKind,
    pub id: String,
}

impl Key {
    pub fn new(kind: RecordKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

/// What one side holds for one record.
///
/// The timestamp *is* the version, so two sides carrying the same one are
/// taken to be holding the same record. Two machines editing one task within
/// the same microsecond would be missed by that, and the answer is a content
/// hash — which is a second thing to keep in step with the record for a case
/// that needs two clocks to agree to the microsecond. Not yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// The record is there, last changed at this moment.
    Live(DateTime<Utc>),
    /// The record was deleted at this moment.
    Deleted(DateTime<Utc>),
}

impl Version {
    fn at(&self) -> DateTime<Utc> {
        match self {
            Self::Live(at) | Self::Deleted(at) => *at,
        }
    }

    fn is_deleted(&self) -> bool {
        matches!(self, Self::Deleted(_))
    }
}

/// What one side holds: every record it knows about, by key.
pub type Snapshot = BTreeMap<Key, Version>;

/// The work one pass should do.
///
/// Empty in every field is the steady state and worth asserting on: a plan
/// that is not empty when nothing changed means something is re-uploading the
/// list on a timer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Send this machine's copy up.
    pub push: Vec<Key>,
    /// Take the server's copy down.
    pub pull: Vec<Key>,
    /// Delete here, because another machine deleted it and nothing here
    /// disagreed.
    pub delete_local: Vec<Key>,
    /// Delete on the server, because this machine deleted it and nothing there
    /// disagreed.
    pub delete_remote: Vec<Key>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// How many records this pass touches.
    pub fn len(&self) -> usize {
        self.push.len() + self.pull.len() + self.delete_local.len() + self.delete_remote.len()
    }
}

/// Decide what one pass should do.
pub fn plan(base: &Snapshot, local: &Snapshot, remote: &Snapshot) -> Plan {
    let mut plan = Plan::default();

    let keys: BTreeSet<&Key> = base
        .keys()
        .chain(local.keys())
        .chain(remote.keys())
        .collect();

    for key in keys {
        let based = base.get(key);
        let here = local.get(key);
        let there = remote.get(key);

        // Already agreed. The overwhelmingly common case, and the reason a
        // steady-state pass sends nothing.
        if here == there {
            continue;
        }

        let moved_here = here != based;
        let moved_there = there != based;

        let decision = match (here, there, moved_here, moved_there) {
            // Only this machine moved.
            (Some(version), _, true, false) => Some((Side::Local, *version)),
            // Only the other side moved.
            (_, Some(version), false, true) => Some((Side::Remote, *version)),

            // Neither moved but they differ, which means the base is lying —
            // it was written by a pass that did not finish, or the file was
            // restored from a backup. Resolving it is no worse than resolving
            // a genuine double edit, and refusing to would leave the two
            // permanently apart.
            (Some(_), Some(_), false, false) => Some(resolve(here.unwrap(), there.unwrap())),

            // Both moved.
            (Some(_), Some(_), true, true) => Some(resolve(here.unwrap(), there.unwrap())),

            // One side has never heard of it. A record that is simply absent
            // is not a deletion — only a marker is — so the side that has it
            // wins, whether that is a record newly created or one this machine
            // has lost track of.
            (Some(version), None, _, _) => Some((Side::Local, *version)),
            (None, Some(version), _, _) => Some((Side::Remote, *version)),

            // Gone from both, still in the base. Nothing to do but stop
            // mentioning it.
            (None, None, _, _) => None,
        };

        match decision {
            Some((Side::Local, Version::Live(_))) => plan.push.push(key.clone()),
            Some((Side::Local, Version::Deleted(_))) => plan.delete_remote.push(key.clone()),
            Some((Side::Remote, Version::Live(_))) => plan.pull.push(key.clone()),
            Some((Side::Remote, Version::Deleted(_))) => plan.delete_local.push(key.clone()),
            None => {}
        }
    }

    plan
}

/// Which copy wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Local,
    Remote,
}

/// Both sides changed the same record. Decide between them.
fn resolve(here: &Version, there: &Version) -> (Side, Version) {
    match (here.is_deleted(), there.is_deleted()) {
        // A deletion never wins over an edit, whichever side deleted.
        (true, false) => (Side::Remote, *there),
        (false, true) => (Side::Local, *here),

        // Otherwise the later one wins. On an exact tie the server's copy
        // takes it — not arbitrarily: every machine calls the same side
        // "remote", so all of them land on the same value, which a tie-break
        // on anything local would not do.
        _ => {
            if here.at() > there.at() {
                (Side::Local, *here)
            } else {
                (Side::Remote, *there)
            }
        }
    }
}

/// What this store currently holds, in the shape a pass compares.
pub fn snapshot_of(store: &Store) -> Snapshot {
    let mut snapshot = Snapshot::new();

    for task in store.tasks() {
        snapshot.insert(
            Key::new(RecordKind::Task, task.id.as_str()),
            Version::Live(task.updated_at),
        );
    }

    for project in store.projects() {
        snapshot.insert(
            Key::new(RecordKind::Project, project.id.as_str()),
            Version::Live(project.updated_at),
        );
    }
    for section in store.sections() {
        snapshot.insert(
            Key::new(RecordKind::Section, section.id.as_str()),
            Version::Live(section.updated_at),
        );
    }
    for label in store.labels() {
        snapshot.insert(
            Key::new(RecordKind::Label, label.id.as_str()),
            Version::Live(label.updated_at),
        );
    }
    for filter in store.filters() {
        snapshot.insert(
            Key::new(RecordKind::Filter, filter.id.as_str()),
            Version::Live(filter.updated_at),
        );
    }

    // Markers last, so a record that is somehow both listed and deleted reads
    // as deleted — the marker is the deliberate statement of the two.
    for tombstone in store.tombstones() {
        snapshot.insert(
            Key::new(tombstone.kind, tombstone.id.clone()),
            Version::Deleted(tombstone.deleted_at),
        );
    }

    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minute: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(2026, 8, 4)
            .unwrap()
            .and_hms_opt(12, minute, 0)
            .unwrap()
            .and_utc()
    }

    fn task(id: &str) -> Key {
        Key::new(RecordKind::Task, id)
    }

    fn snapshot(entries: &[(Key, Version)]) -> Snapshot {
        entries.iter().cloned().collect()
    }

    #[test]
    fn nothing_changed_means_nothing_to_do() {
        let state = snapshot(&[(task("a"), Version::Live(at(0)))]);
        // A plan that is never empty means something re-uploads the list on a
        // timer, which is the failure this asserts against.
        assert!(plan(&state, &state, &state).is_empty());
    }

    #[test]
    fn a_task_edited_here_is_pushed() {
        let base = snapshot(&[(task("a"), Version::Live(at(0)))]);
        let local = snapshot(&[(task("a"), Version::Live(at(5)))]);

        let plan = plan(&base, &local, &base);
        assert_eq!(plan.push, vec![task("a")]);
        assert_eq!(plan.len(), 1);
    }

    #[test]
    fn a_task_edited_elsewhere_is_pulled() {
        let base = snapshot(&[(task("a"), Version::Live(at(0)))]);
        let remote = snapshot(&[(task("a"), Version::Live(at(5)))]);

        let plan = plan(&base, &base, &remote);
        assert_eq!(plan.pull, vec![task("a")]);
    }

    #[test]
    fn a_task_created_here_is_pushed_rather_than_deleted() {
        // The trap this guards: absent on the server looks like deleted if you
        // only compare two snapshots.
        let plan = plan(
            &Snapshot::new(),
            &snapshot(&[(task("new"), Version::Live(at(1)))]),
            &Snapshot::new(),
        );
        assert_eq!(plan.push, vec![task("new")]);
        assert!(plan.delete_local.is_empty());
    }

    #[test]
    fn a_task_deleted_here_is_deleted_there() {
        let base = snapshot(&[(task("a"), Version::Live(at(0)))]);
        let local = snapshot(&[(task("a"), Version::Deleted(at(5)))]);

        let plan = plan(&base, &local, &base);
        assert_eq!(plan.delete_remote, vec![task("a")]);
    }

    #[test]
    fn a_task_deleted_elsewhere_is_deleted_here() {
        let base = snapshot(&[(task("a"), Version::Live(at(0)))]);
        let remote = snapshot(&[(task("a"), Version::Deleted(at(5)))]);

        let plan = plan(&base, &base, &remote);
        assert_eq!(plan.delete_local, vec![task("a")]);
    }

    #[test]
    fn an_edit_beats_a_deletion_whichever_side_deleted() {
        let base = snapshot(&[(task("a"), Version::Live(at(0)))]);

        // Deleted here, edited there — even though the deletion is later.
        let plan_one = plan(
            &base,
            &snapshot(&[(task("a"), Version::Deleted(at(9)))]),
            &snapshot(&[(task("a"), Version::Live(at(5)))]),
        );
        assert_eq!(plan_one.pull, vec![task("a")], "the edit brings it back");
        assert!(plan_one.delete_remote.is_empty());

        // And the other way round.
        let plan_two = plan(
            &base,
            &snapshot(&[(task("a"), Version::Live(at(5)))]),
            &snapshot(&[(task("a"), Version::Deleted(at(9)))]),
        );
        assert_eq!(plan_two.push, vec![task("a")]);
        assert!(plan_two.delete_local.is_empty());
    }

    #[test]
    fn two_edits_to_one_task_keep_the_later_one() {
        let base = snapshot(&[(task("a"), Version::Live(at(0)))]);

        let later_here = plan(
            &base,
            &snapshot(&[(task("a"), Version::Live(at(9)))]),
            &snapshot(&[(task("a"), Version::Live(at(5)))]),
        );
        assert_eq!(later_here.push, vec![task("a")]);

        let later_there = plan(
            &base,
            &snapshot(&[(task("a"), Version::Live(at(5)))]),
            &snapshot(&[(task("a"), Version::Live(at(9)))]),
        );
        assert_eq!(later_there.pull, vec![task("a")]);
    }

    #[test]
    fn the_same_version_on_both_sides_is_agreement_not_a_conflict() {
        // A version *is* its timestamp, so two sides holding the same one are
        // taken to hold the same record. Two machines editing one task in the
        // same microsecond would therefore stay apart — see the note on
        // `Version` about why that is not worth a content hash.
        let base = snapshot(&[(task("a"), Version::Live(at(0)))]);
        let plan = plan(
            &base,
            &snapshot(&[(task("a"), Version::Live(at(5)))]),
            &snapshot(&[(task("a"), Version::Live(at(5)))]),
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn a_machine_that_was_away_catches_up_in_one_pass() {
        // Nothing local moved since the base; four things happened elsewhere.
        let base = snapshot(&[
            (task("a"), Version::Live(at(0))),
            (task("b"), Version::Live(at(0))),
            (task("c"), Version::Live(at(0))),
        ]);
        let remote = snapshot(&[
            (task("a"), Version::Live(at(9))),
            (task("b"), Version::Deleted(at(9))),
            (task("c"), Version::Live(at(0))),
            (task("d"), Version::Live(at(9))),
        ]);

        let plan = plan(&base, &base, &remote);
        assert_eq!(plan.pull, vec![task("a"), task("d")]);
        assert_eq!(plan.delete_local, vec![task("b")]);
        assert!(plan.push.is_empty(), "nothing here moved");
    }

    #[test]
    fn a_deletion_both_sides_already_agree_on_is_not_work() {
        let state = snapshot(&[(task("a"), Version::Deleted(at(5)))]);
        assert!(plan(&state, &state, &state).is_empty());
    }

    #[test]
    fn a_record_gone_from_both_sides_is_simply_dropped() {
        let base = snapshot(&[(task("a"), Version::Live(at(0)))]);
        assert!(plan(&base, &Snapshot::new(), &Snapshot::new()).is_empty());
    }

    #[test]
    fn records_of_different_kinds_do_not_collide_on_the_same_id() {
        // Ids are unique in practice, but the plan must not depend on that.
        let base = Snapshot::new();
        let local = snapshot(&[
            (Key::new(RecordKind::Task, "x"), Version::Live(at(1))),
            (Key::new(RecordKind::Label, "x"), Version::Live(at(1))),
        ]);

        let plan = plan(&base, &local, &Snapshot::new());
        assert_eq!(plan.push.len(), 2);
    }
}

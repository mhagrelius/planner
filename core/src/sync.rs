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
use serde::{Deserialize, Serialize};

use crate::store::Store;
use crate::tombstone::RecordKind;

/// Which record a snapshot entry is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// One record, with its contents, on its way between machines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub kind: RecordKind,
    pub id: String,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// Why a pass could not finish.
///
/// One string, because every one of them means the same thing to the caller:
/// the server could not be reached or did not agree, so try again later. The
/// text is for a log line, not for branching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncError(pub String);

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SyncError {}

/// The other side, whatever is carrying it.
///
/// A trait so the core does not learn what a socket is. The GTK shell answers
/// it over plain HTTP; a shell on another platform can answer it with whatever
/// that platform already has, and neither needs the other's networking stack.
pub trait Remote {
    /// Every record the server holds, without the bodies.
    fn snapshot(&self) -> Result<Snapshot, SyncError>;
    /// The bodies of these records.
    fn fetch(&self, keys: &[Key]) -> Result<Vec<Record>, SyncError>;
    /// Store these, refusing any the server holds a newer copy of.
    fn push(&self, records: &[Record]) -> Result<(), SyncError>;
    /// Mark these deleted, on the same terms.
    fn delete(&self, records: &[Record]) -> Result<(), SyncError>;
}

/// What one pass brought back, waiting to be applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Incoming {
    /// Records to write into the store.
    pub records: Vec<Record>,
    /// Records to remove from it.
    pub deletions: Vec<Record>,
    /// The snapshot the two sides now agree on, for the next pass's base.
    ///
    /// **What happened, not what was planned.** A push that failed is left
    /// out, so a pass that dies half way retries next time instead of
    /// believing itself.
    pub base: Snapshot,
}

/// The half of a pass that talks to the network, and writes nothing.
///
/// Safe on a worker thread precisely because of that: it is handed a snapshot
/// taken on the main thread and gives back records. Every local write happens
/// in [`apply`], on the thread that owns the store — which is the only one that
/// knows which task is open in the detail panel and whether it has been typed
/// into since.
///
/// Pushes go first and deletions last, so a pass that fails part way has told
/// the server about work that exists rather than about work that is gone.
pub fn gather(
    remote: &dyn Remote,
    base: &Snapshot,
    local: &Snapshot,
    bodies: impl Fn(&Key) -> Option<serde_json::Value>,
) -> Result<Incoming, SyncError> {
    let there = remote.snapshot()?;
    let plan = plan(base, local, &there);

    let mut agreed = base.clone();

    // --- up ----------------------------------------------------------------

    let outgoing: Vec<Record> = plan
        .push
        .iter()
        .filter_map(|key| {
            // Gone between the snapshot and here. The next pass sees the
            // marker and sends the deletion instead.
            let body = bodies(key)?;
            Some(Record {
                kind: key.kind,
                id: key.id.clone(),
                updated_at: version_at(local.get(key))?,
                body: Some(body),
            })
        })
        .collect();
    if !outgoing.is_empty() {
        remote.push(&outgoing)?;
        for record in &outgoing {
            let key = Key::new(record.kind, record.id.clone());
            agreed.insert(key, Version::Live(record.updated_at));
        }
    }

    let removals: Vec<Record> = plan
        .delete_remote
        .iter()
        .filter_map(|key| {
            Some(Record {
                kind: key.kind,
                id: key.id.clone(),
                updated_at: version_at(local.get(key))?,
                body: None,
            })
        })
        .collect();
    if !removals.is_empty() {
        remote.delete(&removals)?;
        for record in &removals {
            let key = Key::new(record.kind, record.id.clone());
            agreed.insert(key, Version::Deleted(record.updated_at));
        }
    }

    // --- down --------------------------------------------------------------

    let records = if plan.pull.is_empty() {
        Vec::new()
    } else {
        remote.fetch(&plan.pull)?
    };

    let deletions: Vec<Record> = plan
        .delete_local
        .iter()
        .filter_map(|key| {
            Some(Record {
                kind: key.kind,
                id: key.id.clone(),
                updated_at: version_at(there.get(key))?,
                body: None,
            })
        })
        .collect();

    Ok(Incoming {
        records,
        deletions,
        base: agreed,
    })
}

fn version_at(version: Option<&Version>) -> Option<DateTime<Utc>> {
    version.map(Version::at)
}

/// What applying a pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub written: usize,
    pub removed: usize,
    /// Records that arrived but could not be read — a newer schema, or a
    /// record this build has no type for. Left alone rather than guessed at,
    /// and left out of the base so the next pass tries again.
    pub unreadable: usize,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn len(&self) -> usize {
        self.written + self.removed
    }
}

/// The half of a pass that writes, on the thread that owns the store.
///
/// Deletions go last, so a failure part way leaves records present rather than
/// gone — the recoverable mistake of the two.
///
/// `held` decides what to leave for later: the shell passes the task currently
/// open and being edited, because a pull landing on it would take the text out
/// from under the cursor. What is held is left out of the returned base, so the
/// next pass offers it again.
pub fn apply(
    store: &mut crate::store::Store,
    incoming: Incoming,
    held: impl Fn(&Key) -> bool,
    now: DateTime<Utc>,
) -> (Report, Snapshot) {
    let mut report = Report::default();
    let mut base = incoming.base;

    for record in incoming.records {
        let key = Key::new(record.kind, record.id.clone());
        if held(&key) {
            continue;
        }
        let Some(body) = record.body else {
            report.unreadable += 1;
            continue;
        };
        match store.merge_record(record.kind, body) {
            Ok(()) => {
                report.written += 1;
                base.insert(key, Version::Live(record.updated_at));
            }
            Err(_) => report.unreadable += 1,
        }
    }

    for record in incoming.deletions {
        let key = Key::new(record.kind, record.id.clone());
        if held(&key) {
            continue;
        }
        store.apply_deletion(record.kind, &record.id, record.updated_at);
        report.removed += 1;
        base.insert(key, Version::Deleted(record.updated_at));
    }

    let _ = now;
    (report, base)
}

/// Where the agreed snapshot is kept, beside the document.
///
/// Per machine, not shared: it records what *this* machine last agreed with the
/// server, and two machines have different answers.
pub fn default_base_path(document: &std::path::Path) -> std::path::PathBuf {
    document.with_file_name("sync-base.json")
}

/// Read the last agreed snapshot.
///
/// A missing or unreadable file is an empty base rather than an error. That is
/// the honest reading — this machine has agreed nothing — and it makes the
/// first pass a full comparison instead of a failure.
pub fn load_base(path: &std::path::Path) -> Snapshot {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<(Key, Version)>>(&raw).ok())
        .map(|entries| entries.into_iter().collect())
        .unwrap_or_default()
}

pub fn save_base(base: &Snapshot, path: &std::path::Path) -> std::io::Result<()> {
    let entries: Vec<(&Key, &Version)> = base.iter().collect();
    let encoded = serde_json::to_string(&entries)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, encoded)
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

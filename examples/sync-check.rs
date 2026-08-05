//! Two machines against one server, driven by the real client code.
//!
//! Run through `./sync-check.sh`, which stands the server up and hands this
//! the addresses. It exists because everything either side of the wire has
//! unit tests and the wire itself has none — and one machine can never
//! contradict itself, which is the only thing a sync has to survive.
//!
//! No GTK: a machine here is a `Store` plus a base snapshot, which is exactly
//! what the shell's sync tick holds. The transport is the shell's, so a change
//! that breaks the real client breaks this.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use planner::model::color::Color;
use planner::model::id::ProjectId;
use planner::model::store::Store;
use planner::model::sync::{self, Snapshot};
use planner::model::task::Task;
use planner::model::{Project, RecordKind};
use planner::ui::sync::HttpRemote;

/// One machine: its document, and what it last agreed with the server.
struct Machine {
    name: &'static str,
    store: Store,
    base: Snapshot,
    remote: HttpRemote,
}

impl Machine {
    fn new(name: &'static str, home: PathBuf, url: &str, token: &str) -> Self {
        let (store, _) = Store::open_at(home.join("planner.json"));
        Self {
            name,
            store,
            base: Snapshot::new(),
            remote: HttpRemote::new(url, token).expect("a url"),
        }
    }

    /// A full pass, the same two halves the shell runs.
    fn sync(&mut self, now: DateTime<Utc>) {
        let local = sync::snapshot_of(&self.store);
        let bodies: std::collections::BTreeMap<_, _> = local
            .keys()
            .filter_map(|key| {
                self.store
                    .record_body(key.kind, &key.id)
                    .map(|body| (key.clone(), body))
            })
            .collect();

        let incoming = sync::gather(&self.remote, &self.base, &local, |key| {
            bodies.get(key).cloned()
        })
        .unwrap_or_else(|error| panic!("{}: {error}", self.name));

        let (_, base) = sync::apply(&mut self.store, incoming, |_| false, now);
        self.base = base;
    }

    fn titles(&self) -> Vec<String> {
        let mut titles: Vec<String> = self
            .store
            .tasks()
            .iter()
            .map(|task| task.content.clone())
            .collect();
        titles.sort();
        titles
    }

    fn add(&mut self, title: &str, now: DateTime<Utc>) -> planner::model::TaskId {
        self.store
            .add_task(Task::new(ProjectId::inbox(), title, now))
    }
}

/// A moment, `step` minutes into the scenario.
///
/// Added to a base rather than written into the minute field, so a step past
/// the top of the hour is still a real time.
fn at(step: i64) -> DateTime<Utc> {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 5)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        + chrono::Duration::minutes(step)
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set — run ./sync-check.sh"))
}

fn check(name: &str, condition: bool) {
    if condition {
        println!("  ok   {name}");
    } else {
        println!("  FAIL {name}");
        std::process::exit(1);
    }
}

fn main() {
    let url = env("SYNC_CHECK_URL");
    let token = env("SYNC_CHECK_TOKEN");

    let mut a = Machine::new("A", PathBuf::from(env("SYNC_CHECK_A")), &url, &token);
    let mut b = Machine::new("B", PathBuf::from(env("SYNC_CHECK_B")), &url, &token);

    // --- a task added here arrives there --------------------------------
    a.add("Email Sam", at(0));
    a.sync(at(1));
    b.sync(at(2));
    check(
        "a task added on A arrives at B",
        b.titles().contains(&"Email Sam".to_string()),
    );

    // --- and a pass that changes nothing sends nothing -------------------
    let before = sync::snapshot_of(&b.store);
    b.sync(at(3));
    check(
        "a second pass with nothing new is a no-op",
        sync::snapshot_of(&b.store) == before,
    );

    // --- an edit propagates, and does not bounce back --------------------
    let id = b
        .store
        .tasks()
        .iter()
        .find(|task| task.content == "Email Sam")
        .map(|task| task.id.clone())
        .expect("B has it");
    {
        let task = b.store.task_mut(&id).expect("B has it");
        task.content = "Email Sam about the lease".into();
        task.touch(at(10));
    }
    b.sync(at(11));
    a.sync(at(12));
    check(
        "an edit on B reaches A",
        a.titles()
            .contains(&"Email Sam about the lease".to_string()),
    );
    check("and does not duplicate the task", a.titles().len() == 1);

    // --- a deletion survives, rather than being resurrected --------------
    a.store.remove_task(&id, at(20));
    a.sync(at(21));
    b.sync(at(22));
    check("a delete on A reaches B", b.titles().is_empty());

    // The trap this is really for: B still has its own memory of the task,
    // so a pass that could not tell "deleted" from "not seen" would send it
    // straight back up and A would get it again on the next pass.
    b.sync(at(23));
    a.sync(at(24));
    check("and the task does not come back", a.titles().is_empty());

    // --- both sides edit: one winner, and nothing lost -------------------
    let contested = a.add("Contested", at(30));
    a.sync(at(31));
    b.sync(at(32));

    a.store.task_mut(&contested).expect("A").content = "Contested by A".into();
    a.store.task_mut(&contested).expect("A").touch(at(40));
    b.store.task_mut(&contested).expect("B").content = "Contested by B".into();
    b.store.task_mut(&contested).expect("B").touch(at(41));

    a.sync(at(50));
    b.sync(at(51));
    a.sync(at(52));

    check(
        "a double edit leaves exactly one task",
        a.titles().len() == 1 && b.titles().len() == 1,
    );
    check(
        "both machines agree which edit won",
        a.titles() == b.titles(),
    );
    check(
        "and it is the later one",
        a.titles() == vec!["Contested by B".to_string()],
    );

    // --- an edit beats a deletion ---------------------------------------
    let survivor = a.add("Survivor", at(60));
    a.sync(at(61));
    b.sync(at(62));

    a.store.remove_task(&survivor, at(70));
    b.store.task_mut(&survivor).expect("B").content = "Survivor, edited".into();
    b.store.task_mut(&survivor).expect("B").touch(at(71));

    a.sync(at(80));
    b.sync(at(81));
    a.sync(at(82));

    // The rule that is a rule rather than a question: losing work is the one
    // unrecoverable failure, so the edit wins even though the deletion is
    // earlier and even on the machine that did the deleting.
    let survived = "Survivor, edited".to_string();
    check(
        "an edit beats a deletion, on the machine that deleted it",
        a.titles().contains(&survived),
    );
    check(
        "and on the machine that edited it",
        b.titles().contains(&survived),
    );

    // --- projects and sections travel too --------------------------------
    let work = a
        .store
        .add_project(Project::new("Work", Color::Blue), at(90));
    a.sync(at(91));
    b.sync(at(92));
    check(
        "a project reaches the other machine",
        b.store.project(&work).is_some(),
    );

    // --- a machine that was away catches up in one pass -------------------
    for (index, title) in ["One", "Two", "Three"].iter().enumerate() {
        a.add(title, at(100 + index as i64));
    }
    a.sync(at(110));
    // B has been switched off for all of that.
    b.sync(at(111));
    check(
        "a machine that was away catches up in one pass",
        ["One", "Two", "Three"]
            .iter()
            .all(|title| b.titles().contains(&title.to_string())),
    );

    // --- and the two are identical at the end ----------------------------
    a.sync(at(120));
    b.sync(at(121));
    a.sync(at(122));
    check("the two machines end up with the same list", {
        let mut left: Vec<_> = a.titles();
        let mut right: Vec<_> = b.titles();
        left.sort();
        right.sort();
        left == right
    });
    check(
        "and the same tombstones",
        a.store.tombstones().len() == b.store.tombstones().len(),
    );

    // --- and a waiting machine is woken rather than polled ----------------
    //
    // The part that fails silently if it is wrong: a client that is never
    // woken still syncs, just on its backstop timer, and every test above
    // would still pass. So this asserts the wake actually happens — and that
    // it happens *quickly*, which is the whole point of it existing.
    {
        use planner::model::sync::Remote;
        use std::sync::mpsc;

        // Where the server is now, so the wait has a cursor and does not
        // return immediately on history.
        let (_, cursor) = b
            .remote
            .wait_for_change(chrono::DateTime::UNIX_EPOCH)
            .expect("a cursor");

        let (sender, receiver) = mpsc::channel();
        let waiter = HttpRemote::new(&url, &token).expect("a url");
        std::thread::spawn(move || {
            let _ = sender.send(waiter.wait_for_change(cursor));
        });

        // Give the waiter a moment to actually be parked, then write.
        std::thread::sleep(std::time::Duration::from_millis(300));
        a.add("Woke you up", at(200));
        a.sync(at(201));

        // Well inside the server's fifty-second give-up, so arriving at all
        // means it was the notification rather than the timeout.
        match receiver.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok((changed, _))) => check("a waiting machine is woken by a write", changed),
            Ok(Err(error)) => {
                println!("  FAIL a waiting machine is woken by a write: {error}");
                std::process::exit(1);
            }
            Err(_) => {
                println!("  FAIL a waiting machine is woken by a write: it was not");
                std::process::exit(1);
            }
        }

        b.sync(at(202));
        check(
            "and finds the change when it looks",
            b.titles().contains(&"Woke you up".to_string()),
        );
    }

    // A record kind the client does not know must not take the pass down.
    check(
        "an unreadable record is skipped rather than fatal",
        a.store
            .merge_record(RecordKind::Task, serde_json::json!({"nonsense": true}))
            .is_err(),
    );
}

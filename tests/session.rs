//! Whole scenarios against the real store.
//!
//! The unit tests check one rule each. These check that the rules still hold
//! when strung together across a save and a reopen — which is where a field
//! that serialises but does not deserialise, or a mutation that forgets to
//! mark the store dirty, actually shows up.
//!
//! No GTK here either: a session is a sequence of model operations, and being
//! able to replay one without a display is the point of the split.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use planner::model::color::Color;
use planner::model::due::Due;
use planner::model::id::ProjectId;
use planner::model::parse::parse_quick_add;
use planner::model::project::{Project, Section};
use planner::model::query::Query;
use planner::model::recurrence::{End, Recurrence, Unit};
use planner::model::store::{LoadOutcome, SaveError, Store, SCHEMA_VERSION};
use planner::model::task::{Completion, Task};
use planner::model::Priority;
use tempfile::TempDir;

fn date(y: u32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y as i32, m, d).expect("a real date")
}

fn instant(y: u32, m: u32, d: u32) -> DateTime<Utc> {
    date(y, m, d).and_hms_opt(12, 0, 0).unwrap().and_utc()
}

/// Add a task the way the app does: through quick-add.
fn quick_add(store: &mut Store, line: &str, today: NaiveDate, now: DateTime<Utc>) -> Task {
    let parsed = parse_quick_add(line, today, &store.vocabulary());
    let id = store.add_from_quick_add(&parsed, &ProjectId::inbox(), None, now);
    store.task(&id).expect("just added").clone()
}

#[test]
fn a_days_work_survives_being_closed_and_reopened() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("planner.json");
    let today = date(2026, 7, 30);
    let now = instant(2026, 7, 30);

    let ids = {
        let (mut store, outcome) = Store::open_at(&path);
        assert_eq!(outcome, LoadOutcome::Fresh);

        let project = store.add_project(Project::new("Work", Color::Blue));
        store.add_section(Section::new(project.clone(), "Admin"));

        let a = quick_add(
            &mut store,
            "Email Sam about the lease #Work /Admin @email p2 friday 9am",
            today,
            now,
        );
        let b = quick_add(&mut store, "Water the plants every! 10 days", today, now);
        let c = quick_add(&mut store, "Buy 3 apples", today, now);

        store.save().unwrap();
        (a.id, b.id, c.id)
    };

    let (store, outcome) = Store::open_at(&path);
    assert_eq!(outcome, LoadOutcome::Loaded);

    let (email, plants, apples) = ids;

    let email = store.task(&email).expect("the emailed task");
    assert_eq!(email.content, "Email Sam about the lease");
    assert_eq!(email.priority, Priority::P2);
    assert_eq!(email.due.as_ref().unwrap().date, date(2026, 7, 31));
    assert_eq!(
        email.due.as_ref().unwrap().time,
        NaiveTime::from_hms_opt(9, 0, 0)
    );
    assert_eq!(store.project(&email.project_id).unwrap().name, "Work");
    assert!(email.section_id.is_some());
    assert_eq!(email.labels.len(), 1);

    let plants = store.task(&plants).expect("the plants");
    let rule = plants.due.as_ref().unwrap().recurrence.as_ref().unwrap();
    assert!(rule.from_completion);
    assert_eq!(rule.interval, 10);

    // The number stayed in the title rather than becoming a date.
    assert_eq!(store.task(&apples).unwrap().content, "Buy 3 apples");
    assert_eq!(store.task(&apples).unwrap().due, None);
}

#[test]
fn a_recurring_task_walks_forward_across_reopens_and_finally_finishes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("planner.json");

    let id = {
        let (mut store, _) = Store::open_at(&path);
        let mut task = Task::new(ProjectId::inbox(), "Standup", instant(2026, 7, 30));
        task.due = Some(Due::on(date(2026, 7, 30)).repeating(Recurrence {
            end: End::After { remaining: 2 },
            ..Recurrence::every(1, Unit::Day)
        }));
        let id = store.add_task(task);
        store.save().unwrap();
        id
    };

    // Two occurrences, each in its own session.
    for (day, expected) in [(30, date(2026, 7, 31)), (31, date(2026, 8, 1))] {
        let (mut store, _) = Store::open_at(&path);
        let outcome = store
            .complete_task(&id, instant(2026, 7, day), date(2026, 7, day))
            .unwrap();
        assert!(matches!(outcome, Completion::Rescheduled { .. }));
        assert_eq!(
            store.task(&id).unwrap().due.as_ref().unwrap().date,
            expected
        );
        assert!(!store.task(&id).unwrap().checked);
        store.save().unwrap();
    }

    // The third completion is the last one.
    let (mut store, _) = Store::open_at(&path);
    let outcome = store
        .complete_task(&id, instant(2026, 8, 1), date(2026, 8, 1))
        .unwrap();
    assert_eq!(outcome, Completion::Done);
    assert!(store.task(&id).unwrap().checked);
    store.save().unwrap();

    let (store, _) = Store::open_at(&path);
    let task = store.task(&id).unwrap();
    assert!(task.checked);
    assert!(
        !task.due.as_ref().unwrap().is_recurring(),
        "an exhausted rule must not survive the save"
    );
}

#[test]
fn the_built_in_views_hold_up_on_a_realistic_store() {
    let dir = TempDir::new().unwrap();
    let (mut store, _) = Store::open_at(dir.path().join("planner.json"));
    let today = date(2026, 7, 30);
    let now = instant(2026, 7, 30);

    let work = store.add_project(Project::new("Work", Color::Blue));
    let mut admin = Project::new("Admin", Color::Teal);
    admin.parent_id = Some(work.clone());
    store.add_project(admin);

    quick_add(&mut store, "Renew passport today p1", today, now);
    quick_add(&mut store, "Call the bank tomorrow", today, now);
    quick_add(&mut store, "No date at all @someday", today, now);
    quick_add(&mut store, "Weekly review every monday #Work", today, now);
    let overdue = quick_add(&mut store, "Late thing", today, now);
    store.task_mut(&overdue.id).unwrap().due = Some(Due::on(date(2026, 7, 1)));
    let pinned = quick_add(&mut store, "Important", today, now);
    store.task_mut(&pinned.id).unwrap().pinned = true;

    let names = |source: &str| -> Vec<String> {
        let query = Query::parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let mut found: Vec<String> = store
            .query(&query, today)
            .into_iter()
            .map(|task| task.content.clone())
            .collect();
        found.sort();
        found
    };

    assert_eq!(
        names("due: today | overdue"),
        vec!["Late thing", "Renew passport"]
    );
    assert_eq!(
        names("due after: today"),
        vec!["Call the bank", "Weekly review"]
    );
    assert_eq!(names("no date"), vec!["Important", "No date at all"]);
    assert_eq!(names("pinned"), vec!["Important"]);
    assert_eq!(names("recurring"), vec!["Weekly review"]);
    assert_eq!(names("@someday"), vec!["No date at all"]);
    assert_eq!(names("p1"), vec!["Renew passport"]);
    assert_eq!(names("##Work"), vec!["Weekly review"]);
    assert!(names("completed").is_empty());
}

#[test]
fn completing_and_undoing_leaves_the_store_where_it_started() {
    let dir = TempDir::new().unwrap();
    let (mut store, _) = Store::open_at(dir.path().join("planner.json"));
    let today = date(2026, 7, 30);

    let parent = quick_add(&mut store, "Move house", today, instant(2026, 7, 30));
    let child = quick_add(&mut store, "Pack the kitchen", today, instant(2026, 7, 30));
    store.task_mut(&child.id).unwrap().parent_id = Some(parent.id.clone());

    store.complete_task(&parent.id, instant(2026, 7, 30), today);
    assert!(store.task(&parent.id).unwrap().checked);
    assert!(store.task(&child.id).unwrap().checked);

    store.uncomplete_task(&child.id, instant(2026, 7, 30));
    assert!(!store.task(&child.id).unwrap().checked);
    assert!(!store.task(&parent.id).unwrap().checked);

    // And a delete can be undone whole.
    let removed = store.remove_task(&parent.id);
    assert_eq!(removed.len(), 2);
    store.restore_tasks(removed);
    assert!(store.task(&parent.id).is_some());
    assert!(store.task(&child.id).is_some());
}

#[test]
fn a_corrupt_file_costs_the_data_but_not_the_app() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("planner.json");

    {
        let (mut store, _) = Store::open_at(&path);
        quick_add(
            &mut store,
            "Something valuable",
            date(2026, 7, 30),
            instant(2026, 7, 30),
        );
        store.save().unwrap();
    }

    // Something truncates the file.
    std::fs::write(&path, "{\"version\":1,\"tasks\":[{\"id\"").unwrap();

    let (mut store, outcome) = Store::open_at(&path);
    let LoadOutcome::Recovered { backup, .. } = outcome else {
        panic!("expected a recovery, got {outcome:?}");
    };
    assert!(backup.exists(), "the damaged file is kept for the user");
    assert!(store.tasks().is_empty());

    // The app carries on and the next save works.
    quick_add(
        &mut store,
        "Starting again",
        date(2026, 7, 31),
        instant(2026, 7, 31),
    );
    store.save().unwrap();

    let (store, outcome) = Store::open_at(&path);
    assert_eq!(outcome, LoadOutcome::Loaded);
    assert_eq!(store.tasks().len(), 1);
}

#[test]
fn a_file_from_a_future_version_is_left_exactly_as_it_was_found() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("planner.json");
    let original = format!(
        r#"{{"version":{},"projects":[],"labels":[],"tasks":[],"something_new":42}}"#,
        SCHEMA_VERSION + 1
    );
    std::fs::write(&path, &original).unwrap();

    let (mut store, outcome) = Store::open_at(&path);
    assert!(matches!(outcome, LoadOutcome::ReadOnly { .. }));

    // Even after the user edits away happily for an hour.
    quick_add(
        &mut store,
        "This will not be kept",
        date(2026, 7, 30),
        instant(2026, 7, 30),
    );
    assert!(matches!(store.save(), Err(SaveError::Newer { .. })));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn deleting_a_project_and_undoing_it_restores_every_task() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("planner.json");
    let today = date(2026, 7, 30);

    let (mut store, _) = Store::open_at(&path);
    let work = store.add_project(Project::new("Work", Color::Blue));
    let mut admin = Project::new("Admin", Color::Teal);
    admin.parent_id = Some(work.clone());
    store.add_project(admin);

    quick_add(&mut store, "In work #Work", today, instant(2026, 7, 30));
    quick_add(&mut store, "In admin #Admin", today, instant(2026, 7, 30));
    quick_add(&mut store, "In the inbox", today, instant(2026, 7, 30));

    let removed = store.remove_project(&work).expect("the project");
    assert_eq!(store.tasks().len(), 1, "only the inbox task is left");

    store.restore_project(removed);
    store.save().unwrap();

    let (store, _) = Store::open_at(&path);
    assert_eq!(store.tasks().len(), 3);
    assert_eq!(store.projects().len(), 3); // inbox, Work, Admin
}

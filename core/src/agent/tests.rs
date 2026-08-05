//! What an assistant driving this would actually run into.
//!
//! These go through [`parse`] and [`execute`] rather than calling the store,
//! because the interface *is* the argument list and the JSON — a change that
//! keeps the store correct while renaming a field or dropping a candidate list
//! has still broken the caller, and only a test at this level notices.

use super::*;
use crate::color::Color;
use crate::project::Section;
use crate::store::SCHEMA_VERSION;
use crate::{Priority, Task};
use serde_json::Value;
use tempfile::TempDir;

/// Thursday, 30 July 2026.
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
}

fn now() -> DateTime<Utc> {
    today().and_hms_opt(12, 0, 0).unwrap().and_utc()
}

fn store() -> (TempDir, Store) {
    let dir = TempDir::new().expect("a temporary directory");
    let (store, _) = Store::open_at(dir.path().join("planner.json"));
    (dir, store)
}

/// Run a command line the way the application does.
fn run(store: &mut Store, line: &str) -> Result<Response, AgentError> {
    let args: Vec<String> = shell_words(line);
    let command = parse(&args)?;
    execute(store, command, now(), today())
}

/// Split a line into arguments, honouring single quotes.
///
/// The real caller passes an argument list and never goes near a shell; this
/// only exists so a test reads like the command it stands for.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;

    for character in line.chars() {
        match character {
            '\'' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

/// The JSON a command prints.
fn json(store: &mut Store, line: &str) -> Value {
    let result = run(store, line);
    serde_json::from_str(&render(&result)).expect("the response is JSON")
}

fn error(store: &mut Store, line: &str) -> AgentError {
    run(store, line).expect_err("this should not have worked")
}

// --- adding -------------------------------------------------------------

#[test]
fn a_task_is_added_from_the_same_line_the_dialog_would_take() {
    let (_dir, mut store) = store();
    let work = store.add_project(Project::new("Work", Color::Blue));
    store
        .project_mut(&work)
        .unwrap()
        .add_section(Section::new("Admin"));

    let response = json(
        &mut store,
        "add Email Sam about the lease #Work /Admin @email p2 friday 9am",
    );

    assert_eq!(response["ok"], true);
    assert_eq!(response["action"], "added");
    let task = &response["task"];
    assert_eq!(task["content"], "Email Sam about the lease");
    assert_eq!(task["project"], "Work");
    assert_eq!(task["section"], "Admin");
    assert_eq!(task["priority"], "p2");
    assert_eq!(task["labels"][0], "email");
    assert_eq!(task["due"], "2026-07-31 09:00");
}

#[test]
fn a_task_with_no_project_named_lands_in_the_inbox() {
    let (_dir, mut store) = store();
    let response = json(&mut store, "add Buy milk");
    assert_eq!(response["task"]["project"], "Inbox");
}

#[test]
fn a_repeat_comes_back_as_the_phrase_that_made_it() {
    let (_dir, mut store) = store();
    let response = json(&mut store, "add Water the plants every other monday");
    assert_eq!(response["task"]["content"], "Water the plants");
    assert_eq!(response["task"]["repeats"], "every other monday");
}

#[test]
fn a_line_that_is_all_tokens_has_nothing_to_call_the_task() {
    let (_dir, mut store) = store();
    let error = error(&mut store, "add #Work p1 friday");
    assert_eq!(error.kind, ErrorKind::BadValue);
    assert!(error.hint.is_some(), "it should say what to do instead");
    assert!(store.tasks().is_empty(), "nothing was created");
}

#[test]
fn a_misspelled_project_does_not_quietly_become_a_new_one() {
    let (_dir, mut store) = store();
    store.add_project(Project::new("Work", Color::Blue));

    let response = json(&mut store, "add Something #Wrok");

    assert_eq!(store.projects().len(), 2, "no project was invented");
    assert_eq!(
        response["task"]["project"], "Inbox",
        "it lands somewhere visible instead"
    );
}

#[test]
fn a_subtask_shares_its_parents_project_whatever_the_line_says() {
    let (_dir, mut store) = store();
    let work = store.add_project(Project::new("Work", Color::Blue));
    store.add_task(Task::new(work, "Move house", now()));

    let response = json(&mut store, "subtask 'Move house' Pack the books #Inbox p1");

    assert_eq!(response["task"]["content"], "Pack the books");
    assert_eq!(response["task"]["project"], "Work");
    assert_eq!(response["task"]["priority"], "p1");
}

// --- completing ---------------------------------------------------------

#[test]
fn completing_a_task_says_it_is_done() {
    let (_dir, mut store) = store();
    store.add_task(Task::new(ProjectId::inbox(), "Email Sam", now()));

    let response = json(&mut store, "complete Email Sam");

    assert_eq!(response["outcome"], "done");
    assert_eq!(response["task"]["completed"], true);
}

#[test]
fn completing_a_repeating_task_says_it_moved_on_rather_than_that_it_is_done() {
    // The one answer this interface must never get wrong: a recurring task
    // that was ticked off is still outstanding, on a later day.
    let (_dir, mut store) = store();
    run(&mut store, "add Water the plants every week").unwrap();

    let response = json(&mut store, "complete Water the plants");

    assert_eq!(response["outcome"], "completed-and-repeats");
    assert_eq!(response["next_due"], "2026-08-06");
    assert_eq!(response["ok"], true, "the completion itself succeeded");
    assert_eq!(
        response["task"]["completed"],
        Value::Null,
        "an omitted `completed` is how the task says it is still open"
    );
}

#[test]
fn completing_something_already_done_changes_nothing() {
    let (_dir, mut store) = store();
    store.add_task(Task::new(ProjectId::inbox(), "Email Sam", now()));
    run(&mut store, "complete Email Sam").unwrap();

    let response = run(&mut store, "complete Email Sam").unwrap();

    assert!(matches!(
        response,
        Response::Completed {
            outcome: view::CompletionView::AlreadyDone,
            ..
        }
    ));
    assert!(
        !response.changed_the_store(),
        "a no-op must not mark the store dirty"
    );
}

#[test]
fn reopening_something_that_was_never_finished_says_it_undid_nothing() {
    let (_dir, mut store) = store();
    store.add_task(Task::new(ProjectId::inbox(), "Still to do", now()));

    let response = run(&mut store, "reopen Still to do").unwrap();

    assert!(matches!(
        response,
        Response::Reopened {
            reopened: false,
            ..
        }
    ));
    assert!(
        !response.changed_the_store(),
        "a no-op must not mark the store dirty"
    );
}

#[test]
fn reopening_a_subtask_reopens_the_parents_above_it() {
    let (_dir, mut store) = store();
    store.add_task(Task::new(ProjectId::inbox(), "Move house", now()));
    run(&mut store, "subtask 'Move house' Pack").unwrap();
    run(&mut store, "complete Move house").unwrap();

    run(&mut store, "reopen Pack").unwrap();

    let response = json(&mut store, "show Move house");
    assert_eq!(response["task"]["completed"], Value::Null);
}

// --- naming the thing you meant -----------------------------------------

#[test]
fn a_reference_matching_two_open_tasks_is_refused_with_both_ids() {
    let (_dir, mut store) = store();
    let work = store.add_project(Project::new("Work", Color::Blue));
    store.add_task(Task::new(ProjectId::inbox(), "Email Sam", now()));
    store.add_task(Task::new(work, "Email Sam again", now()));

    let error = error(&mut store, "complete Email");

    assert_eq!(error.kind, ErrorKind::Ambiguous);
    assert_eq!(error.candidates.len(), 2);
    // Every candidate carries the id that would have been unambiguous, and
    // enough context to choose between them.
    for candidate in &error.candidates {
        assert!(!candidate.id.is_empty());
        assert!(candidate.context.is_some());
    }
    assert!(
        store.tasks().iter().all(|task| !task.checked),
        "nothing was completed on a guess"
    );
}

#[test]
fn an_open_task_wins_over_a_completed_one_of_the_same_name() {
    let (_dir, mut store) = store();
    let old = store.add_task(Task::new(ProjectId::inbox(), "Weekly report", now()));
    store.complete_task(&old, now(), today());
    let current = store.add_task(Task::new(ProjectId::inbox(), "Weekly report", now()));

    assert_eq!(resolve_task(&store, "Weekly report").unwrap(), current);
}

#[test]
fn a_completed_task_can_still_be_named_when_it_is_the_only_one() {
    let (_dir, mut store) = store();
    let id = store.add_task(Task::new(ProjectId::inbox(), "Filed the taxes", now()));
    store.complete_task(&id, now(), today());

    let response = json(&mut store, "reopen Filed the taxes");
    assert_eq!(response["ok"], true);
}

#[test]
fn an_exact_title_beats_a_longer_one_that_merely_contains_it() {
    let (_dir, mut store) = store();
    let exact = store.add_task(Task::new(ProjectId::inbox(), "Pack", now()));
    store.add_task(Task::new(ProjectId::inbox(), "Pack the books", now()));

    assert_eq!(resolve_task(&store, "Pack").unwrap(), exact);
}

#[test]
fn an_id_is_always_taken_literally() {
    let (_dir, mut store) = store();
    let first = store.add_task(Task::new(ProjectId::inbox(), "Email Sam", now()));
    store.add_task(Task::new(ProjectId::inbox(), "Email Sam", now()));

    // Ambiguous by name, exact by id.
    assert_eq!(
        error(&mut store, "show Email Sam").kind,
        ErrorKind::Ambiguous
    );
    let response = json(&mut store, &format!("show {first}"));
    assert_eq!(response["task"]["id"], first.to_string());
}

#[test]
fn a_task_that_is_not_there_says_how_to_look_for_it() {
    let (_dir, mut store) = store();
    let error = error(&mut store, "complete Nothing like this");
    assert_eq!(error.kind, ErrorKind::NotFound);
    assert!(
        error.hint.unwrap().contains("search"),
        "the way out of a miss is a search"
    );
}

#[test]
fn naming_a_project_that_does_not_exist_says_so_rather_than_creating_it() {
    let (_dir, mut store) = store();
    store.add_task(Task::new(ProjectId::inbox(), "Email Sam", now()));

    let error = error(&mut store, "update Email project=Nowhere");

    assert_eq!(error.kind, ErrorKind::NotFound);
    assert_eq!(store.projects().len(), 1);
}

// --- listing and understanding ------------------------------------------

#[test]
fn a_query_is_the_same_language_the_app_filters_with() {
    let (_dir, mut store) = store();
    store.add_project(Project::new("Work", Color::Blue));
    run(&mut store, "add Urgent thing #Work p1 today").unwrap();
    run(&mut store, "add Lesser thing #Work p3 today").unwrap();
    run(&mut store, "add Home thing today").unwrap();

    let response = json(&mut store, "list #Work & p1");

    assert_eq!(response["count"], 1);
    assert_eq!(response["tasks"][0]["content"], "Urgent thing");
}

#[test]
fn a_list_is_soonest_first_so_two_calls_can_be_compared() {
    let (_dir, mut store) = store();
    run(&mut store, "add Later next friday").unwrap();
    run(&mut store, "add Sooner tomorrow").unwrap();
    run(&mut store, "add Undated").unwrap();

    let response = json(&mut store, "list");
    let order: Vec<&str> = response["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["content"].as_str().unwrap())
        .collect();

    assert_eq!(order, vec!["Sooner", "Later", "Undated"]);
}

#[test]
fn a_truncated_list_says_how_many_there_really_were() {
    let (_dir, mut store) = store();
    for index in 0..10 {
        run(&mut store, &format!("add Task number {index}")).unwrap();
    }

    let response = json(&mut store, "list limit=3");

    assert_eq!(response["count"], 3);
    assert_eq!(response["matched"], 10);
    assert_eq!(response["truncated"], true);
}

#[test]
fn a_full_list_is_not_marked_truncated() {
    let (_dir, mut store) = store();
    run(&mut store, "add Only one").unwrap();
    let response = json(&mut store, "list");
    assert_eq!(response["count"], 1);
    assert_eq!(response["truncated"], Value::Null);
}

#[test]
fn completed_tasks_stay_out_of_a_list_until_the_query_asks_for_them() {
    let (_dir, mut store) = store();
    run(&mut store, "add Finished").unwrap();
    run(&mut store, "complete Finished").unwrap();

    assert_eq!(json(&mut store, "list")["count"], 0);
    assert_eq!(json(&mut store, "list completed")["count"], 1);
}

#[test]
fn a_query_that_will_not_parse_points_at_the_syntax() {
    let (_dir, mut store) = store();
    let error = error(&mut store, "list due: nonsenseday");
    assert_eq!(error.kind, ErrorKind::BadQuery);
    assert!(error.hint.unwrap().contains("help list"));
}

#[test]
fn an_overview_is_enough_to_work_out_what_to_ask_next() {
    let (_dir, mut store) = store();
    let work = store.add_project(Project::new("Work", Color::Blue));
    store
        .project_mut(&work)
        .unwrap()
        .add_section(Section::new("Admin"));
    run(&mut store, "add Overdue thing #Work yesterday").unwrap();
    run(&mut store, "add Due today #Work today @email").unwrap();
    run(&mut store, "add Inbox thing").unwrap();
    run(&mut store, "add Done thing").unwrap();
    run(&mut store, "complete Done thing").unwrap();

    let response = json(&mut store, "overview");

    let projects = response["projects"].as_array().unwrap();
    let work = projects
        .iter()
        .find(|project| project["name"] == "Work")
        .expect("Work is listed");
    assert_eq!(work["sections"][0], "Admin");
    assert_eq!(work["open"], 2);

    assert_eq!(response["labels"][0]["name"], "email");
    assert_eq!(response["labels"][0]["open"], 1);

    let counts = &response["counts"];
    assert_eq!(counts["open"], 3);
    assert_eq!(counts["completed"], 1);
    assert_eq!(counts["overdue"], 1);
    assert_eq!(counts["due_today"], 1);
    assert_eq!(counts["inbox"], 1);
}

#[test]
fn show_carries_the_things_a_list_leaves_out() {
    let (_dir, mut store) = store();
    run(&mut store, "add Move house friday !30m").unwrap();
    run(&mut store, "subtask 'Move house' Pack the books").unwrap();
    store
        .task_mut(&resolve_task(&store, "Move house").unwrap().clone())
        .unwrap()
        .description = "Ring the agent first".into();

    let response = json(&mut store, "show Move house");

    assert_eq!(response["task"]["description"], "Ring the agent first");
    assert_eq!(response["task"]["subtasks"][0]["content"], "Pack the books");
    assert_eq!(response["task"]["reminders"][0], "30 minutes before");
}

#[test]
fn search_finds_projects_and_labels_as_well_as_tasks() {
    let (_dir, mut store) = store();
    store.add_project(Project::new("Leasehold", Color::Blue));
    run(&mut store, "add Email Sam about the lease").unwrap();

    let response = json(&mut store, "search lease");
    let kinds: Vec<&str> = response["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["kind"].as_str().unwrap())
        .collect();

    assert!(kinds.contains(&"task"));
    assert!(kinds.contains(&"project"));
}

// --- updating -----------------------------------------------------------

#[test]
fn a_date_is_set_with_the_same_phrases_that_can_be_typed() {
    let (_dir, mut store) = store();
    run(&mut store, "add Email Sam").unwrap();

    let response = json(&mut store, "update Email Sam due=next friday 9am");

    assert_eq!(response["task"]["due"], "2026-08-07 09:00");
    assert_eq!(response["applied"][0], "due → 2026-08-07 09:00");
}

#[test]
fn a_repeat_can_be_set_after_the_fact() {
    let (_dir, mut store) = store();
    run(&mut store, "add Water the plants").unwrap();

    let response = json(&mut store, "update Water due=every 3 days");

    assert_eq!(response["task"]["repeats"], "every 3 days");
}

#[test]
fn clearing_a_due_date_takes_the_repeat_with_it() {
    let (_dir, mut store) = store();
    run(&mut store, "add Water the plants every week").unwrap();

    let response = json(&mut store, "update Water due=none");

    assert_eq!(response["task"]["due"], Value::Null);
    assert_eq!(response["task"]["repeats"], Value::Null);
}

#[test]
fn a_date_with_a_word_in_it_that_was_not_understood_is_refused() {
    // Taking the part it recognised would file the task on a day nobody chose.
    let (_dir, mut store) = store();
    run(&mut store, "add Email Sam").unwrap();

    let error = error(&mut store, "update Email due=sometime friday");

    assert_eq!(error.kind, ErrorKind::BadDate);
    assert!(error.message.contains("sometime"), "{}", error.message);
    assert!(store.tasks()[0].due.is_none(), "nothing was set");
}

#[test]
fn a_field_set_to_what_it_already_was_reports_no_change() {
    let (_dir, mut store) = store();
    run(&mut store, "add Email Sam p1").unwrap();

    let response = run(&mut store, "update Email priority=p1").unwrap();

    let Response::Updated { applied, .. } = &response else {
        panic!("an update");
    };
    assert!(applied.is_empty(), "nothing changed, and it says so");
    assert!(!response.changed_the_store());
}

#[test]
fn several_fields_are_set_in_one_call() {
    let (_dir, mut store) = store();
    store.add_project(Project::new("Work", Color::Blue));
    run(&mut store, "add Email Sam").unwrap();

    let response = json(
        &mut store,
        "update Email project=Work priority=p1 add-label=urgent pinned=true",
    );

    let task = &response["task"];
    assert_eq!(task["project"], "Work");
    assert_eq!(task["priority"], "p1");
    assert_eq!(task["labels"][0], "urgent");
    assert_eq!(task["pinned"], true);
    assert_eq!(response["applied"].as_array().unwrap().len(), 4);
}

#[test]
fn a_moved_task_takes_its_subtasks_to_the_new_project() {
    let (_dir, mut store) = store();
    store.add_project(Project::new("Work", Color::Blue));
    run(&mut store, "add Move house").unwrap();
    run(&mut store, "subtask 'Move house' Pack the books").unwrap();

    run(&mut store, "update Move house project=Work").unwrap();

    let response = json(&mut store, "show Pack the books");
    assert_eq!(response["task"]["project"], "Work");
}

#[test]
fn a_section_is_looked_for_in_the_project_the_task_is_moving_to() {
    let (_dir, mut store) = store();
    let work = store.add_project(Project::new("Work", Color::Blue));
    store
        .project_mut(&work)
        .unwrap()
        .add_section(Section::new("Doing"));
    run(&mut store, "add Email Sam").unwrap();

    let response = json(&mut store, "update Email project=Work section=Doing");

    assert_eq!(response["task"]["project"], "Work");
    assert_eq!(response["task"]["section"], "Doing");
}

#[test]
fn a_section_the_project_does_not_have_lists_the_ones_it_does() {
    let (_dir, mut store) = store();
    let work = store.add_project(Project::new("Work", Color::Blue));
    store
        .project_mut(&work)
        .unwrap()
        .add_section(Section::new("Doing"));
    run(&mut store, "add Email Sam #Work").unwrap();

    let error = error(&mut store, "update Email section=Blocked");

    assert_eq!(error.kind, ErrorKind::NotFound);
    assert!(error.hint.unwrap().contains("Doing"));
}

#[test]
fn a_label_is_created_by_using_it_and_survives_being_taken_off_a_task() {
    let (_dir, mut store) = store();
    run(&mut store, "add Email Sam").unwrap();

    run(&mut store, "update Email add-label=urgent").unwrap();
    assert_eq!(store.labels().len(), 1);

    let response = json(&mut store, "update Email remove-label=urgent");
    assert_eq!(response["task"]["labels"], Value::Null);
    assert_eq!(store.labels().len(), 1, "the label itself stays");
}

// --- deleting -----------------------------------------------------------

#[test]
fn deleting_a_task_reports_the_subtasks_that_went_with_it() {
    let (_dir, mut store) = store();
    run(&mut store, "add Move house").unwrap();
    run(&mut store, "subtask 'Move house' Pack the books").unwrap();
    run(&mut store, "subtask 'Move house' Book a van").unwrap();

    let response = json(&mut store, "delete Move house");

    assert_eq!(response["count"], 3);
    assert!(store.tasks().is_empty());
}

// --- projects -----------------------------------------------------------

#[test]
fn a_project_can_be_created_under_another() {
    let (_dir, mut store) = store();
    store.add_project(Project::new("Home", Color::Blue));

    let response = json(&mut store, "add-project Loft conversion parent=Home");

    assert_eq!(response["project"]["name"], "Loft conversion");
    assert_eq!(response["project"]["parent"], "Home");
}

#[test]
fn deleting_a_project_says_how_much_went_with_it() {
    let (_dir, mut store) = store();
    store.add_project(Project::new("Work", Color::Blue));
    run(&mut store, "add Email Sam #Work").unwrap();
    run(&mut store, "add Ring Pat #Work").unwrap();

    let response = json(&mut store, "remove-project Work");

    assert_eq!(response["name"], "Work");
    assert_eq!(response["projects"], 1);
    assert_eq!(response["tasks"], 2);
}

#[test]
fn the_inbox_cannot_be_deleted_or_renamed() {
    let (_dir, mut store) = store();

    assert_eq!(
        error(&mut store, "remove-project Inbox").kind,
        ErrorKind::Refused
    );
    assert_eq!(
        error(&mut store, "rename-project Inbox Elsewhere").kind,
        ErrorKind::Refused
    );
    assert!(store.project(&ProjectId::inbox()).is_some());
}

// --- the wire ------------------------------------------------------------

#[test]
fn a_success_and_a_failure_are_told_apart_without_the_exit_code() {
    let (_dir, mut store) = store();

    let good: Value = serde_json::from_str(&render(&run(&mut store, "overview"))).unwrap();
    assert_eq!(good["ok"], true);
    assert_eq!(good["action"], "overview");

    let bad: Value = serde_json::from_str(&render(&run(&mut store, "show Nothing"))).unwrap();
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["error"], "not-found");
    assert!(bad["message"].as_str().unwrap().ends_with('.'));
}

#[test]
fn help_is_text_rather_than_json() {
    let (_dir, mut store) = store();
    let text = render(&run(&mut store, "help"));
    assert!(text.starts_with("planner agent"), "{text}");
    assert!(!text.starts_with('{'), "help is read, not parsed");
}

#[test]
fn describe_carries_every_verb_with_what_it_returns() {
    let (_dir, mut store) = store();
    let response = json(&mut store, "describe");

    let verbs = response["verbs"].as_array().expect("a list of verbs");
    assert_eq!(verbs.len(), help::VERBS.len());
    for verb in verbs {
        assert!(verb["name"].is_string());
        assert!(verb["usage"].is_string());
        assert!(verb["returns"].is_string());
        assert!(verb["mutates"].is_boolean());
    }
    // And it says which verbs write, which is what a caller gating writes
    // behind a confirmation reads.
    let complete = verbs
        .iter()
        .find(|verb| verb["name"] == "complete")
        .unwrap();
    assert_eq!(complete["mutates"], true);
    let list = verbs.iter().find(|verb| verb["name"] == "list").unwrap();
    assert_eq!(list["mutates"], false);
}

#[test]
fn a_file_from_a_newer_planner_can_be_read_but_not_written() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("planner.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"version":{},"projects":[],"labels":[],"tasks":[]}}"#,
            SCHEMA_VERSION + 1
        ),
    )
    .unwrap();
    let (mut store, _) = Store::open_at(&path);

    assert!(run(&mut store, "overview").is_ok(), "reading still works");

    let error = error(&mut store, "add Anything");
    assert_eq!(error.kind, ErrorKind::ReadOnly);
    assert!(
        store.tasks().is_empty(),
        "and nothing was changed in memory"
    );
}

#[test]
fn every_write_verb_is_refused_on_a_read_only_store() {
    // One test rather than one per verb, because the guard is one check and
    // what matters is that no verb slipped past the list it checks against.
    let lines = [
        "add Something",
        "subtask Parent Something",
        "complete Something",
        "reopen Something",
        "delete Something",
        "update Something priority=p1",
        "add-project Something",
        "rename-project Something Else",
        "remove-project Something",
    ];
    for line in lines {
        let command = parse(&shell_words(line)).expect("a command");
        assert!(
            command.changes_the_store(),
            "`{line}` writes but does not say so, so it would run against a read-only file"
        );
    }
}

#[test]
fn the_priority_a_task_comes_back_with_is_the_one_that_was_asked_for() {
    let (_dir, mut store) = store();
    for token in ["p1", "p2", "p3", "p4"] {
        let response = json(&mut store, &format!("add Task {token} {token}"));
        assert_eq!(response["task"]["priority"], token);
    }
    assert_eq!(
        store
            .tasks()
            .iter()
            .filter(|task| task.priority == Priority::P1)
            .count(),
        1
    );
}

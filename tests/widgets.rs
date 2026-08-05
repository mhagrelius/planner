//! The real widgets, headless.
//!
//! **Why this file has one `#[test]`.** GTK may be initialised from exactly
//! one thread and every widget call must come from it, but Rust's test
//! harness spawns a thread per `#[test]` — and `--test-threads=1` only
//! serialises them, it does not make them share a thread. So there is one
//! test, and a runner inside it that names each case and carries on after a
//! failure, which gets back the one thing a `#[test]` per case was buying.
//!
//! These cover the wiring the model tests cannot: that a row bound to a task
//! shows the right thing, that recycling a row does not leave it driven by
//! the task it used to show, and that the sidebar's queries agree with the
//! list the window renders.

use gtk::glib;
use gtk::prelude::*;

use chrono::NaiveDate;
use planner::model::color::Color;
use planner::model::due::Due;
use planner::model::id::ProjectId;
use planner::model::parse::Vocabulary;
use planner::model::project::{Project, SavedFilter, Section};
use planner::model::query::Query;
use planner::model::recurrence::{Recurrence, Unit};
use planner::model::store::Store;
use planner::model::task::Task;
use planner::model::{Priority, ViewStyle};
use planner::ui::detail_panel::{DetailPanel, Edit};
use planner::ui::project_view::ProjectView;
use planner::ui::quick_add::QuickAddDialog;
use planner::ui::quick_find::QuickFindDialog;
use planner::ui::sidebar::{builtin_views, project_views, Sidebar};
use planner::ui::task_list::TaskList;
use planner::ui::task_object::TaskObject;
use planner::ui::task_row::TaskRow;
use planner::ui::{PlannerApplication, PlannerWindow};

/// Thursday, 30 July 2026.
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn now() -> chrono::DateTime<chrono::Utc> {
    today().and_hms_opt(12, 0, 0).unwrap().and_utc()
}

fn store() -> Store {
    let dir = tempfile::TempDir::new().expect("a temp dir");
    let (store, _) = Store::open_at(dir.keep().join("planner.json"));
    store
}

/// Run every widget case in one thread, reporting all failures rather than
/// stopping at the first.
struct Runner {
    failures: Vec<String>,
}

impl Runner {
    fn case(&mut self, name: &str, body: impl FnOnce() + std::panic::UnwindSafe) {
        // Widgets are not unwind-safe by Rust's reckoning, but a panic here
        // unwinds ordinary Rust frames — nothing crosses `extern "C"`, because
        // no case asserts from inside a signal handler.
        match std::panic::catch_unwind(body) {
            Ok(()) => println!("  ok   {name}"),
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panicked".to_string());
                println!("  FAIL {name}: {message}");
                self.failures.push(format!("{name}: {message}"));
            }
        }
    }
}

#[test]
fn widgets() {
    if gtk::init().is_err() {
        // No display: skip rather than fail, so `cargo test` is still useful
        // on a machine without one. CI runs `./test.sh --headless`.
        eprintln!("no display available; skipping widget tests");
        return;
    }
    adw::init().expect("libadwaita initialises once GTK has");

    let mut runner = Runner {
        failures: Vec::new(),
    };

    runner.case("a row shows the task it is bound to", || {
        let mut store = store();
        let mut task = Task::new(ProjectId::inbox(), "Email Sam", now());
        task.due = Some(Due::on(today()));
        task.priority = Priority::P1;
        let id = store.add_task(task);
        let task = store.task(&id).unwrap();

        let object = TaskObject::from_task(task, &store, today());
        let row = TaskRow::new();
        row.bind(&object);

        assert_eq!(object.content(), "Email Sam");
        assert_eq!(object.due_label(), "Today");
        assert_eq!(object.due_class(), "today");
        assert!(object.has_priority());
        assert_eq!(object.priority_class(), "priority-1");
        assert!(!row.has_css_class("completed"));
        row.unbind();
    });

    runner.case("a completed row is marked, and unbinding clears it", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Done thing", now()));
        store.complete_task(&id, now(), today());
        let task = store.task(&id).unwrap();

        let object = TaskObject::from_task(task, &store, today());
        let row = TaskRow::new();
        row.bind(&object);
        assert!(row.has_css_class("completed"), "a ticked row is struck out");

        row.unbind();
        assert!(
            !row.has_css_class("completed"),
            "a recycled row must not keep the last task's state"
        );
    });

    runner.case("an unbound row is no longer driven by its old task", || {
        let mut store = store();
        let first = store.add_task(Task::new(ProjectId::inbox(), "First", now()));
        let second = store.add_task(Task::new(ProjectId::inbox(), "Second", now()));

        let a = TaskObject::from_task(store.task(&first).unwrap(), &store, today());
        let b = TaskObject::from_task(store.task(&second).unwrap(), &store, today());

        let row = TaskRow::new();
        row.bind(&a);
        row.unbind();
        row.bind(&b);

        // Changing the *old* item must not touch the row now showing the new
        // one. This is the recycling bug, and it is silent when it happens.
        a.set_checked(true);
        assert!(
            !row.has_css_class("completed"),
            "the old task still drives the row"
        );
        assert_eq!(row.item().map(|item| item.id()), Some(b.id()));
        row.unbind();
    });

    runner.case("ticking a row reports it rather than acting on it", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Report me", now()));
        let object = TaskObject::from_task(store.task(&id).unwrap(), &store, today());

        let row = TaskRow::new();
        let reported: std::rc::Rc<std::cell::RefCell<Vec<(String, bool)>>> = Default::default();
        row.connect_closure("toggled", false, glib_closure(reported.clone()));

        row.bind(&object);
        // Binding writes the checkbox; that is not the user doing anything.
        assert!(
            reported.borrow().is_empty(),
            "binding must not report a toggle"
        );

        object.set_checked(true);
        assert!(
            reported.borrow().is_empty(),
            "a model change must not report a toggle either"
        );
        row.unbind();
    });

    runner.case("the list shows what the query matched", || {
        let mut store = store();
        let a = store.add_task(Task::new(ProjectId::inbox(), "Due today", now()));
        store.task_mut(&a).unwrap().due = Some(Due::on(today()));
        let b = store.add_task(Task::new(ProjectId::inbox(), "Due later", now()));
        store.task_mut(&b).unwrap().due = Some(Due::on(date(2026, 9, 1)));

        let list = TaskList::new();
        let query = Query::parse("due: today | overdue").unwrap();
        let matching = store.query(&query, today());
        list.set_tasks(&matching, &store, today());

        assert_eq!(list.len(), 1);
        assert!(!list.is_empty());
    });

    runner.case(
        "refreshing the list keeps the object for a surviving task",
        || {
            let mut store = store();
            let id = store.add_task(Task::new(ProjectId::inbox(), "Stays", now()));

            let list = TaskList::new();
            let all = Query::all();

            let matching = store.query(&all, today());
            list.set_tasks(&matching, &store, today());
            let first = list_item(&list, 0);

            // Rename it and refresh: the same object should still be there,
            // updated in place, so the row keeps its scroll position and state.
            store.task_mut(&id).unwrap().content = "Renamed".into();
            let matching = store.query(&all, today());
            list.set_tasks(&matching, &store, today());
            let second = list_item(&list, 0);

            assert_eq!(first.id(), second.id());
            assert_eq!(second.content(), "Renamed");
        },
    );

    runner.case("an empty list shows the empty state", || {
        let store = store();
        let list = TaskList::new();
        let matching = store.query(&Query::all(), today());
        list.set_tasks(&matching, &store, today());
        assert!(list.is_empty());
    });

    runner.case("a recurring task renders as repeating", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Bins", now()));
        store.task_mut(&id).unwrap().due =
            Some(Due::on(date(2026, 8, 3)).repeating(Recurrence::every(1, Unit::Week)));

        let object = TaskObject::from_task(store.task(&id).unwrap(), &store, today());
        assert!(object.recurring());
        assert_eq!(object.due_label(), "Mon");
    });

    runner.case("a task's labels and subtask count reach the row", || {
        let mut store = store();
        let label = store.label_for_name("errand");
        let parent = store.add_task(Task::new(ProjectId::inbox(), "Move house", now()));
        store.task_mut(&parent).unwrap().add_label(label);

        let child = store.add_task(Task::new(ProjectId::inbox(), "Pack", now()));
        store.task_mut(&child).unwrap().parent_id = Some(parent.clone());
        store.complete_task(&child, now(), today());

        let object = TaskObject::from_task(store.task(&parent).unwrap(), &store, today());
        assert!(object.has_labels());
        assert_eq!(object.labels().to_vec(), ["errand"]);
        assert!(object.has_subtasks());
        assert_eq!(object.subtasks(), "1 of 1");
    });

    runner.case("each label stays its own entry, commas and all", || {
        let mut store = store();
        let task = store.add_task(Task::new(ProjectId::inbox(), "Move house", now()));
        // The row draws one chip per entry. A name containing the separator
        // the projection used to join on is the case that has to survive.
        for name in ["errand", "home, garden"] {
            let label = store.label_for_name(name);
            store.task_mut(&task).unwrap().add_label(label);
        }

        let object = TaskObject::from_task(store.task(&task).unwrap(), &store, today());
        assert_eq!(object.labels().to_vec(), ["errand", "home, garden"]);
    });

    runner.case("the sidebar lists the built-ins then the projects", || {
        let mut store = store();
        let work = store.add_project(Project::new("Work", Color::Blue));
        let mut admin = Project::new("Admin", Color::Teal);
        admin.parent_id = Some(work.clone());
        store.add_project(admin);

        let sidebar = Sidebar::new();
        sidebar.refresh(&store, today());

        let projects = project_views(&store);
        let titles: Vec<&str> = projects.iter().map(|v| v.title.as_str()).collect();
        assert_eq!(titles, vec!["Work", "Admin"]);
        // A subproject is indented under its parent.
        assert_eq!(projects[0].depth, 0);
        assert_eq!(projects[1].depth, 1);

        // Today is selected when nothing else was.
        assert_eq!(
            sidebar.selected_view().map(|view| view.id),
            Some("today".to_string())
        );
    });

    runner.case(
        "every sidebar view's query runs against a real store",
        || {
            let mut store = store();
            store.add_project(Project::new("Work", Color::Blue));
            let id = store.add_task(Task::new(ProjectId::inbox(), "Something", now()));
            store.task_mut(&id).unwrap().due = Some(Due::on(today()));

            let mut views = builtin_views();
            views.extend(project_views(&store));
            for view in views {
                let query = Query::parse(&view.query)
                    .unwrap_or_else(|error| panic!("{}: {error}", view.title));
                // Just running it is the assertion: a term the evaluator does not
                // handle would panic here rather than in front of a user.
                let _ = store.query(&query, today());
            }
        },
    );

    runner.case(
        "Today shows overdue tasks and Completed shows only done",
        || {
            let mut store = store();
            let overdue = store.add_task(Task::new(ProjectId::inbox(), "Late", now()));
            store.task_mut(&overdue).unwrap().due = Some(Due::on(date(2026, 7, 1)));
            let done = store.add_task(Task::new(ProjectId::inbox(), "Finished", now()));
            store.complete_task(&done, now(), today());

            let views = builtin_views();
            let find = |id: &str| views.iter().find(|v| v.id == id).unwrap().query();

            let today_tasks = store.query(&find("today"), today());
            assert_eq!(today_tasks.len(), 1);
            assert_eq!(today_tasks[0].content, "Late");

            let completed = store.query(&find("completed"), today());
            assert_eq!(completed.len(), 1);
            assert_eq!(completed[0].content, "Finished");
        },
    );

    runner.case(
        "the dialog parses what is typed into the real entry",
        || {
            let mut store = store();
            store.add_project(Project::new("Work", Color::Blue));

            let dialog = QuickAddDialog::new();
            dialog.prepare(store.vocabulary(), today(), "Inbox");
            dialog.set_text("Email Sam #Work @email p1 friday 9am");

            let parsed = dialog.parsed();
            assert_eq!(parsed.title, "Email Sam");
            assert_eq!(parsed.project.as_deref(), Some("Work"));
            assert_eq!(parsed.labels, vec!["email"]);
            assert_eq!(parsed.priority, Some(Priority::P1));
            assert_eq!(parsed.due.as_ref().unwrap().date, date(2026, 7, 31));
            assert!(dialog.can_submit());
        },
    );

    runner.case("a line with no title cannot be submitted", || {
        let dialog = QuickAddDialog::new();
        dialog.prepare(Vocabulary::default(), today(), "Inbox");

        dialog.set_text("");
        assert!(!dialog.can_submit(), "nothing typed");

        dialog.set_text("p1 tomorrow @work");
        assert!(!dialog.can_submit(), "tokens but no task");

        dialog.set_text("Actually do something p1");
        assert!(dialog.can_submit());
    });

    runner.case("highlighting covers exactly the recognised tokens", || {
        let dialog = QuickAddDialog::new();
        dialog.prepare(Vocabulary::default(), today(), "Inbox");
        let text = "Email Sam p1 tomorrow";
        dialog.set_text(text);

        let parsed = dialog.parsed();
        let spans: Vec<&str> = parsed
            .spans
            .iter()
            .map(|span| &text[span.range.clone()])
            .collect();
        assert_eq!(spans, vec!["p1", "tomorrow"]);

        // And the attribute list built from them is non-empty and in range.
        let attributes = planner::ui::quick_add::highlight(&parsed);
        assert!(
            attributes.attributes().len() >= parsed.spans.len(),
            "every span contributes at least one attribute"
        );
        for attribute in attributes.attributes() {
            assert!(
                attribute.end_index() as usize <= text.len(),
                "an attribute runs past the end of the text"
            );
        }
    });

    runner.case(
        "a multi-word project name is recognised from the store",
        || {
            let mut store = store();
            store.add_project(Project::new("My Big Project", Color::Blue));

            let dialog = QuickAddDialog::new();
            dialog.prepare(store.vocabulary(), today(), "Inbox");
            dialog.set_text("Do the thing #My Big Project");

            assert_eq!(dialog.parsed().project.as_deref(), Some("My Big Project"));
            assert_eq!(dialog.parsed().title, "Do the thing");
        },
    );

    runner.case("keep-adding is off by default and can be turned on", || {
        let dialog = QuickAddDialog::new();
        dialog.prepare(Vocabulary::default(), today(), "Inbox");
        assert!(!dialog.keeps_adding());
        dialog.set_keeps_adding(true);
        assert!(dialog.keeps_adding());
    });

    runner.case(
        "a submitted line becomes the task the chips promised",
        || {
            let mut store = store();
            let work = store.add_project(Project::new("Work", Color::Blue));

            let dialog = QuickAddDialog::new();
            dialog.prepare(store.vocabulary(), today(), "Work");
            let line = "Email Sam #Work @email p2 friday 9am !30m";
            dialog.set_text(line);

            // What the window does with the line the dialog hands over.
            let parsed = planner::model::parse::parse_quick_add(line, today(), &store.vocabulary());
            let id = store.add_from_quick_add(&parsed, &ProjectId::inbox(), None, now());
            let task = store.task(&id).unwrap();

            assert_eq!(task.content, "Email Sam");
            assert_eq!(task.project_id, work);
            assert_eq!(task.priority, Priority::P2);
            assert_eq!(task.due.as_ref().unwrap().date, date(2026, 7, 31));
            assert_eq!(task.reminders.len(), 1);
            assert_eq!(store.label(&task.labels[0]).unwrap().name, "email");
        },
    );

    // --- detail panel ---------------------------------------------------

    runner.case("the panel shows everything about a task", || {
        let mut store = store();
        let label = store.label_for_name("errand");
        let id = store.add_task(Task::new(ProjectId::inbox(), "Move house", now()));
        {
            let task = store.task_mut(&id).unwrap();
            task.description = "Ring the agent first".into();
            task.due = Some(Due::at(
                date(2026, 8, 3),
                chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            ));
            task.deadline = Some(date(2026, 8, 10));
            task.priority = Priority::P2;
            task.pinned = true;
            task.add_label(label);
        }
        let child = store.add_task(Task::new(ProjectId::inbox(), "Pack", now()));
        store.task_mut(&child).unwrap().parent_id = Some(id.clone());

        let panel = DetailPanel::new();
        assert!(!panel.is_showing_task(), "nothing open to begin with");

        panel.show(&id, &store, today());

        assert!(panel.is_showing_task());
        assert_eq!(panel.title_text(), "Move house");
        assert_eq!(panel.description_text(), "Ring the agent first");
        assert_eq!(panel.schedule_text(), "Mon at 09:00");
        assert_eq!(panel.deadline_text(), "10 Aug");
        assert_eq!(panel.priority(), Priority::P2);
        assert!(panel.is_pinned());
        assert_eq!(panel.subtask_count(), 1);
        assert_eq!(panel.task_id(), Some(id));
    });

    runner.case("showing a task does not report edits back", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Quiet please", now()));
        store.task_mut(&id).unwrap().priority = Priority::P1;
        store.task_mut(&id).unwrap().pinned = true;

        let panel = DetailPanel::new();
        let edits: std::rc::Rc<std::cell::RefCell<Vec<Edit>>> = Default::default();
        let sink = edits.clone();
        panel.connect_edited(move |edit| sink.borrow_mut().push(edit.clone()));

        // Every widget here is written to programmatically. Without the
        // loading guard each of those writes would come straight back as an
        // edit, and the edit would trigger another show, and so on.
        panel.show(&id, &store, today());
        assert!(
            edits.borrow().is_empty(),
            "loading a task reported {:?}",
            edits.borrow()
        );
    });

    runner.case(
        "switching tasks does not leak the first one's edits",
        || {
            let mut store = store();
            let first = store.add_task(Task::new(ProjectId::inbox(), "First", now()));
            let second = store.add_task(Task::new(ProjectId::inbox(), "Second", now()));
            store.task_mut(&first).unwrap().priority = Priority::P1;

            let panel = DetailPanel::new();
            panel.show(&first, &store, today());

            let edits: std::rc::Rc<std::cell::RefCell<Vec<Edit>>> = Default::default();
            let sink = edits.clone();
            panel.connect_edited(move |edit| sink.borrow_mut().push(edit.clone()));

            panel.show(&second, &store, today());

            assert!(edits.borrow().is_empty(), "{:?}", edits.borrow());
            assert_eq!(panel.title_text(), "Second");
            assert_eq!(panel.priority(), Priority::P4, "the P1 did not carry over");
            assert_eq!(panel.task_id(), Some(second));
        },
    );

    runner.case("a task that has gone away clears the panel", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Doomed", now()));
        let panel = DetailPanel::new();
        panel.show(&id, &store, today());
        assert!(panel.is_showing_task());

        store.remove_task(&id);
        panel.show(&id, &store, today());

        assert!(!panel.is_showing_task());
        assert_eq!(panel.task_id(), None);
    });

    runner.case("a date row says No date rather than going blank", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Undated", now()));

        let panel = DetailPanel::new();
        panel.show(&id, &store, today());

        assert_eq!(panel.schedule_text(), "No date");
        assert_eq!(panel.deadline_text(), "No deadline");
    });

    runner.case("a repeating task's rule is on its schedule row", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Bins", now()));
        store.task_mut(&id).unwrap().due =
            Some(Due::on(date(2026, 8, 3)).repeating(Recurrence::every(1, Unit::Week)));

        let panel = DetailPanel::new();
        panel.show(&id, &store, today());
        assert_eq!(panel.schedule_text(), "Mon · every week");
    });

    runner.case("the panel re-reads a task after it changes", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Before", now()));

        let panel = DetailPanel::new();
        panel.show(&id, &store, today());
        assert_eq!(panel.title_text(), "Before");

        store.task_mut(&id).unwrap().content = "After".into();
        store.task_mut(&id).unwrap().priority = Priority::P3;
        panel.show(&id, &store, today());

        assert_eq!(panel.title_text(), "After");
        assert_eq!(panel.priority(), Priority::P3);
    });

    runner.case(
        "a subtask row appears and disappears with the subtask",
        || {
            let mut store = store();
            let parent = store.add_task(Task::new(ProjectId::inbox(), "Parent", now()));
            let panel = DetailPanel::new();
            panel.show(&parent, &store, today());
            assert_eq!(panel.subtask_count(), 0);

            let child = store.add_task(Task::new(ProjectId::inbox(), "Child", now()));
            store.task_mut(&child).unwrap().parent_id = Some(parent.clone());
            panel.show(&parent, &store, today());
            assert_eq!(panel.subtask_count(), 1);

            store.remove_task(&child);
            panel.show(&parent, &store, today());
            assert_eq!(
                panel.subtask_count(),
                0,
                "the removed subtask's row is still there"
            );
        },
    );

    runner.case(
        "the date picker carries a repeat rule through a new date",
        || {
            // Moving a repeating task to another day must not quietly stop it
            // repeating — the picker keeps the rule it was loaded with.
            let picker = planner::ui::date_picker::DatePicker::new();
            let due = Due::on(date(2026, 8, 3)).repeating(Recurrence::every(1, Unit::Week));

            let chosen: std::rc::Rc<std::cell::RefCell<Option<Option<Due>>>> = Default::default();
            let sink = chosen.clone();
            picker.connect_chosen(move |due| *sink.borrow_mut() = Some(due));

            picker.load(Some(&due), today());
            picker.choose(today());

            let reported = chosen.borrow().clone().expect("the picker reported");
            let reported = reported.expect("a date, not a clear");
            assert_eq!(reported.date, today());
            assert!(
                reported.is_recurring(),
                "the repeat rule was dropped on the way through"
            );
        },
    );

    runner.case("the repeat box opens showing the rule the task has", || {
        // Prefilled rather than blank: the phrase `describe` writes is the
        // phrase that would have produced the rule, so editing the box is
        // editing the rule rather than retyping it from scratch.
        let picker = planner::ui::date_picker::DatePicker::new();
        let due = Due::on(date(2026, 8, 3)).repeating(Recurrence::every_weekday());
        picker.load(Some(&due), today());
        assert_eq!(picker.repeat_text(), "every weekday");

        picker.load(Some(&Due::on(date(2026, 8, 3))), today());
        assert_eq!(picker.repeat_text(), "", "a task that does not repeat");
    });

    runner.case(
        "a repeat typed into the picker keeps the task's date",
        || {
            let picker = planner::ui::date_picker::DatePicker::new();
            let chosen: std::rc::Rc<std::cell::RefCell<Option<Option<Due>>>> = Default::default();
            let sink = chosen.clone();
            picker.connect_chosen(move |due| *sink.borrow_mut() = Some(due));

            picker.load(Some(&Due::on(date(2026, 8, 3))), today());
            picker.commit_repeat_text("every! 10 days");

            let reported = chosen.borrow().clone().expect("the picker reported");
            let reported = reported.expect("a date, not a clear");
            assert_eq!(reported.date, date(2026, 8, 3), "the date moved");
            let rule = reported.recurrence.expect("a rule");
            assert!(rule.from_completion, "the bang was lost");
            assert_eq!(rule.interval, 10);
        },
    );

    runner.case(
        "a repeat on a task with no date lands on its first occurrence",
        || {
            // A rule lives inside a `Due`, so it needs a date to hang on.
            // "Every monday" on a Thursday means the coming Monday.
            let picker = planner::ui::date_picker::DatePicker::new();
            let chosen: std::rc::Rc<std::cell::RefCell<Option<Option<Due>>>> = Default::default();
            let sink = chosen.clone();
            picker.connect_chosen(move |due| *sink.borrow_mut() = Some(due));

            picker.load(None, today());
            picker.commit_repeat_text("every monday");

            let reported = chosen.borrow().clone().expect("the picker reported");
            let reported = reported.expect("a date, not a clear");
            assert_eq!(reported.date, date(2026, 8, 3), "not the coming Monday");
        },
    );

    runner.case(
        "emptying the repeat box stops the repeat, not the date",
        || {
            let picker = planner::ui::date_picker::DatePicker::new();
            let chosen: std::rc::Rc<std::cell::RefCell<Option<Option<Due>>>> = Default::default();
            let sink = chosen.clone();
            picker.connect_chosen(move |due| *sink.borrow_mut() = Some(due));

            let due = Due::on(date(2026, 8, 3)).repeating(Recurrence::every(1, Unit::Week));
            picker.load(Some(&due), today());
            picker.commit_repeat_text("");

            let reported = chosen.borrow().clone().expect("the picker reported");
            let reported = reported.expect("the date went too");
            assert_eq!(reported.date, date(2026, 8, 3));
            assert!(!reported.is_recurring(), "still repeating");
        },
    );

    runner.case(
        "a repeat phrase that will not parse changes nothing",
        || {
            // The failure mode this guards: committing nonsense as "no rule",
            // which would silently stop a task repeating because of a typo.
            let picker = planner::ui::date_picker::DatePicker::new();
            let chosen: std::rc::Rc<std::cell::RefCell<Option<Option<Due>>>> = Default::default();
            let sink = chosen.clone();
            picker.connect_chosen(move |due| *sink.borrow_mut() = Some(due));

            let due = Due::on(date(2026, 8, 3)).repeating(Recurrence::every(1, Unit::Week));
            picker.load(Some(&due), today());
            picker.commit_repeat_text("every fortnight");

            assert_eq!(chosen.borrow().clone(), None, "nonsense was committed");
        },
    );

    runner.case("a deadline has no repeat box to type into", || {
        let picker = planner::ui::date_picker::DatePicker::new();
        picker.hide_repeat();
        let chosen: std::rc::Rc<std::cell::RefCell<Option<Option<Due>>>> = Default::default();
        let sink = chosen.clone();
        picker.connect_chosen(move |due| *sink.borrow_mut() = Some(due));

        picker.load(Some(&Due::on(date(2026, 8, 3))), today());
        picker.commit_repeat_text("every week");

        assert_eq!(chosen.borrow().clone(), None, "a deadline took a repeat");
    });

    runner.case("clearing a date reports no date at all", || {
        let picker = planner::ui::date_picker::DatePicker::new();
        let chosen: std::rc::Rc<std::cell::RefCell<Option<Option<Due>>>> = Default::default();
        let sink = chosen.clone();
        picker.connect_chosen(move |due| *sink.borrow_mut() = Some(due));

        picker.load(Some(&Due::on(date(2026, 8, 3))), today());
        picker.clear_date();

        assert_eq!(chosen.borrow().clone(), Some(None));
    });

    // --- sections, board, dragging --------------------------------------

    runner.case("a project with no sections is one unsectioned lane", || {
        let mut store = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        store.add_task(Task::new(project.clone(), "Something", now()));

        let view = ProjectView::new();
        view.show_project(&project, ViewStyle::List, &store, today());

        assert_eq!(view.lane_count(), 1);
        assert_eq!(view.lane(None).map(|list| list.len()), Some(1));
    });

    runner.case("every section gets a lane, empty ones included", || {
        let mut store = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let doing = store.add_section(Section::new(project.clone(), "Doing"));
        store.add_section(Section::new(project.clone(), "Done"));

        let view = ProjectView::new();
        view.show_project(&project, ViewStyle::Board, &store, today());

        // Unsectioned, Doing, Done — and the empty ones are the point: a
        // section you cannot see is a section you cannot drag into.
        assert_eq!(view.lane_count(), 3);
        assert_eq!(view.lane(Some(&doing)).map(|list| list.len()), Some(0));
        assert!(view.lane(Some(&doing)).is_some_and(|list| list.is_empty()));
    });

    runner.case("tasks land in the lane for their section", || {
        let mut store = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let doing = store.add_section(Section::new(project.clone(), "Doing"));

        store.add_task(Task::new(project.clone(), "Loose", now()));
        let sorted = store.add_task(Task::new(project.clone(), "In progress", now()));
        store.task_mut(&sorted).unwrap().section_id = Some(doing.clone());

        let view = ProjectView::new();
        view.show_project(&project, ViewStyle::List, &store, today());

        assert_eq!(view.lane(None).map(|list| list.len()), Some(1));
        assert_eq!(view.lane(Some(&doing)).map(|list| list.len()), Some(1));
        assert_eq!(
            view.lane(Some(&doing))
                .and_then(|list| list.item(0))
                .map(|item| item.content()),
            Some("In progress".to_string())
        );
    });

    runner.case(
        "each lane knows which list it is, so a drop has a home",
        || {
            let mut store = store();
            let project = store.add_project(Project::new("Work", Color::Blue));
            let doing = store.add_section(Section::new(project.clone(), "Doing"));

            let view = ProjectView::new();
            view.show_project(&project, ViewStyle::List, &store, today());

            assert_eq!(
                view.lane(None).and_then(|list| list.group()),
                Some((project.clone(), None))
            );
            assert_eq!(
                view.lane(Some(&doing)).and_then(|list| list.group()),
                Some((project, Some(doing)))
            );
        },
    );

    runner.case(
        "a filter list refuses drops because tasks do not live there",
        || {
            // Today is a view onto tasks, not a place they are kept. Dropping one
            // "into Today" has no meaning, and a drop target that accepted it
            // would have to invent one.
            let list = TaskList::new();
            assert_eq!(list.group(), None);
        },
    );

    runner.case("switching to the board keeps the same lanes", || {
        let mut store = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        store.add_section(Section::new(project.clone(), "Doing"));

        let view = ProjectView::new();
        view.show_project(&project, ViewStyle::List, &store, today());
        assert_eq!(view.style(), ViewStyle::List);
        let lanes = view.lane_count();

        view.show_project(&project, ViewStyle::Board, &store, today());
        assert_eq!(view.style(), ViewStyle::Board);
        assert_eq!(
            view.lane_count(),
            lanes,
            "the board is a re-layout, not a rebuild"
        );
    });

    runner.case(
        "adding a section adds a lane without losing the others",
        || {
            let mut store = store();
            let project = store.add_project(Project::new("Work", Color::Blue));
            store.add_task(Task::new(project.clone(), "Loose", now()));

            let view = ProjectView::new();
            view.show_project(&project, ViewStyle::List, &store, today());
            assert_eq!(view.lane_count(), 1);

            store.add_section(Section::new(project.clone(), "Doing"));
            view.show_project(&project, ViewStyle::List, &store, today());

            assert_eq!(view.lane_count(), 2);
            assert_eq!(view.lane(None).map(|list| list.len()), Some(1));
        },
    );

    runner.case(
        "a section header offers rename and delete, and the unsectioned lane does not",
        || {
            let mut store = store();
            let project = store.add_project(Project::new("Work", Color::Blue));
            let doing = store.add_section(Section::new(project.clone(), "Doing"));

            let view = ProjectView::new();
            view.show_project(&project, ViewStyle::List, &store, today());

            let actions = view
                .lane(Some(&doing))
                .expect("the section lane")
                .header_actions();
            assert_eq!(actions, vec!["win.rename-section", "win.delete-section"]);
            assert!(
                view.lane(None)
                    .expect("the unsectioned lane")
                    .header_actions()
                    .is_empty(),
                "the unsectioned lane is not a section, so there is nothing to rename or delete"
            );
        },
    );

    runner.case("removing a section leaves its tasks in the project", || {
        let mut store = store();
        let project = store.add_project(Project::new("Work", Color::Blue));
        let doing = store.add_section(Section::new(project.clone(), "Doing"));
        let id = store.add_task(Task::new(project.clone(), "In progress", now()));
        store.task_mut(&id).unwrap().section_id = Some(doing.clone());

        store.remove_section(&doing, now());
        let view = ProjectView::new();
        view.show_project(&project, ViewStyle::List, &store, today());

        assert_eq!(view.lane_count(), 1);
        assert_eq!(
            view.lane(None).map(|list| list.len()),
            Some(1),
            "the task fell back into the project rather than vanishing"
        );
    });

    // --- selection, search, filters -------------------------------------

    runner.case(
        "selection mode selects nothing until you pick something",
        || {
            let mut store = store();
            for name in ["One", "Two", "Three"] {
                store.add_task(Task::new(ProjectId::inbox(), name, now()));
            }
            let list = TaskList::new();
            let matching = store.query(&Query::all(), today());
            list.set_tasks(&matching, &store, today());

            assert!(!list.is_selecting());
            assert!(list.selected().is_empty());

            list.set_selecting(true);
            assert!(list.is_selecting());
            assert!(
                list.selected().is_empty(),
                "turning the mode on must not select the list"
            );
        },
    );

    runner.case("leaving selection mode forgets what was selected", || {
        let mut store = store();
        store.add_task(Task::new(ProjectId::inbox(), "One", now()));
        let list = TaskList::new();
        let matching = store.query(&Query::all(), today());
        list.set_tasks(&matching, &store, today());

        list.set_selecting(true);
        list.set_selecting(false);
        assert!(
            list.selected().is_empty(),
            "a stale selection would let a bulk action hit tasks that are              no longer on screen"
        );
    });

    runner.case(
        "every lane joins selection mode, including new ones",
        || {
            let mut store = store();
            let project = store.add_project(Project::new("Work", Color::Blue));
            store.add_task(Task::new(project.clone(), "Loose", now()));

            let view = ProjectView::new();
            view.show_project(&project, ViewStyle::List, &store, today());
            view.set_selecting(true);

            assert!(view.lane(None).is_some_and(|list| list.is_selecting()));

            // A section added while selecting must not be the one column you
            // cannot select anything in.
            let doing = store.add_section(Section::new(project.clone(), "Doing"));
            view.show_project(&project, ViewStyle::List, &store, today());

            assert!(view
                .lane(Some(&doing))
                .is_some_and(|list| list.is_selecting()));
        },
    );

    runner.case("quick find offers what the store holds", || {
        let mut store = store();
        store.add_project(Project::new("Plumbing", Color::Blue));
        store.add_task(Task::new(ProjectId::inbox(), "Call the plumber", now()));

        let dialog = QuickFindDialog::new();
        dialog.set_query("plumb");
        dialog.refresh(&store);

        let titles: Vec<String> = dialog
            .hits()
            .iter()
            .map(|hit| hit.title().to_string())
            .collect();
        assert!(titles.contains(&"Plumbing".to_string()));
        assert!(titles.contains(&"Call the plumber".to_string()));
    });

    runner.case("quick find offers nothing for an empty query", || {
        let mut store = store();
        store.add_task(Task::new(ProjectId::inbox(), "Anything", now()));

        let dialog = QuickFindDialog::new();
        dialog.set_query("");
        dialog.refresh(&store);
        assert!(dialog.hits().is_empty());
    });

    runner.case("a saved filter appears in the sidebar and runs", || {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Urgent thing", now()));
        store.task_mut(&id).unwrap().priority = Priority::P1;
        store.add_task(Task::new(ProjectId::inbox(), "Ordinary thing", now()));
        store.put_filter(SavedFilter::new("Urgent", "p1", Color::Red));

        let sidebar = Sidebar::new();
        sidebar.refresh(&store, today());

        let filters = planner::ui::sidebar::filter_views(&store);
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].title, "Urgent");

        let matching = store.query(&filters[0].query(), today());
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].content, "Urgent thing");
    });

    runner.case(
        "a broken saved filter shows nothing rather than everything",
        || {
            // The user wrote this one, so it can be broken at any time. The worst
            // outcome would be falling back to "match everything" and having a
            // sidebar entry that silently claims every task is urgent.
            let mut store = store();
            store.add_task(Task::new(ProjectId::inbox(), "A task", now()));
            store.put_filter(SavedFilter::new("Broken", "p1 & (", Color::Red));

            let filters = planner::ui::sidebar::filter_views(&store);
            assert_eq!(filters.len(), 1);
            assert!(
                store.query(&filters[0].query(), today()).is_empty(),
                "a filter that will not parse must match nothing"
            );
        },
    );

    runner.case(
        "selecting a sidebar entry by id moves the selection",
        || {
            let mut store = store();
            let project = store.add_project(Project::new("Work", Color::Blue));

            let sidebar = Sidebar::new();
            sidebar.refresh(&store, today());
            assert_eq!(
                sidebar.selected_view().map(|view| view.id),
                Some("today".to_string())
            );

            sidebar.select(&format!("project:{project}"));
            assert_eq!(
                sidebar.selected_view().map(|view| view.id),
                Some(format!("project:{project}"))
            );
        },
    );

    runner.case(
        "a new project appears in the sidebar and can be selected",
        || {
            let mut store = store();
            let sidebar = Sidebar::new();
            sidebar.refresh(&store, today());
            assert!(
                planner::ui::sidebar::project_views(&store).is_empty(),
                "a fresh store has only the Inbox"
            );

            let id = store.add_project(Project::new("Work", Color::Blue));
            sidebar.refresh(&store, today());

            let projects = planner::ui::sidebar::project_views(&store);
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].title, "Work");

            sidebar.select(&format!("project:{id}"));
            assert_eq!(
                sidebar.selected_view().and_then(|view| view.project_id()),
                Some(id)
            );
        },
    );

    runner.case("every icon a view asks for exists in the theme", || {
        // A missing icon name is not an error at runtime — GTK draws a
        // broken-image glyph and carries on — so nothing else would catch it.
        let mut store = store();
        store.add_project(Project::new("Work", Color::Blue));
        store.put_filter(SavedFilter::new("Urgent", "p1", Color::Red));

        let theme = gtk::IconTheme::for_display(&gtk::gdk::Display::default().expect("a display"));
        let mut views = builtin_views();
        views.extend(planner::ui::sidebar::filter_views(&store));
        views.extend(planner::ui::sidebar::project_views(&store));

        for view in views {
            assert!(
                theme.has_icon(view.icon),
                "{} asks for a missing icon: {}",
                view.title,
                view.icon
            );
        }
    });

    runner.case(
        "a new window is filled in before anything is edited",
        || {
            // The window opened empty and only filled in once some later edit
            // called `refresh` again: it refreshed from `constructed`, where
            // `GtkWindow:application` is not set yet, so it found no store and
            // did nothing. Anything that reads the application has to run after
            // construction returns.
            let dir = tempfile::TempDir::new().expect("a temp dir");
            // Redirect the store off the real one. `PlannerApplication::startup`
            // opens the default path, and a test must not open the file the
            // developer keeps their own tasks in.
            std::env::set_var("XDG_DATA_HOME", dir.path());

            let seeded = dir.path().join("planner").join("planner.json");
            std::fs::create_dir_all(seeded.parent().expect("a parent")).expect("the data dir");
            let (mut store, _) = Store::open_at(&seeded);
            store.add_project(Project::new("Work", Color::Blue));
            store.save().expect("seeding the store");

            // Its own ID: the real one would make this process a remote for a
            // running Planner and drive the live app instead of itself.
            let app = PlannerApplication::with_application_id("us.hagreli.Planner.WindowTest");
            app.register(gtk::gio::Cancellable::NONE)
                .expect("registering emits startup, which loads the store");

            let window = PlannerWindow::new(&app);
            assert_eq!(
                window.selected_view_id().as_deref(),
                Some("today"),
                "a freshly built window shows nothing until something is edited"
            );
        },
    );

    runner.case(
        "an agent command edits the running application's own store",
        || {
            // The model tests drive `agent::execute` against a store directly.
            // What only shows up here is the wiring around it: that the command
            // reaches the store this process is holding, that the change is
            // written out rather than left waiting for the save tick, and that
            // the borrow taken to run it is released before the redraw that
            // follows — a redraw reads the store, and re-entering a live
            // `borrow_mut` aborts the process rather than failing a test.
            let dir = tempfile::TempDir::new().expect("a temp dir");
            std::env::set_var("XDG_DATA_HOME", dir.path());

            let app = PlannerApplication::with_application_id("us.hagreli.Planner.AgentTest");
            app.register(gtk::gio::Cancellable::NONE)
                .expect("registering emits startup, which loads the store");

            // A window, so the redraw after a change has something to run
            // against rather than being skipped.
            let window = PlannerWindow::new(&app);
            window.refresh();

            let arguments =
                |line: &str| -> Vec<String> { line.split(' ').map(str::to_string).collect() };

            let (output, ok) = app.agent_command(&arguments("add Ring the plumber p1 today"));
            assert!(ok, "the command failed: {output}");
            assert!(
                app.with_store(|store| store
                    .tasks()
                    .iter()
                    .any(|task| task.content == "Ring the plumber")),
                "the running application's own store did not get the task"
            );

            // On disk already, not two seconds from now.
            let written = std::fs::read_to_string(dir.path().join("planner").join("planner.json"))
                .expect("the store was saved");
            assert!(
                written.contains("Ring the plumber"),
                "an agent command must not leave its change unsaved"
            );

            // And a failure is reported without touching anything.
            let (output, ok) = app.agent_command(&arguments("complete Nothing like this"));
            assert!(!ok, "an impossible command should not report success");
            assert!(output.contains("not-found"), "{output}");
        },
    );

    assert!(
        runner.failures.is_empty(),
        "{} widget case(s) failed:\n{}",
        runner.failures.len(),
        runner.failures.join("\n")
    );
}

fn list_item(list: &TaskList, index: u32) -> TaskObject {
    // Reaching through the list's own model keeps the test honest about what
    // the view is actually showing.
    list.item(index).expect("an item at this index")
}

/// A `toggled` handler that records what it was told.
fn glib_closure(
    sink: std::rc::Rc<std::cell::RefCell<Vec<(String, bool)>>>,
) -> glib::closure::RustClosure {
    glib::closure_local!(move |_: TaskRow, id: &str, checked: bool| {
        sink.borrow_mut().push((id.to_string(), checked));
    })
}

//! Render the real widget tree to a PNG.
//!
//! Screenshotting a live GNOME Wayland session needs interactive consent,
//! which makes "does this look right?" hard to answer while iterating. This
//! builds the actual widgets against a seeded store and paints them offscreen
//! instead, so a design change can be looked at in one command.
//!
//! ```sh
//! cargo run --example preview -- /tmp/preview
//! cargo run --example preview -- /tmp/preview dark
//! ```

use adw::prelude::*;
use gtk::glib;

use chrono::NaiveDate;
use planner::model::color::Color;
use planner::model::id::ProjectId;
use planner::model::parse::parse_quick_add;
use planner::model::project::{Project, Section};
use planner::model::query::Query;
use planner::model::store::Store;
use planner::model::ViewStyle;
use planner::ui::detail_panel::DetailPanel;
use planner::ui::project_view::ProjectView;
use planner::ui::quick_add::QuickAddDialog;
use planner::ui::sidebar::{builtin_views, Sidebar};
use planner::ui::task_list::TaskList;
use planner::ui::window::PlannerWindow;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "/tmp/preview".to_string());
    let dark = args.next().is_some_and(|scheme| scheme == "dark");

    gtk::init().expect("a display — run under xvfb-run if there is none");
    adw::init().expect("libadwaita");

    // An animating widget is a widget that is not finished being laid out.
    // Turning animations off makes a snapshot deterministic rather than a
    // race against a transition.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_enable_animations(false);
    }

    adw::StyleManager::default().set_color_scheme(if dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });
    if let Some(display) = gtk::gdk::Display::default() {
        planner::ui::load_stylesheet(&display);
    }

    let store = seeded();
    let today = today();
    std::fs::create_dir_all(&out).expect("output directory");

    // The task list, showing Today.
    let list = TaskList::new();
    let query = builtin_views()
        .into_iter()
        .find(|view| view.id == "today")
        .expect("a Today view")
        .query();
    let matching = store.query(&query, today);
    list.set_tasks(&matching, &store, today);
    render(
        &list,
        520,
        420,
        &format!("{out}/tasks-{}.png", scheme(dark)),
    );

    // Everything, so the empty state is not the only thing on show.
    let all = TaskList::new();
    let matching = store.query(&Query::all(), today);
    all.set_tasks(&matching, &store, today);
    render(&all, 520, 520, &format!("{out}/all-{}.png", scheme(dark)));

    // The sidebar.
    let sidebar = Sidebar::new();
    sidebar.refresh(&store, today);
    render(
        &sidebar,
        260,
        420,
        &format!("{out}/sidebar-{}.png", scheme(dark)),
    );

    // Quick add, mid-sentence, so the highlighting and chips are on show.
    let dialog = QuickAddDialog::new();
    dialog.prepare(store.vocabulary(), today, "Inbox");
    dialog.set_text("Email Sam about the lease #Work @email p1 friday 9am !30m");
    render_dialog(
        &dialog,
        480,
        320,
        &format!("{out}/quick-add-{}.png", scheme(dark)),
    );

    // Quick add again, typing over something that already exists, so the
    // duplicate check has something to say. Both states are on show: a
    // near-identical title the local pass caught on its own, and a synonym only
    // the model could have.
    {
        let seeded = seed_for_duplicates(today);
        let dialog = QuickAddDialog::new();
        dialog.prepare(seeded.vocabulary(), today, "Inbox");
        dialog.set_candidate_source(move |title: &str| {
            planner::model::similar::candidates(
                &seeded,
                title,
                None,
                planner::model::duplicate::MAX_CANDIDATES,
                planner::model::similar::RECALL_FLOOR,
            )
        });
        dialog.set_text("Ring the plumber about the boiler");
        dialog.apply_judgements(planner::model::duplicate::Judgements {
            duplicates: vec![planner::model::duplicate::Judgement {
                id: "dup-plumber".into(),
                verdict: planner::model::duplicate::Verdict::Same,
                reason: "ringing and calling the plumber are the same".into(),
            }],
        });
        render_dialog(
            &dialog,
            480,
            420,
            &format!("{out}/quick-add-duplicate-{}.png", scheme(dark)),
        );
    }

    // The detail panel, on the task with the most going on.
    let busiest = store
        .tasks()
        .iter()
        .max_by_key(|task| {
            usize::from(task.due.is_some())
                + usize::from(task.deadline.is_some())
                + task.labels.len()
                + store.subtasks(&task.id).len()
        })
        .map(|task| task.id.clone())
        .expect("a seeded store is not empty");
    let panel = DetailPanel::new();
    panel.show(&busiest, &store, today);
    render(
        &panel,
        380,
        700,
        &format!("{out}/detail-{}.png", scheme(dark)),
    );

    // A project, both ways round.
    let work_project = store
        .project_by_name("Work")
        .map(|project| project.id.clone())
        .expect("the seeded Work project");

    let list_view = ProjectView::new();
    list_view.show_project(&work_project, ViewStyle::List, &store, today);
    render(
        &list_view,
        520,
        520,
        &format!("{out}/project-list-{}.png", scheme(dark)),
    );

    let board = ProjectView::new();
    board.show_project(&work_project, ViewStyle::Board, &store, today);
    render(
        &board,
        980,
        460,
        &format!("{out}/project-board-{}.png", scheme(dark)),
    );

    // The sync dialog, with a server that is working.
    let rows: Vec<(String, String)> = [
        ("Server", "http://nas:8083"),
        ("Records here", "24 records"),
        ("Synced", "All 24"),
        ("Last pass", "2 minutes ago"),
        ("File", "/home/you/.local/share/planner/planner.json"),
    ]
    .into_iter()
    .map(|(title, value)| (title.to_string(), value.to_string()))
    .collect();
    let sync = PlannerWindow::sync_status_content(
        &rows,
        "A pass runs when an edit settles, and the server holds a request open so a change \
         made elsewhere arrives as it happens.",
    );
    render(&sync, 460, 400, &format!("{out}/sync-{}.png", scheme(dark)));

    println!("wrote {out}/*-{}.png", scheme(dark));
}

/// Render a dialog by painting its content, not by presenting it.
///
/// `present` puts an `AdwDialog` into an animated overlay driven by the
/// window's frame clock, and a frame clock does not advance under a main loop
/// that is only being drained. Detaching the content and painting that is the
/// same widget tree with none of the choreography.
fn render_dialog(dialog: &QuickAddDialog, width: i32, height: i32, path: &str) {
    let Some(content) = dialog.child() else {
        eprintln!("{path}: the dialog has no content");
        return;
    };
    dialog.set_child(gtk::Widget::NONE);
    render(&content, width, height, path);
}

fn scheme(dark: bool) -> &'static str {
    if dark {
        "dark"
    } else {
        "light"
    }
}

/// Paint a widget offscreen and write it out.
fn render(widget: &impl IsA<gtk::Widget>, width: i32, height: i32, path: &str) {
    let window = gtk::Window::builder()
        .default_width(width)
        .default_height(height)
        .child(widget)
        .build();
    // No titlebar: these are pictures of a widget, and a window decoration
    // around one that already has a header bar just reads as a mistake.
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.present();

    settle();

    snapshot(&window, width, height, path);
    window.destroy();
}

/// Run the main loop until there is nothing left to lay out.
///
/// One drain is not enough: presenting a widget schedules work that schedules
/// more, so this pumps until it stops finding any, with a bound so a
/// misbehaving widget cannot hang the run.
fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..100 {
        let mut worked = false;
        while context.iteration(false) {
            worked = true;
        }
        if !worked {
            break;
        }
    }
}

/// Paint a realised window into a PNG.
fn snapshot(window: &impl IsA<gtk::Widget>, width: i32, height: i32, path: &str) {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width as f64, height as f64);

    let Some(node) = snapshot.to_node() else {
        eprintln!("{path}: nothing was drawn");
        return;
    };
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize(gtk::gdk::Surface::NONE)
        .expect("a renderer");
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).expect("write the png");
    renderer.unrealize();
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// A store with enough in it to show every part of a row.
/// A store holding exactly the two tasks the duplicate preview needs: one the
/// word comparison catches on its own, and one only a model could.
fn seed_for_duplicates(today: NaiveDate) -> Store {
    let dir = std::env::temp_dir().join("planner-preview-duplicates");
    let _ = std::fs::remove_dir_all(&dir);
    let (mut store, _) = Store::open_at(dir.join("planner.json"));
    let now = today.and_hms_opt(9, 0, 0).unwrap().and_utc();

    let home = store.add_project(Project::new("Home", Color::Green), now);

    // Fixed ids so the preview can hand the dialog a verdict about one of them.
    let mut plumber =
        planner::model::task::Task::new(home.clone(), "Call the plumber about the boiler", now);
    plumber.id = planner::model::TaskId::from_raw("dup-plumber");
    store.add_task(plumber);

    let mut boiler = planner::model::task::Task::new(home, "Ring the boiler service line", now);
    boiler.id = planner::model::TaskId::from_raw("dup-boiler");
    store.add_task(boiler);

    store
}

fn seeded() -> Store {
    let dir = std::env::temp_dir().join("planner-preview");
    let _ = std::fs::remove_dir_all(&dir);
    let (mut store, _) = Store::open_at(dir.join("planner.json"));
    let today = today();
    let now = chrono::Utc::now();

    let work = store.add_project(Project::new("Work", Color::Blue), now);
    let mut admin = Project::new("Admin", Color::Teal);
    admin.parent_id = Some(work.clone());
    store.add_project(admin, now);
    store.add_project(Project::new("Home", Color::Green), now);

    for line in [
        "Email Sam about the lease @email p1 today 9am",
        "Renew the parking permit p2 today",
        "Water the plants every! 10 days",
        "Book the dentist @phone",
        "Pay the electricity bill p3 yesterday",
        "Weekly review every monday",
    ] {
        add(&mut store, line, today, now);
    }

    // One overdue with a deadline, and one with subtasks.
    let late = add(&mut store, "File the tax return p1 yesterday", today, now);
    store.task_mut(&late).unwrap().deadline = today.checked_sub_days(chrono::Days::new(2));

    let parent = add(
        &mut store,
        "Move house @errand @home p2 today 09:00",
        today,
        now,
    );
    store.task_mut(&parent).unwrap().description =
        "Ring the agent before Friday.\nConfirm the van booking.".into();
    store.task_mut(&parent).unwrap().deadline = today.checked_add_days(chrono::Days::new(9));
    for child in ["Pack the kitchen", "Book a van"] {
        let id = add(&mut store, child, today, now);
        store.task_mut(&id).unwrap().parent_id = Some(parent.clone());
    }
    let packed = store.subtasks(&parent)[0].id.clone();
    store.complete_task(&packed, now, today);

    let done = add(&mut store, "Cancel the old broadband", today, now);
    store.complete_task(&done, now, today);

    // A project with sections, so the board has columns worth looking at.
    let doing = store.add_section(Section::new(work.clone(), "In progress"), now);
    let blocked = store.add_section(Section::new(work.clone(), "Blocked"), now);
    for (line, section) in [
        ("Draft the Q3 report p1 tomorrow", Some(&doing)),
        ("Review the contract @legal", Some(&doing)),
        ("Chase the supplier p2", Some(&blocked)),
        ("Tidy the shared drive", None),
    ] {
        let id = add(&mut store, line, today, now);
        let task = store.task_mut(&id).unwrap();
        task.project_id = work.clone();
        task.section_id = section.cloned();
    }

    store
}

fn add(
    store: &mut Store,
    line: &str,
    today: NaiveDate,
    now: chrono::DateTime<chrono::Utc>,
) -> planner::model::TaskId {
    let parsed = parse_quick_add(line, today, &store.vocabulary());
    store.add_from_quick_add(&parsed, &ProjectId::inbox(), None, now)
}

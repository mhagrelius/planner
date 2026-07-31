# Planner — design for review

A GTK 4 / libadwaita task planner for GNOME, in Rust. Local-first, single user.
Built the way Stickies is built: a GTK-free `model/` half that `cargo test`
exercises with no display, an imperative `ui/` half of `glib::wrapper!`
subclasses, no blueprint, no `.ui` XML, no meson, no async runtime.

## Scope

**v1 is local-only.** No Todoist account, no CalDAV, no collaboration. Every
model field that sync would need (stable UUID, `updated_at`, tombstones) exists
from day one so a sync source can be added later without a migration, but none
of that code gets written now.

## What it does

### Structure

Project → Section → Task → Subtask. Projects nest. Subtasks nest. Every task
lives in exactly one project; the Inbox is a project with a reserved id.

### Task

| Field | Notes |
|---|---|
| `content` | title, Markdown inline |
| `description` | Markdown, reuses the Stickies live-preview parser |
| `due` | date, optional time, optional recurrence |
| `deadline` | separate from `due` — "work on it Tuesday, it's due Friday" |
| `priority` | P1–P4, P4 = none, Todoist-compatible colours |
| `labels` | many, flat, coloured |
| `reminders` | many; absolute or relative-to-due |
| `pinned` | drives the Pinboard view |
| `checked`, `completed_at`, `added_at`, `updated_at` | |

No file attachments in v1 (Flatpak portal work for little return). Links in the
description cover the common case.

### Views

Inbox, Today, Upcoming, Labels, Pinboard, Completed, and a per-project view with
a list/board toggle. **All of them are the same widget over a different query**
— see below. Board mode is one column per section, drag between columns.

### The filter query language

This is the one place I'd deliberately go past Planify, which hardcodes each
filter view. A small parser in `model/query.rs` over

```
due:today | overdue          #Work & !p4          @errand, @home
deadline:before:friday       ##Work               no date
p1 | (p2 & @urgent)          recurring            subtask
```

with `& | ! ( )` and `,` for multi-list rendering. It costs maybe 400 lines of
pure, exhaustively testable code, and it pays for itself immediately: Today is
`due:today | overdue`, Pinboard is `pinned`, a project is `#Name`. Saved
user filters then fall out for free as a sidebar item holding a query string.

### Natural-language quick add

One entry, parsed as you type, tokens highlighted and stripped from the title:

```
Email Sam about the lease #Work /Admin @email p2 friday 9am !30m
```

`#project` `/section` `@label` `p1..p4` `!reminder`, plus dates: `today`,
`tomorrow`, `fri`, `next friday`, `27th`, `in 3 days`, `end of month`,
`every other monday`, `every! 10 days` (recur from completion, not from due).
English only. Written here as pure functions in `model/parse/` — this is the
signature feature and the most test-dense module in the crate.

`Ctrl+Return` keeps the dialog open for the next task, carrying date/project
forward.

### Recurrence

Own type, not a full RFC 5545 RRULE: interval + unit + weekday set + end
condition (never / on date / after N), and the `every` vs `every!` distinction
(next occurrence computed from the due date vs from the completion date). That
covers everything the NL syntax can express and nothing it can't.

### Also in v1

Quick Find (`Ctrl+F`) fuzzy search over tasks, projects, labels. Desktop
notifications for reminders, re-armed on a midnight tick. Multi-select toolbar
for bulk date/priority/label/complete/delete. Undo via toast on delete and
complete. JSON export/import. Per-project progress rings.

### Deliberately not in v1

Sync, attachments, calendar-event display (needs libecal), change history,
productivity/karma stats, a CLI + D-Bus service, i18n. Each is additive.

## Architecture

```
src/
  model/                     no GTK — cargo test with no display
    task.rs, project.rs, label.rs    the record types (serde)
    recurrence.rs                    next-occurrence arithmetic
    query.rs                         filter parser + evaluator
    parse/                           quick-add: tokens, dates, recurrence NL
    store.rs                         the JSON document, atomic writes
    schedule.rs                      which reminders fire when
  ui/
    application.rs      owns the store; the only thing that mutates a task
    window.rs           the split views, the breakpoint, the view stack
    task_row.rs         one row; emits intent, never persists
    task_list.rs        ListStore + ListView over a query result
    detail_panel.rs     the 360px right-hand pane
    quick_add.rs        the entry, live parse feedback, chips
    project_view.rs     a project as sections or as a board of them
    sidebar.rs
    style.css
```

**The store is canonical**, exactly as in Stickies: rows emit signals describing
what the user did, `PlannerApplication` is the single place that mutates or
writes. A `dirty: Cell<bool>` plus a `glib::timeout_add_local` tick coalesces
saves so typing never blocks on I/O; writes go tmp → fsync → rename; a corrupt
file is set aside and a newer schema version opens read-only.

**Persistence is one JSON file**, `~/.local/share/planner/planner.json`, held
entirely in memory. A personal task list is tens of kilobytes; evaluating a
filter over every task is a microsecond-scale linear scan, and the file stays
greppable, syncable and hand-editable. `Store` exposes a query interface rather
than field access, so SQLite could slot in behind it — but that only becomes
worth a dependency somewhere north of ~50k tasks, which this will never see.

**Lists use `gio::ListStore` + `gtk::ListView`** with a `SignalListItemFactory`.
This is the one real deviation from Stickies, which had one window per record
and needed none of it; a planner needs recycling and per-item bindings.

**Widget tree in Rust, styled by `include_str!`'d CSS.** Structure follows
Planify's spine because it is the right one: outer `Adw.OverlaySplitView` for
the nav sidebar, inner `Adw.OverlaySplitView` packed `END` at 360px for the task
detail panel, one `Adw.Breakpoint` at `675sp` collapsing both.

**Nothing async.** GLib timers for the save tick, the midnight re-arm, and the
quick-add parse debounce; `glib::spawn_future_local` if anything ever needs to
await.

## Testing

Same four layers as Stickies. Unit tests inline per model module — parsing,
queries, and recurrence are where the bugs live and they are all pure functions,
so the ratio should be lopsided towards them. `tests/session.rs` for whole
scenarios against the real store, one `tests/widgets.rs` with a hand-rolled case
runner (GTK is thread-affine), `tests/lifecycle.rs` driving the real application
under a redirected `XDG_DATA_HOME` and a test-only app id. `./test.sh` runs fmt,
clippy `-D warnings`, and tests, with `--headless` under Xvfb.

An `examples/preview.rs` renders the row, detail panel and board to PNG, so
"does this look right?" is answerable without a Wayland screenshot prompt.

## Dependencies

`gtk4`, `libadwaita`, `gio`, `serde`, `serde_json`, and `chrono` — the last one
being the only addition over Stickies. Date arithmetic across DST, month-end
clamping and weekday math is exactly the kind of thing to not hand-roll. Icons,
`.desktop`, metainfo and cargo+bash packaging scripts follow Stickies.

## Milestones

1. ~~Model core: records, store, recurrence, query parser. No UI. All tested.~~
2. ~~Shell: window, split views, sidebar, list view, a task row you can check
   off.~~
3. ~~Quick add with full NL parsing.~~
4. ~~Detail panel: description, dates, priority, labels, subtasks.~~
5. ~~Sections, board view, drag and drop.~~
6. ~~Reminders and notifications, multi-select, Quick Find, saved filters.~~
7. ~~Packaging: deb, Flatpak, icons, metainfo.~~

## Built differently, or not built

Where the finished thing differs from this document, this is what happened.

- **A Labels view** is not in the sidebar. `@label` works in queries and a
  saved filter covers it, so a fixed view would be a second way to do the same
  thing. Cheap to add if the filter turns out to be too much ceremony.
- **Markdown in the description** is stored and round-trips, but is shown as
  plain text. Rendering it needs the live-preview parser lifting out of
  Stickies, which is a job of its own.
- **JSON export/import** is not built. The store *is* a JSON file, so copying
  it is the export; a menu item that copies a file is not yet worth the code.
- **Progress rings** per project are a "3 of 8 done" subtitle instead. The
  number says the same thing and needs no custom drawing.
- **Keep adding** is a checkbox plus `Ctrl+K`, not `Ctrl+Return`. Two ways to
  commit a dialog is a way to press the wrong one.
- **`tests/lifecycle.rs`** was not needed: `tests/widgets.rs` drives the real
  widgets and `tests/session.rs` drives the real store, which is what that
  file was going to cover between them.
- **Bulk label editing** is not in the multi-select bar. Priority, date,
  complete and delete are; labels need a picker that does not exist yet.

## Settled

- App id `us.hagreli.Planner`, binary `planner`.
- `deadline` stays, as a chip in the detail panel distinct from `due`.
- Board columns are sections only. Grouping by priority or label is a small
  follow-on once the query layer exists, not a v1 commitment.
- No notes-as-tasks. A task is a task; the description field carries prose.

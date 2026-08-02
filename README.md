# Planner

A task planner for the GNOME desktop. GTK 4, libadwaita, Rust. Everything is
kept on your own machine, in one plain JSON file you can read, grep and back
up.

There is no account, no sync and no network code. If that ever changes it will
be by adding a `--share=network` line to the Flatpak manifest, which is a
visible thing to review.

## Install

```sh
./install.sh          # ~/.local, no root
./uninstall.sh        # and back out again; your tasks are left alone
```

Or build a package:

```sh
packaging/build-deb.sh        # dist/planner_0.1.0_amd64.deb
packaging/build-flatpak.sh    # installed --user
```

### Requirements

GTK 4.22 and libadwaita 1.9 (GNOME 49 or newer), and a Rust toolchain of 1.80
or later to build.

## Using it

Type the whole task on one line and it is read as you go, with each token
highlighted as it is recognised:

```
Email Sam about the lease #Work /Admin @email p2 friday 9am !30m
```

| | |
|---|---|
| `#project` | which project — an unknown name is *not* created |
| `/section` | which section of it |
| `@label` | a label; an unknown one **is** created |
| `p1`–`p4` | priority, `p4` meaning none |
| `!30m` `!2h` `!1d` | remind me this long before it is due |

Dates are English: `today`, `tomorrow`, `fri`, `next friday`, `27th`,
`3 august`, `in 3 days`, `end of month`, `2026-08-03`. Times too: `9am`,
`17:30`, `noon`, `at 5pm`.

Ambiguous numeric dates like `03/07` are refused rather than guessed — whether
that is July or March depends on where you grew up, and quietly filing a task
five months out is worse than saying no.

### Repeats

`every day`, `every 3 days`, `every other monday`, `every mon and fri`,
`every weekday`, `every month`, `every week until 1 september`,
`every day x3`.

`every` and `every!` are different rules, and the difference is the point:

- **`every 10 days`** counts from the due date. Complete it three weeks late
  and the next one is still ten days after the one you missed.
- **`every! 10 days`** counts from *completion*. Water the plants ten days
  after you last actually did, not ten days after you were meant to.

The same phrase is how you change one later: the date picker has a repeat box,
already filled in with what the task does now. It is a text box rather than a
row of spinners because `every!` is the one piece of this that has no obvious
widget, and a control set that could not say it would be worse than the syntax.
Emptying the box stops the repeat and keeps the date; a phrase that will not
parse changes nothing rather than guessing. Setting a repeat on a task with no
date puts it on the rule's first occurrence — `every monday` typed on a
Thursday means the coming Monday.

Rules are shown as the phrase that would have produced them, so the schedule
row reads `Mon · every weekday` rather than "repeats".

### Views and filters

Inbox, Today, Upcoming, Pinned and Completed are built in. Every one of them is
a filter query — Today is literally `due: today | overdue` — so your own saved
filters are the same machinery, not a lesser version of it.

```
p1 & due before: next week
@errand | @town
##Work & !subtask
overdue, no date
```

`&` `|` `!` `( )` combine terms; a comma renders separate lists. Terms:
`due:` / `deadline:` with `before:` and `after:`, `overdue`, `no date`,
`no deadline`, `no labels`, `recurring`, `subtask`, `pinned`, `completed`,
`p1`–`p4`, `@label`, `#project`, `##project` (including subprojects),
`/section`, `search: text`. A name containing an operator is escaped with a
backslash: `#R\&D`.

A saved filter that will not parse matches nothing, and the editor tells you
why while you type it.

### Projects

Projects nest, and each has sections. A project shows as a list of its sections
or as a board of columns — the toggle is in the header and is remembered per
project. Tasks drag between positions and sections either way. Sections are
added from the project menu and renamed or deleted from the menu on their own
header; deleting one leaves its tasks in the project, and undo puts it back.

### Keyboard

| | |
|---|---|
| `Ctrl+N` | new task |
| `Ctrl+F` | quick find |
| `Ctrl+B` | show or hide the sidebar |
| `Ctrl+K` | keep adding, in the quick-add dialog |
| `Ctrl+Q` | quit |

### From a script or an assistant

`planner agent` reads and changes tasks from outside the window, printing JSON.

```sh
planner agent overview                     # projects, labels, counts
planner agent list 'due: today | overdue'
planner agent add Email Sam #Work @email p2 friday 9am
planner agent complete 'Email Sam'
planner agent update 'Email Sam' due=next friday priority=p1
```

It speaks the two languages the window already uses — a quick-add line to
create, a filter query to list — rather than a second set of fields that could
disagree with them. `planner agent help` documents both; `planner agent
describe` prints the same thing as JSON, for a caller generating tool
definitions.

When Planner is running, the command is handed to it over the same D-Bus
channel a second launch uses. That is not a detail: the running app holds the
whole document in memory, so a separate process writing the file would be
overwritten by its next save. Handing the command over instead means the
window updates as the commands run. With no instance running, the command
reads and writes the file itself.

Replies name things rather than pointing at them — a project name, not a
project id — and they say what actually happened. Completing a repeating task
returns `completed-and-repeats` with the date it comes back on, because it is
not finished. A reference matching two open tasks is an error listing both with
their ids rather than a guess.

## How it works

```
src/
  model/            no GTK, no display — unit-testable anywhere
    task.rs           a task: due date, deadline, priority, labels, reminders
    project.rs        projects, their sections, labels, saved filters
    recurrence.rs     repeat rules and the arithmetic that steps them
    due.rs            a date, optionally a time, optionally repeating
    query.rs          the filter language: parser and evaluator
    search.rs         Quick Find's ranking
    schedule.rs       which reminders are due, and when the next one is
    store.rs          the JSON file: atomic writes, corruption recovery
    parse/            quick add — dates, times, repeats, tokens
    agent/            the `planner agent` surface: verbs, JSON, its own help
  ui/
    application.rs    owns the store, the save tick and the reminder tick
    window.rs         the split views, the breakpoint, the view switching
    sidebar.rs        built-in views, saved filters, the project tree
    project_view.rs   a project as a list of sections or a board of them
    task_list.rs      one recycling ListView, its drop target and selection
    task_row.rs       one row; renders and reports, never persists
    detail_panel.rs   everything about one task
    quick_add.rs      the entry, live parse feedback, chips
    quick_find.rs     search across tasks, projects and labels
    date_picker.rs    quick choices, a calendar, and the same date parser
```

**The store is canonical.** Widgets emit signals describing what the user did;
`PlannerApplication` is the only thing that mutates a task or writes to disk.
One place to lose data means one place to get it right.

**Saving is coalesced.** A two-second tick flushes the store if anything
changed, so typing never blocks on I/O. Writes go to a temporary file and are
`fsync`ed before an atomic rename, so an interrupted write cannot destroy the
previous file. A file that fails to parse is moved to
`planner.json.corrupt-<timestamp>` and the app starts empty rather than
refusing to launch; a file from a *newer* schema version is opened read-only
and never overwritten.

**Every view is a query.** There is no bespoke filter object per view, which
is why a bug in Today is a bug in your saved filters too — and why there is
only one thing to get right.

**Nothing reads the clock below the UI.** Every model function that depends on
today's date takes it as an argument. That is what makes "typing `31st` in
September" a test rather than a thing you find out about in September.

**One JSON file, held in memory.** A personal task list is tens of kilobytes
and every view is a linear scan over a few thousand records. `Store` exposes a
query interface rather than field access, so SQLite could slot in behind it —
but that only becomes worth a dependency somewhere north of ~50k tasks.

Tasks live in `~/.local/share/planner/planner.json`.

## Development

```sh
cargo run                                # against your real tasks
XDG_DATA_HOME=/tmp/scratch cargo run     # against a throwaway store

./test.sh              # fmt, clippy, tests
./test.sh --headless   # the same under Xvfb and a private D-Bus session

cargo run --example preview -- /tmp/preview
cargo run --example preview -- /tmp/preview dark
```

`preview` renders the real widget tree to PNG. Screenshotting a live GNOME
Wayland session needs interactive consent, which makes "does this look right?"
hard to answer while iterating; this paints the widgets offscreen instead.

### Tests

| Where | Covers |
|---|---|
| `src/model/**` | Dates, repeats, queries, search, reminders, the store |
| `src/ui/**` | Date formatting, quick-add chips, drop-position arithmetic |
| `tests/session.rs` | Whole scenarios: relaunch, corrupt file, a repeat running out |
| `tests/widgets.rs` | The real widgets, headless |

`tests/widgets.rs` is one `#[test]` on purpose: GTK may be initialised from
exactly one thread and every widget call must come from it, but Rust's test
harness spawns a thread per `#[test]` and `--test-threads=1` only serialises
them. The runner inside it names each case and continues after failures.

The lopsided ratio is deliberate. Almost every bug worth having in a planner is
in date arithmetic, recurrence, filter evaluation or parsing — all pure
functions over plain data, none of which needs a display to test.

## Licence

GPL-3.0-or-later.

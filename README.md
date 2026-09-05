# Planner

A keyboard-driven task planner for Omarchy. Qt 6 / Qt Quick, C++. Everything
is kept on your own machine, in one plain JSON file you can read, grep and
back up — the same file the GTK version of this app (the `main` branch)
writes, so the two can be swapped without migrating anything.

There is no account, no sync and no network code.

## Install

```sh
./install.sh          # builds, then installs into ~/.local; no root
./uninstall.sh        # and back out again; your tasks are left alone
```

Or `makepkg -si` in `packaging/` for a pacman package. Needs `qt6-base`,
`qt6-declarative`, `qt6-svg`, `cmake` and `ninja` to build.

The window is a normal Hyprland client: no titlebar, no controls, no border
of its own. Colours come from the active Omarchy theme (`colors.toml`) and
re-tint live; every size follows `omarchy display text size`.

## Using it

Type the whole task on one line and it is read as you go, with each token
highlighted as it is recognised and the result shown as chips underneath:

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

Ambiguous numeric dates like `03/07` are refused rather than guessed.

### Repeats

`every day`, `every 3 days`, `every other monday`, `every mon and fri`,
`every weekday`, `every month`, `every week until 1 september`,
`every day x3`.

`every` and `every!` are different rules, and the difference is the point:

- **`every 10 days`** counts from the due date. Complete it three weeks late
  and the next one is still ten days after the one you missed.
- **`every! 10 days`** counts from *completion*. Water the plants ten days
  after you last actually did, not ten days after you were meant to.

The same phrase is how you change one later: the date picker (`Ctrl+D`) has a
repeat box, already filled in with what the task does now. Emptying the box
stops the repeat and keeps the date; a phrase that will not parse changes
nothing rather than guessing. Rules are shown as the phrase that would have
produced them, so a row reads `every! 10 days` rather than "repeats".

### Views and filters

Inbox, Today, Upcoming, Pinned and Completed are built in, on keys `1`–`5`.
Every one of them is a filter query — Today is literally `due: today |
overdue`, and the header shows it — so your own saved filters are the same
machinery. `New Filter…` in the palette takes a query, then a name.

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
backslash: `#R\&D`. A saved filter that will not parse matches nothing, and
the editor says why while you type it.

### Projects

Projects nest, and each has sections. A new one comes from the `+ new project`
row at the end of the rail, or `New Project…` in the palette; `New Subproject…`
in the palette nests one under the project being looked at. A project shows as lanes down the page
or as a board of columns (`Ctrl+Shift+B`), remembered per project. Sections
are added with `Ctrl+Shift+N` and renamed or deleted from the palette;
deleting one leaves its tasks in the project, and `Ctrl+Z` puts it back.

### Keyboard

There are no modes. Every action has a key, and the keys are shown where they
apply: on the cursor row, in the header, in the status line.

| | |
|---|---|
| `↑` `↓` | move the cursor; `←` `→` change board column |
| `Space` | complete or reopen the cursor row |
| `Enter` | open it in the detail pane |
| `Ctrl+D` | date picker for the cursor row (or the selection) |
| `Ctrl+Shift+P` | pin |
| `Del` | delete, with undo |
| `Ctrl+N` | new task; `Ctrl+K` inside the prompt keeps adding |
| `Ctrl+F` | quick find across tasks, projects and labels |
| `Ctrl+K` | command palette: actions, views and tasks in one list |
| `Ctrl+B` | show or hide the rail |
| `Ctrl+A` | select all; `Space` then toggles marks, `Ctrl+1`–`4` sets priority |
| `Ctrl+Z` | undo the last complete, delete or removal |
| `Ctrl+↑↓` | move a task within its lane; `Ctrl+←→` between columns |
| `Ctrl+Enter` | add a subtask, from the detail pane |
| `Esc` | close the innermost thing; with nothing open, the window |
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
create, a filter query to list. `planner agent help` documents both;
`planner agent describe` prints the same thing as JSON.

When Planner is running, the command is handed to it over a local socket and
the window updates as the commands run. That is not a detail: the running app
holds the whole document in memory, so a separate process writing the file
would be overwritten by its next save. With no instance running, the command
reads and writes the file itself. A second bare launch raises the open window.

## How it works

```
src/core/           plain C++ over QtCore — no display, ctest-covered
  model.*             the records, and their JSON in the on-disk shape
  dates.*             English dates, times and repeat phrases
  quickadd.*          the one-line parser, with spans for highlighting
  store.*             the JSON file: QSaveFile writes, corruption recovery
  query.*             the filter language: parser and evaluator
  search.*            Quick Find's ranking
  schedule.*          which reminders are due, and when the next one is
  present.*           how a date reads on a row; the built-in views
  agent.*             the `planner agent` surface: verbs, JSON, its own help
  demo.*              the sample store behind --demo
src/app/
  app.*               the App singleton: state, every derived row, save tick
  icons.*             symbolic SVGs recoloured in the theme's palette
  single.*            one instance; agent commands forwarded over a socket
  qml/                T.qml tokens, components/, views/ — a view over App
```

**The store is canonical.** QML reports what the user did; `App` is the only
thing that mutates a task or writes to disk. Saving is coalesced on a
two-second tick through `QSaveFile`, so an interrupted write cannot destroy
the previous file. A file that fails to parse is moved to
`planner.json.corrupt-<timestamp>` and the app starts empty; a file from a
*newer* schema version is opened read-only and never overwritten.

**Every view is a query.** There is no bespoke filter per view, which is why
a bug in Today is a bug in your saved filters too — and why there is only one
thing to get right.

**Nothing below `App` reads the clock.** Every core function that depends on
today's date takes it as an argument, which is what makes `bin/grab` render
the design's date on any day and "typing `31st` in September" a test.

Tasks live in `~/.local/share/planner/planner.json`.

## Development

```sh
bin/build                                   # cmake + ninja into build/
cd build && ctest --output-on-failure       # the core suites
./build/planner --demo                      # the design's sample tasks, scratch store
./build/planner --data /tmp/x               # an isolated store
bin/grab                                    # every surface as a PNG in docs/screens
OMARCHY_THEME_DIR=/usr/share/omarchy/themes/catppuccin-latte ./build/planner --info
```

`bin/grab` renders headless (`QT_QPA_PLATFORM=offscreen`) from the demo store
pinned to Saturday 5 September 2026, the date the design was drawn for.
`DECISIONS.md` records where the build departs from `design/HANDOFF.md`.

## Licence

GPL-3.0-or-later. The symbolic icons in `src/app/icons/` are from the
GNOME Adwaita icon theme (GPL / CC-BY-SA).

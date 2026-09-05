# Decisions

Where the build departs from `design/HANDOFF.md`, or settles something it left
open, this is what happened and why.

## Toolchain

- **C++ over the Rust model, not Rust over a bridge.** The handoff allowed either
  keeping `src/model/**` or moving to Qt. A first attempt exposed the Rust crate
  as a static library behind a JSON C ABI; it was dropped for a straight port to
  `src/core/` (QtCore only), the way buddy and ledger are built. One language, one
  build, and the model tests came across as ctest suites (nine of them) rather than
  living in a crate the app could not run. The on-disk JSON is byte-compatible in
  meaning with the Rust app's: same keys, same enum spellings, same timestamp and
  time formats, so a `planner.json` written by either reads in the other.
- **Key order in the file is alphabetical**, because `QJsonObject` sorts keys where
  serde wrote struct order. Same data, different diff.
- **Ids are `QString` aliases**, not the Rust newtypes. The compiler no longer
  catches a project id passed for a task id; signatures say which they take.

## Sync

- **Schema v2 and the sync design come from the `server-sync` branch** (now
  `main`), ported to C++ as written: sections as records, order keys,
  tombstones with 90-day retention, three-snapshot planning, last-writer-wins
  per record, deletion never beats an edit. `sync-base.json` uses the Rust
  client's shape (`[[{kind,id},{Live|Deleted: at}]]`), so either client can
  take over a machine's base.
- **The Rust `core/` and `server/` stay in the repo** so the container can be
  built from this branch; only the GTK shell (`src/` on the old workspace) was
  dropped. The C++ core mirrors planner-core's format and is tested to read
  what it writes.
- **The worker does network and nothing else.** A pass gathers on a
  `std::thread` over a blocking `QTcpSocket` and applies on the main thread
  through the same `mutate` path every edit uses; the save tick does the write.
- **Failures are reported after three in a row**, in the status line, below a
  save error. A failed long poll says nothing; the three-minute tick is the
  backstop.
- **`--demo` never syncs**, whatever the config says.
- `planner sync now|status` is a second CLI beside `planner agent`; it is
  answered by the running window or, with none, runs a pass against the file.

## Layout and chrome

- The window is a normal Hyprland client: no titlebar, border or shadow of its own.
  The **yellow border in selection mode** shown on surface G is the compositor's
  active border and is not set by the app.
- The rail's **`SUPER T` keycap is read from `hyprctl binds`** and shown only when a
  bind targets `planner`, so it is never a lie. While selecting, the same slot
  shows `N SELECTED`.
- Design px map 1:1 through `T.s()` at scale 1; the mock's Catppuccin hexes map to
  palette roles (`surface0` = lighter_background, `surface1` = selection,
  `meta` = mix(text2, muted, .5) = overlay2, `hintText` = mix(text2, muted, .25)
  = subtext0), so every theme gets the same ramp.
- **Priority hues follow the theme's red/peach/blue roles**, not a fixed hex: the
  handoff's rule is that they never follow the user accent, and they do not.
- Fonts are the system `sans-serif` and `monospace` aliases, as the handoff asks.

## Behaviour

- **Board drag and drop is keyboard only** (`Ctrl+←→` between columns,
  `Ctrl+↑↓` within a lane). Mouse drag was not built; the columns are still
  one list per section, so it can be added per lane later.
- **Deleting a project asks first** with a confirmation prompt (Enter deletes,
  Esc keeps it), then offers undo; a section deletion goes straight to a toast
  with undo, since its tasks stay.
- **Toasts live in the status line** (the middle text, `· ctrl+z undo` appended
  when the action can be undone), for six seconds. Complete and delete report with
  undo; priority and reschedule report a plain count.
- **`Ctrl+Z` undoes the last undoable action only**, whatever it was: a bulk
  complete, a delete, a section or project deletion.
- **Text-argument actions use the same prompt shell** (`New Section…`,
  `New Project…`, `Rename…`, `New Filter…`), titled with what they will do. A
  filter is entered as its query first (validated live, an unparseable one will
  not save) and then its name.
- **The detail pane follows the cursor** while it is open, and closes when the
  view changes; Esc closes it too. When its task leaves the list — completed out
  of Today, or deleted — the pane moves to the cursor row; where the view still
  shows the task it stays, struck through. Enter opens the cursor row, and the arrow
  keys then read each task in turn.
- **Labels are edited in the detail pane**: click a chip to remove it, type into
  the `+` box and press Enter to add one (created if new). No label picker.
- The date picker is **live**: each change writes to the task at once, so Escape
  simply closes it. Bulk scheduling sets the day only and keeps each task's own
  time and repeat; the deadline picker has no time or repeat row.
- Quick add lands in the **project being looked at** (and the cursor row's
  section on a project), else the Inbox — as the GTK app did. `Ctrl+K` in the
  prompt adds and keeps it open.
- **Palette results** are actions (context-dependent), then views, then tasks;
  the count reads `n of N` over the selectable rows.
- A task opened from find or the palette **switches to the view it lives in**
  (Inbox or its project) before opening, so the list keeps showing it.
- Reminders fire as desktop notifications through `org.freedesktop.Notifications`
  on a 30-second tick; anything already overdue at launch is marked seen.
- The GTK app's `--new-task` launch option was not carried over; `Ctrl+N` in the
  window and `planner agent add` cover it.

## Not built

- Markdown rendering of descriptions (stored and shown as plain text).
- JSON export/import beyond the file itself.
- Sorting a project other than manually (`sort_by` is read and written, not applied).

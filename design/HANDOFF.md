# Handoff: Planner — Omarchy Quattro surface

## Overview

`mhagrelius/planner` is a GTK 4 / libadwaita task planner in Rust (GNOME 49+, one JSON file at
`~/.local/share/planner/planner.json`, no network). This handoff covers two things:

1. **`Planner GTK Recreation.dc.html`** — the UI as it exists today, rebuilt from `src/ui` so the
   two can be compared side by side. Reference only; do not port it back.
2. **`Planner Quattro.dc.html`** — the design to build: the same app restyled and restructured for
   Omarchy 4 ("Quattro", Quickshell / Qt 6, Catppuccin Mocha), as a keyboard-driven window rather
   than a header-bar app.

The intended end state is the existing Rust model layer (`src/model/**`, which is display-free and
already tested) driving a new Quattro-aligned front end. The design is drawn as a normal Hyprland
client — a window with gaps, rounded corners and an active border — **not** as a Quickshell shell
plugin. That was considered and explicitly declined.

## About the design files

The two `.dc.html` files are **design references written in HTML**. They are static prototypes of
look and behaviour — no state, no interaction, no data. Do not lift the markup.

Recreate them in the target environment. For this repo that means **GTK 4 + libadwaita widgets in
Rust** (keep `src/model/**` and `src/ui/application.rs`'s store ownership; replace the widget tree),
or **QML / Qt Quick** if the decision is made to move the front end to the Quattro toolchain. Either
way, the layer boundary the README of the repo describes must survive: widgets emit signals saying
what the user did, `PlannerApplication` is the only thing that mutates the store or writes to disk.

Opening a `.dc.html` file in a browser renders it. `support.js` is the runtime that does that; it is
part of the prototype tooling and has nothing to do with the app.

## Fidelity

**High fidelity.** Every colour, size, weight, radius and string below is measured off the design
files and should be matched. Two substitutions are deliberate and must be corrected in
implementation:

- **Fonts.** The design uses **IBM Plex Sans** and **IBM Plex Mono** because they are web-available.
  Omarchy ships **CaskaydiaMono Nerd Font** and Quattro's bar is configured for **Berkeley Mono**.
  Implement with the system's configured UI font and monospace font, not with IBM Plex.
- **Icons.** The design reuses the **Adwaita symbolic icons the current app already names** (see
  Assets). Keep them if the front end stays GTK. If it moves to QML, substitute the icon set the
  shell uses at the same 15–16 px optical size.

## Design tokens

### Colour — Catppuccin Mocha

| Token | Hex | Used for |
|---|---|---|
| crust | `#11111b` | the area outside the window (monitor / wallpaper stand-in); text on accent fills |
| mantle | `#181825` | rail background, status line, board columns, prompt footers, inset fields |
| base | `#1e1e2e` | window background, popovers, board cards |
| surface0 | `#313244` | keycaps, label chips, hairline separators, unselected toggle track |
| surface1 | `#45475a` | **selected rail row fill**, chips on selected rows, popover border, unset checkbox ring, out-of-month calendar days |
| overlay2 | `#9399b2` | secondary mono meta: counts, `1 of 2`, `every! 10 days`, weekday letters, completed titles |
| subtext0 | `#a6adc8` | hint and legend copy, uppercase section labels, status-line shortcuts |
| subtext1 | `#bac2de` | body text on non-focused rows, description text, values in the detail pane |
| text | `#cdd6f4` | primary text, focused-row text, chip text on `#45475a` |
| mauve | `#cba6f7` | window active border, cursor/selection accent, prompt sigil `›`, selected calendar day |
| blue | `#89b4fa` | project accent (Work), p3 priority, quick-find accent |
| green | `#a6e3a1` | due today, completed checkbox fill, quick-add sigil `+`, project accent (Home) |
| yellow | `#f9e2af` | selection state: window border, rail badge, checked selection boxes, cursor outline |
| peach | `#fab387` | p2 priority |
| red | `#f38ba8` | p1 priority, overdue dates, past deadlines, destructive keycaps |
| teal | `#94e2d5` | project accent (Admin) |

Contrast rule that was enforced and must hold: **surface-ramp colours (`#313244`, `#45475a`,
`#585b70`, `#6c7086`) are for fills, borders and separators only — never for copy.** All text sits on
`#9399b2` or lighter. Hint text measured ≥ 4.5:1 on its own background at 10–12 px.

Priority hues are fixed and must not follow a user accent: **p1 red, p2 peach, p3 blue, p4 no
colour** (`css_class()` in `src/model/priority.rs` returns `None` for p4 — an unset priority draws
no ring).

### Typography

| Role | Family | Size | Weight | Other |
|---|---|---|---|---|
| View title | sans | 20 px | 600 | `letter-spacing: -.01em` |
| Detail title | sans | 19 px | 600 | `line-height: 1.35` |
| Empty-state title | sans | 18 px | 600 | |
| Row title / body | sans | 15 px | 400 | |
| Secondary body (description, detail values) | sans | 14 px | 400 | `line-height: 1.5–1.55` |
| Dates, counts, queries | mono | 13 px | 400 | |
| Meta, chips, status line | mono | 12 px | 400 | |
| Keycaps, legends, hints | mono | 11 px | 400 | |
| Uppercase section labels | mono | 10 px | 400 | `letter-spacing: .16em; text-transform: uppercase` |
| Rail wordmark | mono | 12 px | 400 | `letter-spacing: .16em; text-transform: uppercase` |
| Column headers (board / lane) | mono | 11 px | 400 | `letter-spacing: .14em; text-transform: uppercase` |

Anything numeric or date-like is mono; anything a person wrote is sans. That split is the design.

### Spacing, radii, borders

- Window: `border: 2px solid` accent, `border-radius: 14px`, background `#1e1e2e`.
- Hyprland gap around the window: `16px` of `#11111b` on all sides.
- Rail: `236px` wide (`222px` in the narrow empty-state surface), background `#181825`.
- Rail row: `padding: 9px 12px`, `margin: 0 8px 2px`, `border-radius: 8px`. Subproject indent:
  `padding-left: 28px`.
- List row: `padding: 12px 20px`, `gap: 12px`, no separators. Measured height 45 px.
- Due column: fixed `104px`, right-aligned, so dates line up down the list.
- Detail pane: `372px`, background `#181825`, `border-left: 1px solid #313244`, body `padding: 0 18px 18px`, section `gap: 20px`, groups divided by `1px solid #313244`.
- Status line: `height: 30px`, background `#181825`, `border-top: 1px solid #313244`.
- Checkbox: `18px`, `border-radius: 5px`, `2px` ring in the priority colour (`#45475a` when unset);
  filled with the state colour + a 12 px check glyph when done or selected. Board cards use `16px`.
- Chips (labels, keycaps): `border-radius: 4px`, `padding: 1px 6px`. Square-ish, not pills — pills
  are the GTK design's idiom, and dropping them is intentional.
- Popovers (palette, prompts, date picker): `border: 1px solid #45475a`, `border-radius: 12px`,
  `box-shadow: 0 24px 60px rgba(0,0,0,.55)`.
- Board columns: `flex: 1`, background `#181825`, `border: 1px solid #313244` (`#45475a` on the
  column holding the cursor), `border-radius: 10px`; cards `background: #1e1e2e`,
  `border-radius: 8px`, `padding: 10px`.
- Project colour dot: `7px` square, `border-radius: 2px`.

### Selection and focus — the two treatments

This was iterated on and matters. **No left accent stripes anywhere.**

- **Rail, selected view**: row filled `#45475a`; the accent lives in the row's own number keycap
  (filled mauve / blue / yellow with `#11111b` text) and the icon switches to the `text` colour.
- **List, cursor row**: row filled `#313244`, text `#cdd6f4`, keycap hints revealed at the right.
- **Board, cursor card**: filled `#313244` plus `outline: 1px solid #cba6f7; outline-offset: -1px`.
- **Selection (multi-select)**: rows filled `#313244` with a yellow-filled checkbox; the *cursor*
  row inside a selection is the one with `outline: 1px solid #f9e2af; outline-offset: -1px`.
- **Palette / find, active result**: filled `#313244` with a mauve (palette) or blue (find) marker.

## Keyboard model

The design originally proposed a vim-style modal layer. **That was rejected.** There are no modes,
no mode indicator, and no `j/k`/`dd`/`H/L` bindings. Conventional shortcuts only, extending the five
the app already ships (`Ctrl+N`, `Ctrl+F`, `Ctrl+B`, `Ctrl+K` in quick-add, `Ctrl+Q`):

| Key | Action |
|---|---|
| `↑` `↓` | move the cursor |
| `Space` | complete / uncomplete the cursor row (toggles a selection mark while a selection exists) |
| `Enter` | open the cursor row in the detail pane; run the highlighted palette or find result |
| `Ctrl+D` | open the date picker for the cursor row |
| `Ctrl+Shift+P` | pin the cursor row |
| `Del` | delete |
| `Ctrl+N` | new task (quick-add prompt) |
| `Ctrl+F` | quick find |
| `Ctrl+K` | command palette |
| `Ctrl+B` | show / hide the rail |
| `Ctrl+A` | select all (starts a selection) |
| `Ctrl+1`–`Ctrl+4` | set priority p1–p4 on the selection |
| `Ctrl+Z` | undo the last bulk action |
| `Ctrl+Shift+B` | toggle list / board on a project |
| `Ctrl+Shift+N` | new section |
| `←` `→` | change board column |
| `Ctrl+←` `Ctrl+→` | move a task between board columns |
| `Ctrl+Enter` | add a subtask, from the detail pane |
| `1`–`5` | jump to the numbered built-in view (numbers shown in the rail) |
| `Esc` | clear a selection; close a prompt; leave the detail pane; close the window |
| `Ctrl+K` (in quick-add) | keep adding — unchanged from today's dialog |

Two presentation rules follow from this:

- **Keycap hints are visible, not discovered.** The cursor row shows `space done`, `enter open`,
  `ctrl+d date`. The view header shows `ctrl+n add`, `ctrl+f find`, `ctrl+k palette`.
- **The status line replaces the slide-up action bar.** `gtk::ActionBar` revealing on selection is
  gone; the status line's right half becomes the bulk actions instead.

## Surfaces

Ten labelled surfaces, `data-screen-label` on each. Content throughout is the seeded store from
`examples/preview.rs`, dated **Saturday 5 September 2026**.

### A — Today (`A Today`)

Window `1240 × 600` inside its 16 px gap; mauve border.

**Rail** (`236px`): wordmark `planner` + a `SUPER T` keycap (the summon binding). Then the five
built-in views, each `<number keycap> <icon> <label> <open count>`: Inbox 9, **Today 6 (selected)**,
Upcoming 2, Pinned, Completed. Pinned and Completed show no count — Completed deliberately never
does (`src/ui/sidebar.rs`: a count of finished work would read as work outstanding). Then a
`projects` label and the tree: Work (blue, 4), Admin (teal, indented one level), Home (green).

**Header**: `Today` at 20/600, then the view's **actual query** in mono — `due: today | overdue`.
Showing the query is a design decision worth keeping: every view in this app *is* a query
(`src/model/query.rs`), and the header is where that becomes visible. Right side: the three global
keycap hints.

**Rows**, in this order, all single-line:

| Title | Ring | Meta | Due (104 px column) |
|---|---|---|---|
| Email Sam about the lease | p1 red | `@email` chip | `Today 09:00` green |
| Renew the parking permit | p2 peach | — | `Today` green |
| Water the plants | unset | `every! 10 days` + repeat icon | `Today` green |
| Pay the electricity bill | p3 blue | — | `Yesterday` red |
| File the tax return | p1 red | `Due 3 Sep` red | `Yesterday` red |
| Move house | p2 peach | `1 of 2`, `Due 14 Sep`, `@errand`, `@home` | `Today 09:00` green |

Row 1 is the cursor row: filled `#313244` with the three keycap hints.

Date strings come from `format_date` / `format_due` in `src/ui/task_object.rs` and must not be
re-invented: `Today`, `Tomorrow`, `Yesterday`, a weekday name 2–6 days out, `%-d %b` beyond that,
`%-d %b %Y` in another year, time appended as `%H:%M`. Deadlines render `Due <date>`; subtask
progress renders `<done> of <total>`.

**Status line**: `today` · `6 tasks · 2 overdue` · right: `↑↓ move · ctrl+b rail · ctrl+k palette · esc close`.

Two tweaks exist on the prototype (`keyHints`, `statusLine`, both default true) purely so the
per-row hints and the status line can be inspected switched off. They are not product settings.

### B — Detail pane (`B Detail pane`)

Window `1240 × 640`. Rail and a shortened Today list on the left, cursor on `Move house`, detail
pane `372px` on the right.

**The pane carries no field-name labels.** This was an explicit iteration: `Schedule`, `Deadline`,
`Priority`, `Labels`, `Project` and the `description` / `details` / `subtasks` headings were all
removed. The icon is the label and the value is the content.

Top strip: uppercase `task`, then `ctrl+shift+p pin` and a red `del` keycap. Then:

- Title `Move house`, 19/600.
- Description, plain text on the pane background: `Ring the agent before Friday.` /
  `Confirm the van booking.`
- Hairline, then four icon+value rows: calendar → `Today at 09:00` (green); alarm → `14 Sep`;
  a peach priority ring → `p2 High`; folder → `#Inbox` followed by the `@errand` and `@home` chips.
  Schedule and deadline strings come from `describe_due` / `describe_deadline` in
  `src/ui/date_picker.rs` — `Today at 09:00`, and `Mon · every weekday` when a rule exists (the rule
  itself, never the word "repeats").
- Hairline, then `1 of 2 done`, the two subtasks (`Pack the kitchen` completed — green filled box,
  struck through, `#9399b2`; `Book a van` open, with a chevron to open it), and a
  `ctrl+enter add a subtask` affordance on its own hairline.

Status line: `Move house` · `tab fields · ctrl+d schedule · ctrl+shift+p pin · del delete · esc back`.

### C — Command palette (`C Command palette`)

Window `1240 × 600`. The Today surface behind is dimmed to `opacity: .35` under an
`rgba(17,17,27,.72)` scrim. The palette is `660px`, centred, `top: 64px`.

Prompt row: mauve `›`, the query `sec`, and `4 of 26` at the right. Results grouped under uppercase
mono headings — `actions` (`Add Section…` selected, with an `in #Work` chip; `Select Tasks` with a
`ctrl+a` keycap) and `tasks` (`Tidy the shared drive` → `#Work`; `File the tax return` → `Yesterday`
in red). Footer: `↑↓ move`, `enter run`, `esc dismiss`, and the note
`every view is a query — filters live in the same list`.

Actions, built-in views, saved filters, projects and tasks all resolve in one list — the palette is
the replacement for the header-bar menus, the project `⋯` menu and the section header menus.

### D — Quick add and quick find (`D1 Quick add`, `D2 Quick find`)

Two `610px` frames. Same shell, different sigil: green `+` for add, blue `/` for find.

**D1** input shows `Email Sam about the lease #Work @email p1 friday 9am !30m` with every recognised
token in mauve and a mauve caret. One colour for all tokens, deliberately — `src/ui/quick_add.rs`
explains why: hue would encode kind for people who can tell hues apart, so kind goes on the chips
instead. Chips below, in parser order: `Work` (folder), `Fri 09:00` (calendar, green), `Urgent`
(priority, red), `email` (bookmark), `30 minutes before` (alarm). Then the app's own hint string
verbatim:

> `#project  /section  @label  p1–p4  !30m  — and dates like “friday 9am”, “in 3 days”, “every other monday”`

Footer: `enter add`, `ctrl+k keep adding`, and `lands in #Work` — the destination is always stated,
because that is the question quick-add is worst at answering.

**D2** query `the`, `6 hits`, results in the ranking `src/model/search.rs` actually produces
(word-start match, shorter titles winning ties, completed tasks pushed down by 500): `Book the
dentist`, `Water the plants`, `File the tax return`, `Tidy the shared drive` (`#Work`), `Email Sam
about the lease`, then `Cancel the old broadband` struck through at `opacity: .6` with
`#Inbox · completed`. Footer: `enter open`, `tasks, projects and labels at once`.

### E — Project as sections (`E Project sections`)

Window `1240 × 560`, rail with **Work** selected. Header: `Work`, `0 of 4 done`, and a list/board
toggle (`list` active: `#313244` fill; `board` inactive) on a `#181825` track.

Unsectioned tasks come first with **no lane header** — `Tidy the shared drive`. Then each section is
an uppercase mono lane header with its count and a hairline running to the right edge:
`in progress` 2 → `Draft the Q3 report` (p1, `Tomorrow`), `Review the contract` (`@legal`);
`blocked` 1 → `Chase the supplier` (p2). Lane headers, not boxes — boxes are for the board.

Status line: `#Work` · `3 sections` · `ctrl+shift+b board · ctrl+shift+n new section · ctrl+↑↓ move task`.

### F — Project as a board (`F Project board`)

Window `1240 × 560`, same rail and header with `board` active. Three equal columns —
`no section` 1, `in progress` 2, `blocked` 1 — each a `#181825` panel with a header, a hairline, and
`#1e1e2e` cards. The cursor card (`Draft the Q3 report`, showing `Tomorrow` on a second line) is
filled `#313244` with a mauve `outline`. The column holding the cursor has a `#45475a` border
instead of `#313244`.

Every column is a drop target, including an empty one — that is why `src/ui/project_view.rs` builds
a list per section rather than one sectioned `ListView`, and the same must hold here.

Status line: `#Work · board` · `←→ column · ctrl+←→ move task · ctrl+shift+b list`.

### G — Selection (`G Select mode`)

Window `1240 × 600`, **yellow** border. Rail identical to A plus a `2 SELECTED` yellow badge where A
shows `SUPER T`. All six Today rows present.

Rows 1 and 2 are selected: filled `#313244`, yellow-filled checkboxes with a `#11111b` check. Row 3
is the cursor: yellow `outline`, unselected. Header hints become `ctrl+a all`, `esc clear`.

Status line, yellow context: `2 tasks selected` · then the bulk actions as keycaps —
`space complete` (green cap), `ctrl+d schedule`, `ctrl+1-4 priority`, `del delete` (red cap),
`ctrl+z undo`. Behaviour to preserve from `src/ui/window.rs`: complete and delete report a toast with
undo; priority and reschedule report a plain count with no undo; a selection is discarded when the
view changes.

### H — Empty state and date picker (`H1 Empty state`, `H2 Date picker`)

**H1** (`740` frame, `222px` rail, Pinned selected): centred pin icon at 44 px `opacity: .5`,
`Nothing pinned`, then the app's own description `Pin a task to keep it in reach.` and
`ctrl+shift+p pins whatever the cursor is on`. Status line: `pinned` · `0 tasks`. Each view keeps its
own empty title and description from `builtin_views()` — "Nothing to do" is only right for some of
them.

**H2** (`340px` popover): a natural-language field with the mauve `›` and placeholder
`next friday, in 3 days…`; four quick buttons `Today` / `Tomorrow` / `Next week` / `No date`;
`September 2026` with pan arrows; a Monday-first calendar grid with 5 highlighted mauve and
out-of-month days at `#45475a`; a `time` field showing `09:00`; a `repeat` field holding
`every! 10 days` with a clear button; and the note
`every! counts from completion. Emptying the box stops the repeat and keeps the date.`

The repeat control is a **text box, not a widget bank**, and must stay one: `every!` (repeat from
completion) has no obvious spinner equivalent, `Recurrence::describe()` writes the phrase back so the
box round-trips, and an unparseable phrase must change nothing rather than guess.

## Behaviour to preserve from the existing app

These are load-bearing and documented in the repo; the redesign changes none of them.

- **The store is canonical.** Widgets report; `PlannerApplication` mutates and saves. Saving is
  coalesced on a 2-second tick, written to a temp file, `fsync`ed, then atomically renamed.
- **Every view is a query**, including the built-in ones. Today is literally `due: today | overdue`.
- **Nothing below the UI reads the clock.** Every model function takes today's date as an argument.
- **Refreshing updates the list in place** (match on id, splice once) so scroll position, selection
  and in-flight animations survive a checkbox tick.
- **Completing a recurring task reschedules it** rather than finishing it; `every` counts from the
  due date, `every!` from completion.
- **Ambiguous numeric dates are refused, not guessed** (`03/07`).
- **An unknown `#project` is not created; an unknown `@label` is.**
- **A saved filter that will not parse matches nothing** and says why while you type it.
- **Deleting a section leaves its tasks in the project** and offers undo; deleting a project asks
  first, because it takes subprojects and their tasks.

## State

Per-surface state the front end needs: selected view id; cursor index within the current list;
selection set (plus whether a selection is active); open task id for the detail pane; per-project
list/board style (persisted per project, as today); prompt state (which of palette / add / find is
open, its query, its highlighted index); date-picker state (target task, calendar month, time text,
repeat text). Everything else is derived from a store query.

## Assets

`icons/` holds the Adwaita symbolic icons the current app names, taken from
`GNOME/adwaita-icon-theme` (`Adwaita/symbolic/**`, 16 px, GPL/CC-BY-SA). Colour variants, produced by
recolouring the single `fill`:

- `icons/` — `#2e3436`, for the GTK recreation
- `icons/w/` — `#ffffff`
- `icons/c/` — `#cdd6f4` (Catppuccin text)
- `icons/cd/` — `#7f849c` (dimmed)
- `icons/cm/` — `#cba6f7` (mauve)
- `icons/cc/` — `#11111b` (crust, for glyphs on accent fills)

In a real build, take these from the installed icon theme and recolour at render time rather than
shipping per-colour copies. Names used: `mail-unread`, `view-continuous`, `x-office-calendar`,
`view-pin`, `object-select`, `folder`, `folder-new`, `user-bookmarks`, `alarm`,
`media-playlist-repeat`, `emblem-important`, `user-trash`, `go-next`, `edit-clear`, `edit-find`,
`system-search`, `document-edit`, `list-add`, `selection-mode`, `sidebar-show`, `open-menu`,
`view-list`, `view-grid`, `view-more`, `pan-start`, `pan-end`, `pan-down`, `window-close`.

No raster images and no hand-drawn illustrations anywhere in the design.

## Files

| File | What it is |
|---|---|
| `Planner Quattro.dc.html` | **The design to build.** Ten surfaces, A–H. |
| `Planner GTK Recreation.dc.html` | Today's GTK UI, rebuilt from `src/ui` for comparison. Reference only. |
| `icons/**` | Adwaita symbolic icons in the colour variants above. |
| `support.js` | Prototype runtime that renders the two HTML files. Not part of the app. |
| `github.md` | Source repo, branch, and a screen → source-file map. |

Source of truth for behaviour, in the repo itself: `README.md` and `DESIGN.md` at the root, then
`src/model/**` for the rules and `src/ui/**` for what the current front end does.

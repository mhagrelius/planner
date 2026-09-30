# Prompt: bring planner onto the shared design language

You are Claude (Fable 5.1) working in `~/Projects/planner`. Planner was built from
a Catppuccin handoff with its own chrome: a 236 rail with a mono wordmark and
filled keycaps, 15 px rows, a 20 px view title, a 30 px status line, a 372 detail
pane. The other three apps share ledger's chrome, and the shared spec reconciles
everything to one language. This is the largest of the four updates. Change how
things look, never what they do: the store, the query language, quick-add, dates,
sync, the agent CLI, the socket and every test are out of scope. Every keyboard
behaviour in `design/HANDOFF.md` and `DECISIONS.md` survives untouched.

## Read first, in this order

1. `/home/matthew/Projects/design/DESIGN.md` — the spec. §3 (type), §4 / §4a / §4b
   (geometry, spacing, layout), §5 (components), §9 (the divergence audit; the
   `planner` column is what this app does today, the last column is the target).
2. `/home/matthew/Projects/design/sheet.html` — the spec as pixels. Render it
   headless if you cannot open a browser:
   `chromium --headless=new --disable-gpu --hide-scrollbars --allow-file-access-from-files --window-size=1440,16200 --screenshot=/tmp/sheet.png file:///home/matthew/Projects/design/sheet.html`
   and crop with `magick`. `tokens.css` there is the CSS form of every size below.
   Layouts L3 (list + detail rail), L5 (palette) and L6 (board) are this app.
3. `/home/matthew/Projects/design/screens/planner/` — every surface of this app as
   it is now, rendered under the Monokai Pro theme. Your "before".
4. This repo's `CLAUDE.md`, `DECISIONS.md`, `design/HANDOFF.md`, and the
   `omarchy-app-dev` skill.

If this checkout is not at `~/Projects/planner` next to `~/Projects/design`, copy
`DESIGN.md`, `sheet.html`, `tokens.css`, `theme.js` and `fonts/` from the design
folder into `docs/design-language/` here and read them from there.

## Invariants

- In scope: `src/app/qml/**`, `T.qml`, `palette.cpp` / `.h`, `main.cpp` (fonts),
  `icons.*` only if a new icon is needed, `CMakeLists.txt` (font resources),
  `packaging/`. Out of scope: `src/core/**`, `core/`, `server/`, `app.cpp` except
  where a row string carries a size, `single`, `remote`, tests.
- Every size through `T.s()` / `T.f()`, every colour through `T.<role>`. No hex.
  `T.surface0` / `T.surface1` are renamed to what they are (`selection` /
  `borderStrong`) as you touch each file.
- No new sizes. After this work, `grep -rhoE 'px: *[0-9.]+' src/app/qml | sort -u`
  must print only `10.5 11.5 12.5 13.5 15 21 25` (the empty-state glyph is an
  `Icon` at 44, not text). Two button sizes, one control height.
- Every departure from the spec, and the two decisions below, go in
  `DECISIONS.md` under a dated "Design language" heading. Where the handoff and
  the spec disagree, the spec wins and the entry says so.

## Changes, in order

Fonts, tokens and components first (1–5), rebuild, `bin/grab`, then the surfaces
(6–12). One commit per numbered step.

1. **Sans face.** Bundle IBM Plex Sans exactly as ledger does: copy
   `~/Projects/ledger/fonts/` into `fonts/`, add to the QML resource, register in
   `main.cpp`, set the application font, `palette.setSansFamily("IBM Plex Sans")`.
   Mono stays the `monospace` alias. Record the decision (the handoff asked for
   system fonts).
2. **Type ramp.** Map every `px:` onto the eight roles (DESIGN.md §3):
   - 10, 11 (mono: eyebrows, keycaps, counts, lane headers, status line, footer,
     hint copy, calendar weekday letters) → **10.5**.
   - 12 (mono meta, label chips, wordmark, rail labels) → **10.5** for chips,
     labels and status text; **12.5** for meta, dates and values in rows.
   - 13 (due column, detail values, calendar days) → **12.5**.
   - 14 (nav labels, descriptions, empty-state copy) → **12.5**.
   - 15 (task rows, quick-add text) → **13.5** (list-row role).
   - 16, 18 (empty-state title), 19 (detail title), 20 (view title) → **15**.
   - Eyebrows (`PROJECTS`, `TASK`, `ACTIVITY`, lane headers, the board column
     headers): mono 10.5, uppercase, `T.r(10.5 * 0.16)`, colour **`muted`** (was
     `hintText`, and lane headers were 11 at .14 em).
3. **Keycap and chips.** `Keycap.qml` becomes the outlined keycap: transparent,
   1 px `borderStrong`, 4 r, mono 10.5, `faint` key text, 1 / 5 pad; the word after
   it stays mono 10.5 in `hintText`. The green `space complete` and red `del`
   caps keep their colour as **text and border** (`positive` / `negative`), not as
   a fill; the accent-filled number keycaps in the rail go away (step 6). Key names are
   lowercase with `+`: the rail badge reads `super+t`, not `SUPER T`.
   `LabelChip.qml`: 18 tall, mono 10.5, 4 r, `selection` fill (`borderStrong` on a
   cursor or selected row) — the 22-tall variant in the prompt becomes 18.
   `Check.qml` is already to spec (18, 5 r, 2 px ring).
4. **Controls.** Add `PrimaryButton.qml` / `SecondaryButton.qml` from ledger (28
   tall, sans 12.5, side pad 12, 6 r; `small: true` → 24, 11.5, pad 8). The date
   picker's quick buttons (Today / Tomorrow / Next week / No date) are small
   secondary buttons. `Field.qml` 28 tall (`padY` 5); the prompt field
   (palette / add / find) is the one exception at **32** tall, sans 13.5, with the
   mono sigil. The list / board toggle in the project header becomes ledger's
   `Segmented` (28 tall, 22 segments, mono 10.5, `activeFill`).
5. **Palette.** Move `meta` and `hintText` from `T.qml` into `palette.cpp` using
   brain's derivation (`hintText = mix(light_fg, muted, .25)`,
   `meta = mix(light_fg, muted, .5)`, plus `dimmest = mix(dark_fg, muted, .6)`),
   exposed as roles like every other. `T.onFill` and `T.hex` stay.
6. **Rail → sidebar.** Width **214** (was 236; 222 narrow variant goes). Header:
   "Planner" sans 15 / 600 at 16 / 16 with a mono 10.5 `muted` live line under it
   ("9 open · 2 overdue", derived in `App`); the wordmark and the `SUPER T` keycap
   move out (below). Nav rows **32** tall in an 8 px container, 9 side pad, 6 r: a
   bare mono 10.5 number in a 13 px slot (the key), the label sans 12.5, the count
   mono 10.5 right. **No view icons** (inbox, today, upcoming, pinned, completed
   lose their symbolic icons; no other app has them, and the number plus the
   name identify the view). The `Icon` provider stays for the detail pane's row
   markers, the repeat and pin glyphs and the empty-state glyph. Active row `activeFill` with `activeText`
   number and count and a 600 label (not `surface1` with an accent keycap); hover
   `header`. Project rows: the 7 px square dot in the number slot, subprojects at
   the 37 child indent. Add the 26-tall footer on `header` fill with the summon
   keyhint (`super+t summon`, still read from `hyprctl binds`, or `N selected`
   while selecting) left and `1–5` right, mono 10.5 `faint`; `ctrl+b rail` stays
   in the status bar.
7. **Header → toolbar.** 54 tall stays. Title sans 15 / 600 (was 20) with the
   view's query as the mono 10.5 `muted` subtitle under it (was beside it at 13).
   Right side: the three keycap + word hints at a 14 gap, then, for a project, the
   segmented list / board toggle. No search field and no primary button on this
   app; that is fine.
8. **Task rows.** **44** tall (was 45), 20 side pad, 12 gap. Title sans 13.5 (was
   15); meta mono 12.5 in `meta` colour (was 12); due column 104 wide mono 12.5
   (was 13). Cursor row `selection` fill with the revealed outlined keycaps; a
   selected row `selection` with the `caution` check; the cursor inside a
   selection keeps its `caution` outline. Lane headers are eyebrows with the count
   and hairline. Done rows: title `meta`, struck.
9. **Detail pane → rail.** Width **360** (was 372), `sidebar` fill, 1 px left
   border, 0 / 18 / 18 padding, groups 20 apart divided by hairlines. Top strip
   44 with the `TASK` eyebrow and the two keycaps. Title sans 15 / 600 (was 19).
   Description sans 12.5 `text2` at 1.5. Icon + value rows: mono 12.5 values, 12
   apart, the icon 8 from the value. Subtask rows 30 with the 18 check. Notes rows
   the same. Everything else in the pane (no field labels, live edits, label
   editing) is unchanged.
10. **Status line → status bar.** **26** tall (was 30), `header` fill (was
    `sidebar`), 1 px top border, mono 10.5 `faint` (was 12 `hintText`), 18 side
    pad. Left: the view name; middle: the count sentence or the toast text with
    "· ctrl+z undo" appended (unchanged behaviour); right: the keyhints separated
    by " · ". While selecting, the bulk-action keycaps keep their positive /
    negative colour as text and border.
11. **Prompts, picker, board, empty state.** Prompt popovers 8 r (was 12), `card`
    fill, `borderStrong`, one shadow `0 18 40 / .5`, over the one `rgba(0,0,0,.5)` scrim (replace the
    window-tinted .72 scrim and drop the .35 dim; nothing behind is dimmed); palette 660 at 64 from the
    top, add / find 610; the prompt field a boxed 32-tall field on `window` fill; result rows **30**
    (was 38) at 0 / 14 with sans 13.5 text, the highlighted row on `selection`; group eyebrows; footer 28 with keyhints 16 apart.
    Date picker 340: field 28, small quick buttons, mono 12.5 calendar days,
    today `positive`, selected `accent` on `accentText`, out-of-month
    `borderStrong`, time and repeat fields 28. Board columns on `sidebar` fill at
    **8 r** (was 10), eyebrow headers, 1 px `border` (`borderStrong` for the
    column holding the cursor); cards on **`card` fill** 8 r at 10 pad with a 1 px border (a step below their
    column, like every card; they were `window` and read lighter than their
    surround), 16 check; the cursor card `selection` fill with a 1 px `accent`
    outline. Empty state: the 44
    icon at .5, title sans 15 / 600 (was 18), sentence sans 12.5 `muted`, the
    keycap + word hint.
12. **Screenshots and docs.** `bin/grab docs/screens`; update `DECISIONS.md`
    ("Layout and chrome" now points at the spec) and the README's description of
    the window.

## Decisions to assume unless the owner says otherwise

- Bundle IBM Plex Sans (the handoff asked for system fonts).
- Task rows at 13.5 (the handoff's 15). If the owner wants 15 back, it is the one
  size to restore; everything else in this prompt stands.

## Verify

```sh
bin/build && (cd build && ctest --output-on-failure)
QT_FORCE_STDERR_LOGGING=1 QT_QPA_PLATFORM=offscreen timeout 4 ./build/planner --demo   # exit 124, empty stderr
./build/planner --info                                                                 # sans: IBM Plex Sans
bin/grab docs/screens
OMARCHY_TEXT_SCALE=1.3 bin/grab /tmp/scale
OMARCHY_THEME_DIR=/usr/share/omarchy/themes/catppuccin-latte bin/grab /tmp/light
grep -rhoE 'px: *[0-9.]+' src/app/qml | sort -u                                        # only the seven sizes
grep -rn 'surface0\|surface1' src/app/qml                                              # empty
cargo test --workspace                                                                 # untouched, still green
```

Compare each new grab with `~/Projects/design/screens/planner/<surface>.png`
(before) and with the sheet's L3 / L5 / L6 frames (target). Then, if `hypruse` is
available, launch the installed build: `1`–`5` between views, `↑↓`, `space`,
`enter` into the detail pane, `ctrl+n`, `ctrl+f`, `ctrl+k`, `ctrl+a` then `ctrl+1`,
`ctrl+shift+b` on Work, `ctrl+d`; every one must behave exactly as before, and
every row of controls (toolbar hints, the segmented toggle, the picker's quick
buttons and fields) must align at one height.

## Report

Finish with: the list of `px` values before and after, every place a size or
spacing changed that the spec did not anticipate (with the value you chose and
why), the DECISIONS.md entries, and the before / after grabs side by side for all
ten surfaces.

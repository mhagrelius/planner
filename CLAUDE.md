# Planner

Native Omarchy (Qt 6.11 / Qt Quick) task planner: the Quattro branch of the
GTK/Rust Planner on `main`, rewritten in C++ with the same JSON file and the
same two mini-languages (quick-add lines, filter queries). Design handoff in
`design/`; departures in `DECISIONS.md`. Scaffolded from the `omarchy-app-dev`
skill — read it before changing theming, scaling or packaging.

## Commands

- `bin/build` → `build/planner`. `cd build && ctest --output-on-failure` runs the
  core suites (headless, temp dirs only).
- `QT_FORCE_STDERR_LOGGING=1 QT_QPA_PLATFORM=offscreen timeout 4 ./build/planner --demo`
  must exit 124 with empty stderr. `./build/planner --info` prints theme and scale.
- `bin/grab [dir]` renders every surface headless at `OMARCHY_TEXT_SCALE=1` from the
  demo store pinned to 2026-09-05 (`docs/screens/` by default). One surface:
  `--screen <view> --act <state> --grab <png>`; states are `detail`, `palette`,
  `add`, `find`, `select`, `board`, `picker`, `norail`, `cursor:N`.
- `./build/planner --demo` seeds the design's tasks into `~/.local/share/planner-demo`
  (wiped each launch). `--data <path>` isolates a store; `--today YYYY-MM-DD` pins
  the clock. Scratch runs (`--demo`, `--data`, `--grab`) do not take the
  single-instance socket; `PLANNER_SOCKET_SUFFIX=-x` isolates one that should.
- `./build/planner agent help` — the CLI. Answered by the running window over the
  socket when there is one, else against the file directly. `planner sync
  now|status` likewise.
- Sync needs `~/.config/planner/config.json` with `sync_url` and `sync_token`;
  `server/README.md` is the server. `cargo test --workspace` runs the Rust core
  and server suites; the C++ core must keep reading what planner-core writes.
- Install: `cmake --install build --prefix ~/.local` (or `./install.sh`);
  `packaging/PKGBUILD` for pacman.

## Layout

- `src/core/` — plain C++ (QtCore only), no Qt Quick: `model` (records + JSON in
  the on-disk shape), `dates` (English dates, times, repeat phrases), `quickadd`,
  `store` (the only thing that reads or writes planner.json), `query` (the filter
  language), `search`, `schedule` (reminders), `present` (row strings, built-in
  views), `agent` (the CLI surface), `order` (fractional keys), `sync` (the
  three-snapshot planner, gather/apply), `config`, `demo`.
- `core/`, `server/` — the Rust `planner-core` and `planner-server` (Postgres);
  the container is built from here with `packaging/deploy-server.sh`.
- `src/app/` — `App` singleton (`app.cpp`: every surface as pre-formatted rows and
  one `changed()`, plus the sync passes), `remote` (planner-server over HTTP on a
  worker thread), `icons` (SVG recolouring image provider), `single` (the
  socket), `main`, and the QML in `qml/` (`T.qml` tokens, `components/`, `views/`).
  QML is a view over `App`; nothing in QML touches the store.
- `tests/` — one ctest executable per core area.

## Rules

- Colours only through `T.<role>` / `T.role(name)`; sizes only through `T.s()` /
  `T.f()`; anything numeric or date-like in mono, anything a person wrote in sans.
- The store is canonical: rows report, `App` mutates and saves. Saving is coalesced
  on a two-second tick through `QSaveFile`; the agent path saves at once.
- Every view is a query. Nothing below `App` reads the clock: `today` is an argument.
- A task's `notes` field exists in both cores; a change to the record shape goes
  into `core/src/task.rs` on `main` as well, or the GTK client drops it on sync.
- Surface-ramp colours (`T.surface0/1`) are for fills and hairlines, never copy.
- Record every departure from the handoff in `DECISIONS.md`.

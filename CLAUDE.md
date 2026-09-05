# Planner

Qt 6 / Qt Quick app for Omarchy, scaffolded from the `omarchy-app-dev` skill (read it before changing theming, scaling or packaging).

- Build: `bin/build` → `build/planner`. Install: `cmake --install build --prefix ~/.local`.
- Sizes only through `T.s()` (layout px) / `T.f()` (font pt); colours only through `T.<role>` / `T.role(name)` (derived in `src/palette.cpp` from colors.toml).
- App state and derived values live in `src/backend.*` (`App` singleton); QML is a view.
- Qt logs go to the journal unless `QT_FORCE_STDERR_LOGGING=1`. Headless check: `QT_QPA_PLATFORM=offscreen ./build/planner`.
- Preview a theme: `OMARCHY_THEME_DIR=/usr/share/omarchy/themes/<name>`.

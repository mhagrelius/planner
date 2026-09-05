#!/usr/bin/env sh
# Build and install into ~/.local (no root). The binary must be on the session
# PATH for the Omarchy menu to find it; ~/.local/bin is on Omarchy's.
set -eu
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
"$ROOT/bin/build"
cmake --install "$ROOT/build" --prefix "${PREFIX:-$HOME/.local}"
echo "installed to ${PREFIX:-$HOME/.local}; your tasks stay in ~/.local/share/planner/planner.json"

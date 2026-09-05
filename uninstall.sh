#!/usr/bin/env sh
# Remove what install.sh put in place. Your tasks are left alone.
set -eu
PREFIX="${PREFIX:-$HOME/.local}"
rm -f "$PREFIX/bin/planner" "$PREFIX/share/applications/planner.desktop" "$PREFIX/share/icons/hicolor/scalable/apps/planner.svg"
echo "removed planner from $PREFIX; ~/.local/share/planner/planner.json is untouched"

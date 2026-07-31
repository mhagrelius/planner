#!/usr/bin/env bash
#
# Remove everything install.sh put in place. Your tasks are left alone —
# they live in $XDG_DATA_HOME/planner and are not this script's to delete.

set -euo pipefail

APP_ID=us.hagreli.Planner
PREFIX="${PREFIX:-$HOME/.local}"

rm -fv "$PREFIX/bin/planner" \
       "$PREFIX/share/applications/$APP_ID.desktop" \
       "$PREFIX/share/metainfo/$APP_ID.metainfo.xml" \
       "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
       "$PREFIX/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
       "$PREFIX/share/dbus-1/services/$APP_ID.service"

if command -v gtk4-update-icon-cache >/dev/null; then
    gtk4-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" || true
fi

echo
echo "Removed. Your tasks are still in ${XDG_DATA_HOME:-$HOME/.local/share}/planner."

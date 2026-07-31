#!/usr/bin/env bash
#
# Build and install the Flatpak for the current user.
#
# The one fiddly part is cargo-sources.json: Flathub builds with no network,
# so every crate in Cargo.lock has to be listed as a source with a checksum.
# flatpak-cargo-generator.py turns the lockfile into that list; this fetches
# it on first run and regenerates whenever Cargo.lock is newer.

set -euo pipefail
cd "$(dirname "$0")/.."

MANIFEST=packaging/flatpak/us.hagreli.Planner.yml
SOURCES=packaging/flatpak/cargo-sources.json
GENERATOR=.flatpak-build/flatpak-cargo-generator.py
RUNTIME_VERSION=50

command -v flatpak >/dev/null || { echo "flatpak is not installed" >&2; exit 1; }
command -v flatpak-builder >/dev/null || {
    echo "flatpak-builder is not installed" >&2; exit 1; }

echo "==> Runtime"
flatpak install --user --noninteractive --or-update flathub \
    "org.gnome.Platform//$RUNTIME_VERSION" \
    "org.gnome.Sdk//$RUNTIME_VERSION" \
    org.freedesktop.Sdk.Extension.rust-stable//24.08

if [[ ! -f "$GENERATOR" ]]; then
    echo "==> Fetching flatpak-cargo-generator"
    mkdir -p "$(dirname "$GENERATOR")"
    curl -fsSL -o "$GENERATOR" \
        https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
fi

if [[ ! -f "$SOURCES" || Cargo.lock -nt "$SOURCES" ]]; then
    echo "==> Generating $SOURCES from Cargo.lock"
    python3 "$GENERATOR" Cargo.lock -o "$SOURCES"
fi

echo "==> Building"
flatpak-builder --force-clean --user --install \
    .flatpak-build/build "$MANIFEST"

echo
echo "Installed. Run: flatpak run us.hagreli.Planner"

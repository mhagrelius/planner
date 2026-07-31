#!/usr/bin/env bash
#
# Build a .deb into dist/. Hand-rolled rather than debhelper: this is one
# binary and five data files, and dpkg-deb will assemble that directly.
#
# Needs: dpkg-deb, dpkg-shlibdeps, fakeroot.

set -euo pipefail
cd "$(dirname "$0")/.."

APP_ID=us.hagreli.Planner
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(dpkg --print-architecture)
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

for tool in dpkg-deb dpkg-shlibdeps fakeroot; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

echo "==> Building planner $VERSION for $ARCH"
cargo build --release --locked

install -Dm755 target/release/planner "$STAGE/usr/bin/planner"
install -Dm644 "data/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "data/$APP_ID.metainfo.xml" "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
install -Dm644 "data/icons/hicolor/scalable/apps/$APP_ID.svg" \
    "$STAGE/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "data/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
    "$STAGE/usr/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
install -Dm644 packaging/deb/copyright "$STAGE/usr/share/doc/planner/copyright"

install -Dm644 /dev/stdin "$STAGE/usr/share/dbus-1/services/$APP_ID.service" <<EOF
[D-BUS Service]
Name=$APP_ID
Exec=/usr/bin/planner --gapplication-service
EOF

# Let dpkg work out the library dependencies from the binary itself, rather
# than a hand-written list that goes stale the next time GTK moves.
#
# dpkg-shlibdeps insists on a debian/control relative to the working
# directory, even with -O, so it gets a minimal one. Without it the command
# fails and the fallback list below silently takes over — which is how a
# package ends up claiming to need GTK 4.16 long after it needs 4.22.
mkdir -p "$STAGE/DEBIAN" "$STAGE/debian"
cat > "$STAGE/debian/control" <<EOF
Source: planner

Package: planner
Architecture: $ARCH
EOF

DEPENDS=$(
    cd "$STAGE" &&
    dpkg-shlibdeps -O --ignore-missing-info usr/bin/planner 2>/dev/null |
    sed -n 's/^shlibs:Depends=//p'
)
rm -rf "$STAGE/debian"

if [[ -z "$DEPENDS" ]]; then
    echo "warning: dpkg-shlibdeps found nothing; falling back to a fixed list" >&2
    DEPENDS="libgtk-4-1 (>= 4.16), libadwaita-1-0 (>= 1.6)"
fi

sed -e "s/@VERSION@/$VERSION/" -e "s/@ARCH@/$ARCH/" -e "s|@DEPENDS@|$DEPENDS|" \
    packaging/deb/control.in > "$STAGE/DEBIAN/control"
install -m755 packaging/deb/postinst "$STAGE/DEBIAN/postinst"
install -m755 packaging/deb/postrm "$STAGE/DEBIAN/postrm"

mkdir -p dist
OUT="dist/planner_${VERSION}_${ARCH}.deb"
fakeroot dpkg-deb --build "$STAGE" "$OUT" >/dev/null

echo "==> $OUT"
dpkg-deb --info "$OUT" | sed -n 's/^ /  /p' | head -12

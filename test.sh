#!/usr/bin/env bash
#
# Everything CI would run, in the order that fails fastest.
#
#   ./test.sh              against your own session
#   ./test.sh --headless   under Xvfb and a private D-Bus session
#
# The headless mode exists for the UI tests: GTK needs a display, and a test
# run must not attach to the developer's real session bus, where it would talk
# to a live instance of the app instead of itself.

set -euo pipefail

cd "$(dirname "$0")"

headless=false
if [[ "${1:-}" == "--headless" ]]; then
    headless=true
    shift
fi

# Accessibility bridges and the GSettings backend both reach out to session
# services that may not exist. Neither is under test.
export GTK_A11Y=none
export GSETTINGS_BACKEND=memory
export RUST_BACKTRACE=1

run() {
    echo "==> $*"
    if $headless; then
        xvfb-run -a dbus-run-session -- "$@"
    else
        "$@"
    fi
}

# Formatting and lints need no display, so they never go through the wrapper.
echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

# --workspace so planner-core is tested too. Without it cargo checks only the
# root package, and the half of the suite that needs no display — which is most
# of it — silently stops running.
run cargo test --workspace --all-targets

echo
echo "All green."

#!/usr/bin/env bash
#
# Prove a sync pass against a real server, with two machines.
#
#     ./sync-check.sh
#
# **A sync that has never been contradicted has not been tested.** Everything
# in `core/src/sync.rs` is a pure function with unit tests, and everything in
# `server/` has its own — but between them sit a wire format, a transport, and
# the question of whether the two ends agree about what a record is. One
# machine can never ask that question, and there is no second platform coming
# along to shake it out by accident.
#
# So: two stores, one throwaway database, the real server binary, the real
# client code path. Needs the Postgres in server/.env reachable; it creates and
# drops a database of its own and touches nothing else.

set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! -f server/.env ]]; then
    echo "server/.env is missing — copy server/.env.example and fill it in" >&2
    exit 1
fi
set -a; . ./server/.env; set +a

WORK="$(mktemp -d)"
DB="planner_synccheck_$$"
PORT="${SYNC_CHECK_PORT:-8098}"
TOKEN="0123456789abcdef0123456789abcdef0123456789abcdef"
PID=""

psql_as() {
    local user="$1" password="$2" database="$3"
    shift 3
    PGPASSWORD="$password" podman run --rm -i --network=host -e PGPASSWORD \
        docker.io/library/postgres:18-alpine psql -q -v ON_ERROR_STOP=1 \
        -h "$PLANNER_DB_HOST" -p "$PLANNER_DB_PORT" -U "$user" -d "$database" "$@"
}

cleanup() {
    local code=$?
    [[ -n "$PID" ]] && kill "$PID" 2>/dev/null || :
    # The throwaway database goes whatever happened, so a failed run does not
    # leave one behind on a NAS nobody is looking at.
    psql_as "$PLANNER_DB_SUPERUSER" "$PLANNER_DB_SUPERUSER_PASSWORD" default \
        -c "DROP DATABASE IF EXISTS $DB" >/dev/null 2>&1 || :
    rm -rf "$WORK"
    exit $code
}
trap cleanup EXIT

fail() { echo "  FAIL $1" >&2; exit 1; }
pass() { echo "  ok   $1"; }

echo "==> building"
cargo build -q -p planner-server

echo "==> a database of its own ($DB)"
psql_as "$PLANNER_DB_SUPERUSER" "$PLANNER_DB_SUPERUSER_PASSWORD" default \
    -c "DROP DATABASE IF EXISTS $DB" -c "CREATE DATABASE $DB OWNER $PLANNER_DB_USER" >/dev/null
# Piped rather than -f: psql runs inside a container, where a repo path means
# nothing.
psql_as "$PLANNER_DB_USER" "$PLANNER_DB_PASSWORD" "$DB" < server/migrations/0002-records.sql >/dev/null

echo "==> starting a throwaway server on $PORT"
PLANNER_TOKEN="$TOKEN" PLANNER_ADDR="127.0.0.1:$PORT" \
PLANNER_DB_HOST="$PLANNER_DB_HOST" PLANNER_DB_PORT="$PLANNER_DB_PORT" \
PLANNER_DB_NAME="$DB" PLANNER_DB_USER="$PLANNER_DB_USER" \
PLANNER_DB_PASSWORD="$PLANNER_DB_PASSWORD" \
    ./target/debug/planner-server > "$WORK/server.log" 2>&1 &
PID=$!

for _ in $(seq 1 40); do
    curl -sf --max-time 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break
    sleep 0.25
done
curl -sf --max-time 2 "http://127.0.0.1:$PORT/health" >/dev/null \
    || { cat "$WORK/server.log" >&2; fail "the server never came up"; }

# Two machines: two XDG_DATA_HOMEs, so two documents and two sync bases.
export SYNC_CHECK_URL="http://127.0.0.1:$PORT"
export SYNC_CHECK_TOKEN="$TOKEN"
export SYNC_CHECK_A="$WORK/a"
export SYNC_CHECK_B="$WORK/b"
mkdir -p "$SYNC_CHECK_A" "$SYNC_CHECK_B"

echo "==> two machines, one server"
if cargo run -q --example sync-check; then
    echo
    echo "All green."
else
    echo
    echo "--- server log ---" >&2
    cat "$WORK/server.log" >&2
    exit 1
fi

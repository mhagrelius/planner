-- The records the server arbitrates over.
--
--     psql -h "$PLANNER_DB_HOST" -p "$PLANNER_DB_PORT" \
--          -U "$PLANNER_DB_USER" -d "$PLANNER_DB_NAME" \
--          -f server/migrations/0002-records.sql
--
-- Run as the planner role rather than the superuser, so everything here is
-- owned by the role that will be reading and writing it.
--
-- One table, not one per kind. The five kinds differ only in what is inside
-- `body`, which the server never looks at — it stores and arbitrates, and
-- every question it can answer is about `kind`, `id` and the timestamps. Five
-- tables would be five copies of the same three statements.

CREATE TABLE IF NOT EXISTS records (
    kind        text        NOT NULL,
    id          text        NOT NULL,

    -- The client's `updated_at` for this record, which is also its version:
    -- a write whose version is not newer than the stored one is refused. This
    -- is the whole reason the server is Postgres and not a directory of files
    -- — `WHERE records.updated_at < excluded.updated_at` on an upsert is one
    -- atomic statement, where a filesystem would need a lock and a
    -- read-modify-write that two clients can interleave.
    updated_at  timestamptz NOT NULL,

    -- Set when the record has been deleted. The row stays: a machine that has
    -- been switched off has to be able to tell "deleted" from "never seen",
    -- and a missing row says the second.
    deleted_at  timestamptz,

    -- The record as the client serialises it. Null once deleted, because
    -- keeping the contents of a deleted task on a NAS is not something anyone
    -- asked for.
    body        jsonb,

    PRIMARY KEY (kind, id)
);

-- What a pass actually asks: everything that changed since the version this
-- client last agreed on. Without this it is a sequential scan per sync, per
-- machine, forever.
CREATE INDEX IF NOT EXISTS records_changed_at
    ON records (greatest(updated_at, coalesce(deleted_at, updated_at)));

-- Give planner-server its own role and database on the shared Postgres.
--
-- The instance on the NAS also holds whatever else uses it, so this deliberately
-- does not touch the superuser's `default` database or grant anything wider
-- than planner-server needs. The role owns exactly one database and PUBLIC can
-- neither connect to it nor see it.
--
-- The password is passed in rather than written here, so this file carries no
-- secret and can be read by anyone:
--
--     psql -h nas.example.ts.net -p 5433 -U postgres -d default \
--          -v planner_password="$PLANNER_DB_PASSWORD" \
--          -f server/migrations/0001-init.sql
--
-- Re-running it is a no-op rather than an error.

-- CREATE ROLE has no IF NOT EXISTS, so ask first. \gexec runs the text a query
-- returns, so a query matching no rows runs nothing at all.
--
-- This is not a DO block, and cannot be one: psql substitutes :variables in
-- the text it sends, but not inside a dollar-quoted string, which is all a DO
-- block's body is. The password would arrive as the literal characters
-- `:'planner_password'`.
SELECT 'CREATE ROLE planner LOGIN'
WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'planner')
\gexec

-- Separately, and unconditionally, so that re-running this is how the password
-- gets rotated. %L quotes it, so a password containing a quote cannot end the
-- statement early.
SELECT format('ALTER ROLE planner PASSWORD %L', :'planner_password')
\gexec

-- CREATE DATABASE has no IF NOT EXISTS either, and additionally cannot run
-- inside a transaction — so it gets the same treatment.
SELECT 'CREATE DATABASE planner OWNER planner'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'planner')
\gexec

-- Every role on this instance can connect to every database by default. This
-- one holds a task list, and the other services sharing the instance have no
-- business in it.
REVOKE ALL ON DATABASE planner FROM PUBLIC;
GRANT CONNECT ON DATABASE planner TO planner;

-- The rest has to happen inside the new database rather than beside it.
\connect planner

-- Postgres 15 and later already stop PUBLIC writing to `public`, and the
-- schema's owner is pg_database_owner, which is planner here — so the role can
-- create its tables without an explicit grant. This only closes the read side.
REVOKE ALL ON SCHEMA public FROM PUBLIC;

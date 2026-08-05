# planner-server

One task list, shared between one person's machines. It stores records and
decides who wins; it is never the only copy.

## What it is not

It does not evaluate a filter query, compute a recurrence, parse a quick-add
line or decide what is due today. All of that is the client's, where it already
is and where it is already tested. A server that starts answering "what is in
Today" is a second planner that can disagree with the first.

## Routes

| | |
|---|---|
| `GET /health` | Is it up. No token, so the container healthcheck needs no secret. |
| `GET /snapshot` | Every record's kind, id and version. No bodies. |
| `POST /records` | Store these, refusing any that are not newer. |
| `POST /deletions` | Mark these gone, on the same terms. |

Everything but `/health` needs `Authorization: Bearer <PLANNER_TOKEN>`.

**A write whose version is not newer than the stored one is refused**, and the
response says which. That one rule is why this is Postgres and not a directory
of files: the refusal has to be atomic with the write, or two machines syncing
at once read the same version, both decide theirs is newer, and both write.

## Setting it up

Once, from a workstation, with `server/.env` filled in from `.env.example`:

```sh
# The role and the database, as the Postgres superuser.
psql -h "$PLANNER_DB_HOST" -p "$PLANNER_DB_PORT" -U "$PLANNER_DB_SUPERUSER" -d default \
     -v planner_password="$PLANNER_DB_PASSWORD" \
     -f server/migrations/0001-init.sql

# The table, as the planner role.
psql -h "$PLANNER_DB_HOST" -p "$PLANNER_DB_PORT" -U "$PLANNER_DB_USER" -d "$PLANNER_DB_NAME" \
     -f server/migrations/0002-records.sql
```

Then build and push the image:

```sh
./packaging/deploy-server.sh
```

And on the NAS: Container Manager → Project → Create, name it `planner-server`,
path `/volume1/docker/planner-server`, source **Upload docker-compose.yml**,
and upload `server/docker-compose.yml`.

**Uncheck "Start the project once it is created."** The compose file requires
`PLANNER_TOKEN` and `PLANNER_DB_PASSWORD`, and they arrive in a `.env` that does
not exist yet — starting first just fails. Upload `.env` into the project folder
through File Station, then Action → Build.

**Do not type the compose file into Container Manager's editor.** It carries the
previous line's indentation onto the next and adds what you type to it, so six
lines in everything is nested under everything else. The `YAML Configurations`
tab on an existing project is read-only, including when it is stopped.

## Checking it, rather than believing the dot

```sh
curl -s http://nas.example.ts.net:8083/health
curl -s -H "Authorization: Bearer $PLANNER_SERVER_TOKEN" \
     http://nas.example.ts.net:8083/snapshot
```

The status dot lies in both directions — amber for a missing `curl`, green for a
container that cannot write. And **a blank Log tab means nothing**: when a
container exits, go straight to `sudo docker logs planner-server` over SSH.

## Two things learned the hard way here

**The image name in the compose file is `localhost:5050/…`,** even though the
push went to the NAS's tailnet name. A registry stores repositories by name
rather than by hostname, so it is the same image, and Docker accepts a registry
reached over localhost on plain HTTP without being configured to. Naming the NAS
means editing the daemon config over SSH for no gain.

**`PLANNER_BIND` cannot be the NAS's Tailscale address.** Tailscale runs there
as a DSM package in userspace, so `100.x.y.z` is on no local interface and
Docker refuses the container with `bind: cannot assign requested address`. It
listens on every interface instead, which is the LAN and the tailnet and nothing
else, because nothing is forwarded on the router. The token crosses the LAN in
clear text; on this network that is the accepted trade, and it is the same one
brain-server makes.

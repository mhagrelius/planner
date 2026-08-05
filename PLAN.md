# Planner with a server — plan

One task list, held on the NAS, with clients keeping full local copies. The
scope here is the server and the GTK shell that talks to it, finished and
running on the NAS against Postgres. Shells on macOS and Windows are the reason
this is being built and are not part of it; see **Later** at the end for what
is being kept open for them and what is being deliberately left undecided.

This follows the path `brain` took and departs from it in two places, both
because the data is a different shape. `brain`'s `PLAN.md` is the reference for
anything not restated here.

Nothing below is built yet.

## What does not change

**The store stays canonical and local.** Every machine keeps the whole document
in memory and writes it to `~/.local/share/planner/planner.json` on the save
tick, exactly as now. The server holds a copy and decides who wins; it is never
the only copy and nothing on screen waits for it. A planner that will not open
a task list because a NAS is down is worse than one that does not sync.

**One place mutates.** `PlannerApplication::mutate` is the only thing that
touches the store today and that survives verbatim — a sync pass becomes
another caller of it, not a second writer.

## What the server is not

It stores records and arbitrates between writers. That is the whole job.

It does **not** evaluate filter queries, compute recurrences, parse quick-add
lines, resolve labels or decide what is due today. All of that stays in the
client, where it already is and where it is already tested. A server that
starts answering "what is in Today" is a second planner that can disagree with
the first, and keeping it dumb is what stops the two implementations drifting.

The one thing it must share with the client is the record schema, which is why
it links `planner-core` rather than restating the types.

## Why the core comes out

`brain` extracted a core because policy had collected inside a `glib::wrapper!`
and its tests needed a display. Planner does not have that problem:
`src/model/` is 10,439 lines with zero GTK references, and `ui/application.rs`
is forty five-to-fifteen-line wrappers around `self.mutate(|store| store.x(…))`.
The rules are already below the line, pushed there by the agent CLI being a
second consumer.

So the split earns its place here for exactly one reason: **`planner-server`
needs the record types and the merge rules, and cannot link libadwaita to get
them.** That is enough on its own. The Swift and C# shells would want the same
thing later, but they are not the argument today.

## The part that is not like `brain`

`brain` syncs files. A note has an id, its own bytes and its own hash, so a
snapshot is a map of id to hash and a conflict is a note written beside the
original. Planner has **one JSON document**, and that changes three things.

**Syncing the document whole is not an option.** Two machines both open means
last-writer-wins over everything, silently. This is also why "just point
Syncthing at it" — the right answer for stickies, where every note is its own
file — is actively wrong here.

**So sync is per record.** The document decomposes into records that already
have stable ids and, for tasks, an `updated_at`: tasks, projects, sections,
labels, saved filters. Each syncs independently. Phase 1 makes that true of the
kinds where it is not yet.

**And a conflict is resolved, not preserved.** `brain` writes a conflict copy
because a note is prose somebody wrote and losing a paragraph is unrecoverable.
A task record is a dozen scalar fields, and record-level last-writer-wins by
`updated_at` costs at worst a priority set twice in the same minute on two
machines. That is a re-do, not a loss. **Last-writer-wins per record, and no
conflict copies.** The honest cost, stated so it is not a surprise: editing the
same task's `description` on two machines in one pass loses one of them, and
adding a different label to the same task on each machine keeps one set rather
than the union. If either bites in practice, field-level merge is the next step
— measured first, the way `brain` refused to buy Automerge against a conflict
rate nobody had counted.

**Deletion is the exception, and it is a rule rather than a question.** A
delete never beats an edit, for the reason it does not in `brain`: losing a
task is the one unrecoverable failure here.

## Phases

### 0. Workspace split

`planner-core` in `core/`, holding today's `src/model/` unchanged. Root package
stays the GTK shell with `pub use planner_core as model;` in `src/lib.rs`, so
every call site reads as it does now. Mechanical, reversible, no behaviour
change, its own commit.

Check whether the core needs `chrono`'s `clock` feature at all — nothing in
`model/` reads the clock today, every function takes `now` and `today` as
arguments, and keeping it that way is worth preserving deliberately.

`test.sh` gains `--workspace` on both the clippy and the test line. Without it
cargo checks only the root package and the entire core stops running.

`model/agent/` moves across with the rest. It is a presentation layer rather
than core, and that is worth remembering later, but it is GTK-free and nothing
in this scope is improved by moving it.

### 1. Make the document syncable

Pure core work, no network and no display, and it must land before anything
touches the wire: it is a format change, and doing it afterwards means
migrating every machine at once.

**Tombstones.** `DESIGN.md:12` claims stable ids, `updated_at` and tombstones
all exist from day one "so a sync source can be added later without a
migration". Two of three. `Task::touch` maintains `updated_at` properly, but
deletion is a hard `Vec::retain` — `remove_task` returns `Vec<Task>` and
`remove_project` returns `RemovedProject`, held in memory for the undo toast
and then dropped. Without a tombstone the server cannot tell "A deleted this"
from "B created this and A has not seen it yet", so deleted tasks resurrect on
every pass. Deletion becomes a `deleted_at` mark; the cascades in `remove_task`
and `remove_project` mark every descendant individually, or the cascade will
not replay on the other machine; `restore_tasks` and `restore_project` clear
the mark, so undo keeps working; every read path filters marked records; and
tombstones older than ninety days are dropped at load.

**`order` becomes a fractional key.** `store.rs:759` already saw this coming —
`renumber` deliberately leaves `updated_at` alone because bumping it on every
task in a list "would send the whole list" once there is a sync source. That
instinct is right, and the consequence is that a reorder currently propagates
to nobody. Neither integer answer works: bumping `updated_at` sends the list
and destroys "recently changed", and not bumping it means two machines
reordering the same list merge into interleaved positions that are not a valid
ordering at all. A key that sorts lexicographically and is generated between
its two neighbours fixes both — a move rewrites exactly one record, whose
`updated_at` then honestly changed, and concurrent moves give an arbitrary but
consistent order rather than a broken one. `renumber` and its two call sites in
`move_task` mostly disappear.

**Sections become top-level records.** A `Section` currently lives inside
`Project::sections`, so a section added on one machine and one added on another
are two edits to the same project record and last-writer-wins eats one. Given a
`project_id` and an id of its own it syncs like everything else, and the five
record kinds become uniform — which the wire format and the Postgres schema
both want.

**Bump `SCHEMA_VERSION`.** The protection this needs is already built: a file
from a newer version opens read-only and is never overwritten
(`store.rs:170`), so a machine still on the old build degrades to read-only
rather than destroying the new format. That was speculative when it was
written and this is the day it pays.

### 2. `core/src/sync.rs` — the pure planner

Take what this machine holds, what the server holds, and what the two agreed on
last pass; return the work. A pure function with its own tests, no network, no
clock, no display — the shape `brain`'s `sync::plan` established.

**Three snapshots, not two.** Local against remote says they differ but never
which one moved, and pushing when you should have pulled is how a sync loses
work. Local against base says whether *this* machine changed something; remote
against base says whether another one did. The base is per-machine, beside the
document at `~/.local/share/planner/sync-base.json`.

A snapshot is id → `updated_at` per record kind, plus the tombstone flag.
Records are small and versioning is by timestamp, so none of `brain`'s content
hashing is needed.

The plan comes back as push / pull / delete-local / delete-remote per kind, and
**an empty plan when nothing changed is worth asserting on** — a plan that is
never empty means something re-uploads the list on a timer.

### 3. `planner-server`, push-only

Planner has no derived artifact to rehearse on — no embeddings, no
transcriptions — so `brain`'s "share the cheap thing before the source of
truth" needs a different cut. It is this: **the first deployment pushes and
never applies.** A client that only pushes cannot lose local data no matter how
wrong the server is; the worst case is a stale copy on the NAS, which costs a
backup rather than work. That proves the container, the tailnet path, the
token, the Postgres schema and the client's tolerance of a server that is not
there, against a failure that cannot hurt.

**Storage is Postgres, and this departs from `brain` deliberately.** `brain`'s
server stores real Markdown files because the vault *is* the product —
browsable in File Station, `git init` for history, readable by `cat`. None of
that transfers: nobody is going to hand-edit a task record on a NAS. What this
needs instead is an atomic compare-and-set per record so a stale write is
refused rather than applied, and that is one `UPDATE … WHERE updated_at =
$expected` instead of a lock file and a read-modify-write race invented on top
of a directory. The instance is already running and reachable over the tailnet:
`postgres:18-alpine` in the `postgres` project, **published on 5433** — the
container's own 5432 is mapped up one, so `5432` on the NAS is something else
entirely and connecting to it will not fail in any useful way.

That is one new dependency — the sync `postgres` crate, no tokio — and the
justification is the paragraph above rather than familiarity. It gets its own
database and its own role, not a shared one, and its own migrations checked
into `server/migrations/`. `0001-init.sql` exists already: it creates the role
and the database, takes the password as a psql variable so the file carries no
secret, and revokes `PUBLIC` connect on a database sharing an instance with
several other services.

Follow `brain-server` in everything else: no HTTP crate for a handful of routes
on a private network, a `--health` flag on the binary because the image has no
`curl`, `user: "0:0"` because `/volume1`'s ACLs beat `chown`, `read_only: true`
doing the real hardening, bound to the NAS's Tailscale address so the bearer
token stays inside WireGuard, and `localhost:5050/planner-server:<date>` in the
compose file even though the push goes to the tailnet name.

**Port 8083** — checked free on 2026-08-04, alongside 5002 (DSM), 5050
(registry), 8081 (llama-embed) and 8082 (brain-server), which are not.

`.dockerignore` `target/` and `.git/` **before the first build**, because the
build context becomes the workspace root the moment `planner-server` depends on
`planner-core` by path. `brain` shipped eleven gigabytes finding this out.

Sync stays off until a URL and a token are put in the config on purpose.

### 4. Two-way sync in the GTK shell

Turn on apply, on a timer, on a worker thread.

**The worker/main split is easier here than in `brain`, and only if it is kept
that way.** `brain` had to divide `gather` from `apply` because a pull writes
files the save tick is also writing. Planner's document is held whole in memory
and written by one tick, so the rule is simpler and stricter: **the worker does
network and nothing else.** It is handed a snapshot and gives back records; the
main thread merges them through `PlannerApplication::mutate` and lets the
existing `dirty` flag and tick do the write. No sync code opens `planner.json`.
Break that and two writers race over the whole document rather than over one
note.

The task open in the detail panel is the case to get right, and it is the same
judgement the panel already makes: a pull landing on the task being edited is
held until it closes rather than yanking the text out from under the cursor.

A pass applies deletes last, so a failure part-way leaves records present
rather than gone, and **the base saved afterwards records what happened, not
what was planned** — every failed transfer stays out, so a pass that dies half
way retries instead of believing itself.

UI is awareness, not arbitration: nothing at all on a clean pass, and
`set_save_error`'s banner reused for a server that has been unreachable long
enough to matter. `window.rs:1648` already owns that surface, and an active
save failure outranks a sync problem, because that is data not being written
right now.

### 5. `./sync-check.sh`

**With one shell on one machine, sync has one participant, and a sync that has
never been contradicted has not been tested.** `brain` solved this with a
script that starts a throwaway server, drives it with the real client, and
watches two vaults push, pull and reconcile. Planner needs the same, and needs
it more, because there is no second platform coming along to shake the bugs out
by accident.

Two `XDG_DATA_HOME` directories, one throwaway server against a scratch
database, the real client binary, and assertions on the end state: a task added
on A arrives at B, a delete on A survives a pull on B, a task edited on both
resolves to one winner without either copy vanishing, and a machine that was
"off" for several passes catches up in one.

This is the phase that says whether the previous four were right, so it is not
optional and it is not last because it matters least.

## Out of scope, and worth saying so

**Reminders still need the app running.** Sync makes the task list agree
everywhere; it does not fire a notification on a machine that is switched off,
and a server that could is a push-notification story needing a client that does
not exist. `schedule.rs` is untouched by all of the above.

## Later

Not being built now, recorded so the reasoning is not lost.

**macOS and Windows shells.** Native, not GTK — `brain`'s `PLAN.md` sets out
why, and none of it has improved. Phase 0 is what makes them possible; nothing
else here should assume them.

**The FFI boundary is deliberately not designed.** Designing one against an
imaginary consumer is how you get the wrong one. When it is time, planner has a
second route `brain` lacks: `model/agent/` is already a headless, tested
JSON-in/JSON-out command surface with a `describe` verb that emits its own
vocabulary, so a shell could drive the core through that rather than through
generated bindings — language-agnostic, already built, and it takes
`uniffi-bindgen-cs`, which `brain` names as its riskiest untried leg, off the
critical path. UniFFI with proc-macros is the other route and the well-trodden
one. Decide against a real shell.

## Then update `DESIGN.md`

`DESIGN.md` says v1 is local-only and calls sync "deliberately not in v1", and
that stays the record of what was designed. Where this contradicts it, the
difference belongs in that file's **Built differently, or not built** section
once it is real — including the tombstone correction from phase 1, which is
wrong today regardless of whether any of this gets built.

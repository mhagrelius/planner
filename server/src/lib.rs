//! One task list, shared between one person's machines.
//!
//! The client keeps the whole document in memory and writes it to its own
//! file; this holds a copy and decides who wins. It is never the only copy,
//! and nothing on a screen waits for it — a planner that will not open because
//! a NAS is down is worse than one that does not sync.
//!
//! Four routes:
//!
//! | | |
//! |---|---|
//! | `GET /health` | Is it up. No token, so a container healthcheck needs no secret. |
//! | `GET /snapshot` | Every record's kind, id and version. No bodies. |
//! | `POST /fetch` | The bodies of the records named in the request. |
//! | `POST /records` | Store these, refusing any that are not newer. |
//! | `POST /deletions` | Mark these gone, on the same terms. |

pub mod http;
pub mod records;

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use planner_core::tombstone::RecordKind;
use serde::{Deserialize, Serialize};

use http::{Request, Response};
use records::Entry;

/// A record on its way in.
#[derive(Debug, Clone, Deserialize)]
pub struct Incoming {
    pub kind: RecordKind,
    pub id: String,
    pub updated_at: DateTime<Utc>,
    /// Absent for a deletion, which is what `/deletions` is for.
    #[serde(default)]
    pub body: Option<serde_json::Value>,
}

/// A record a client is asking for by name.
#[derive(Debug, Clone, Deserialize)]
pub struct Wanted {
    pub kind: RecordKind,
    pub id: String,
}

/// What a write did, record by record.
#[derive(Debug, Clone, Serialize)]
pub struct WriteReport {
    pub applied: usize,
    pub stale: usize,
    /// The ones that were refused, so a client can say which without diffing
    /// two snapshots.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stale_ids: Vec<String>,
}

/// How long a waiting request is held before it is answered anyway.
///
/// Long enough that an idle client is not reconnecting constantly, short
/// enough that no NAT table, proxy or sleeping Wi-Fi radio between here and a
/// laptop decides a silent connection is a dead one. A client that gets
/// "nothing changed" simply asks again.
pub const MAX_WAIT: Duration = Duration::from_secs(50);

/// What a parked request is woken by.
///
/// A counter rather than a flag: a change that lands between a client's read
/// and its wait would set a flag that had already been cleared, and the client
/// would sleep through it. Comparing counters cannot miss one.
///
/// This exists so a waiting request does *not* hold a database connection. One
/// dedicated connection runs `LISTEN` and bumps this; every waiter blocks on a
/// condvar, which costs a parked thread and nothing else.
#[derive(Default)]
pub struct Changes {
    seen: Mutex<u64>,
    signal: Condvar,
}

impl Changes {
    /// Something changed. Wake everyone waiting.
    pub fn announce(&self) {
        let mut seen = match self.seen.lock() {
            Ok(seen) => seen,
            Err(poisoned) => poisoned.into_inner(),
        };
        *seen = seen.wrapping_add(1);
        self.signal.notify_all();
    }

    /// The count to compare against later.
    pub fn mark(&self) -> u64 {
        match self.seen.lock() {
            Ok(seen) => *seen,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    /// Wait for the count to move past `mark`, or for the time to run out.
    pub fn wait_past(&self, mark: u64, timeout: Duration) {
        let seen = match self.seen.lock() {
            Ok(seen) => seen,
            Err(poisoned) => poisoned.into_inner(),
        };
        // The predicate is re-checked on every wake, so a spurious one costs a
        // loop rather than a missed change.
        let _ = self
            .signal
            .wait_timeout_while(seen, timeout, |seen| *seen == mark);
    }
}

/// What a waiting client is told.
#[derive(Debug, Clone, Serialize)]
pub struct Changed {
    /// Whether anything moved. False means the wait simply timed out.
    pub changed: bool,
    /// The newest version the server holds. The client sends this back as
    /// `since` next time, so the cursor is the server's to define and a client
    /// cannot drift by keeping its own.
    pub now: DateTime<Utc>,
}

/// Anything the server needs to answer a request.
pub struct Server {
    /// One connection behind a lock. This serves one person's three machines
    /// on a timer, so contention is a few requests a minute — a pool would be
    /// machinery for load that does not exist. A thread per connection still
    /// means requests overlap; they queue here, briefly.
    client: Mutex<postgres::Client>,
    token: String,
    changes: Arc<Changes>,
}

impl Server {
    pub fn new(client: postgres::Client, token: String, changes: Arc<Changes>) -> Self {
        Self {
            client: Mutex::new(client),
            token,
            changes,
        }
    }

    /// Answer one request.
    pub fn handle(&self, request: &Request) -> Response {
        // Health first, and before the token check: a container healthcheck
        // that needed the secret would mean handing it to Docker as well.
        if request.path == "/health" {
            return match request.method.as_str() {
                "GET" => Response::text(200, "ok"),
                _ => Response::text(405, "GET /health"),
            };
        }

        if request.bearer() != Some(self.token.as_str()) {
            // Deliberately says nothing about which part was wrong.
            return Response::text(401, "unauthorized");
        }

        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/snapshot") => self.snapshot(),
            ("GET", path) if path.starts_with("/changes") => self.changes(path),
            ("POST", "/fetch") => self.fetch(request),
            ("POST", "/records") => self.write(request, false),
            ("POST", "/deletions") => self.write(request, true),
            ("GET", _) | ("POST", _) => Response::text(404, "no such route"),
            _ => Response::text(405, "GET or POST"),
        }
    }

    fn snapshot(&self) -> Response {
        let mut client = match self.client.lock() {
            Ok(client) => client,
            Err(poisoned) => poisoned.into_inner(),
        };

        let rows = match client.query(records::SNAPSHOT, &[]) {
            Ok(rows) => rows,
            Err(error) => return Response::text(503, &format!("database: {error}")),
        };

        let entries: Vec<Entry> = rows
            .iter()
            .filter_map(|row| {
                let name: String = row.get(0);
                // A row under a kind this build does not know is left out
                // rather than guessed at. It can only come from a newer server
                // writing into the same database, and inventing a kind for it
                // would put a record the client cannot read into its store.
                let kind = records::kind_from_name(&name)?;
                Some(Entry {
                    kind,
                    id: row.get(1),
                    updated_at: row.get(2),
                    deleted_at: row.get(3),
                    body: None,
                })
            })
            .collect();

        Response::json(200, &entries)
    }

    /// Hold the request until something changes, or until `MAX_WAIT`.
    ///
    /// This is the whole of "push". A client that has just finished a pass
    /// asks this and gets nothing back until another machine writes, at which
    /// point it runs a pass — so a task ticked off on the Mac shows up here in
    /// about as long as the network takes, rather than on the next timer.
    ///
    /// **The mark is taken before the query, not after.** A change landing in
    /// between would otherwise be committed, missed by the read, and then
    /// waited straight through — the client would sleep for fifty seconds
    /// holding a snapshot it already knew was stale.
    fn changes(&self, path: &str) -> Response {
        let since = match query_parameter(path, "since") {
            Some(raw) => match DateTime::parse_from_rfc3339(&raw) {
                Ok(at) => at.with_timezone(&Utc),
                Err(error) => return Response::text(400, &format!("bad since: {error}")),
            },
            // No cursor means "tell me where you are" — the first call
            // bootstraps itself and returns at once.
            None => DateTime::UNIX_EPOCH,
        };

        let deadline = Instant::now() + MAX_WAIT;
        loop {
            let mark = self.changes.mark();

            let looked = {
                let mut client = match self.client.lock() {
                    Ok(client) => client,
                    Err(poisoned) => poisoned.into_inner(),
                };
                // Both answers in one borrow, and the lock is dropped before
                // anything waits — a parked request must not hold the only
                // database connection this server has.
                client
                    .query_one(records::CHANGED_SINCE, &[&since])
                    .and_then(|row| {
                        let changed: bool = row.get(0);
                        client
                            .query_one(records::HIGH_WATER, &[])
                            .map(|row| (changed, row.get::<_, Option<DateTime<Utc>>>(0)))
                    })
            };

            let (changed, high_water) = match looked {
                Ok(looked) => looked,
                Err(error) => return Response::text(503, &format!("database: {error}")),
            };
            let now = high_water.unwrap_or(DateTime::UNIX_EPOCH);

            if changed {
                return Response::json(200, &Changed { changed: true, now });
            }

            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Response::json(
                    200,
                    &Changed {
                        changed: false,
                        now,
                    },
                );
            };
            self.changes.wait_past(mark, remaining);
        }
    }

    /// The bodies of the records a client decided to pull.
    ///
    /// Asked for by key rather than "everything since X": the client has
    /// already compared three snapshots and knows exactly which records it
    /// wants, and a server that decided for itself would be making the same
    /// judgement with less information.
    fn fetch(&self, request: &Request) -> Response {
        let wanted: Vec<Wanted> = match serde_json::from_slice(&request.body) {
            Ok(wanted) => wanted,
            Err(error) => return Response::text(400, &format!("bad body: {error}")),
        };

        let mut client = match self.client.lock() {
            Ok(client) => client,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut found: Vec<Entry> = Vec::new();
        for kind in [
            RecordKind::Task,
            RecordKind::Project,
            RecordKind::Section,
            RecordKind::Label,
            RecordKind::Filter,
        ] {
            let ids: Vec<String> = wanted
                .iter()
                .filter(|key| key.kind == kind)
                .map(|key| key.id.clone())
                .collect();
            if ids.is_empty() {
                continue;
            }

            let rows = match client.query(records::FETCH, &[&records::kind_name(kind), &ids]) {
                Ok(rows) => rows,
                Err(error) => return Response::text(503, &format!("database: {error}")),
            };
            for row in &rows {
                found.push(Entry {
                    kind,
                    id: row.get(1),
                    updated_at: row.get(2),
                    deleted_at: row.get(3),
                    body: row.get(4),
                });
            }
        }

        Response::json(200, &found)
    }

    fn write(&self, request: &Request, deleting: bool) -> Response {
        let incoming: Vec<Incoming> = match serde_json::from_slice(&request.body) {
            Ok(incoming) => incoming,
            Err(error) => return Response::text(400, &format!("bad body: {error}")),
        };

        let mut client = match self.client.lock() {
            Ok(client) => client,
            Err(poisoned) => poisoned.into_inner(),
        };

        // One transaction for the batch. A pass that dies half way through
        // then leaves the server exactly as it was, rather than holding a
        // project whose tasks never arrived.
        let mut transaction = match client.transaction() {
            Ok(transaction) => transaction,
            Err(error) => return Response::text(503, &format!("database: {error}")),
        };

        let mut report = WriteReport {
            applied: 0,
            stale: 0,
            stale_ids: Vec::new(),
        };

        for record in &incoming {
            let kind = records::kind_name(record.kind);
            let result = if deleting {
                transaction.query_opt(records::DELETE, &[&kind, &record.id, &record.updated_at])
            } else {
                transaction.query_opt(
                    records::UPSERT,
                    &[&kind, &record.id, &record.updated_at, &record.body],
                )
            };

            match result {
                // `RETURNING` gives a row only when the conflict clause fired,
                // so no row is precisely "the stored copy was not older".
                Ok(Some(_)) => report.applied += 1,
                Ok(None) => {
                    report.stale += 1;
                    report.stale_ids.push(record.id.clone());
                }
                Err(error) => return Response::text(503, &format!("database: {error}")),
            }
        }

        // Inside the transaction, so a rollback announces nothing: Postgres
        // holds notifications until commit for exactly this reason.
        if report.applied > 0 {
            if let Err(error) = transaction.execute(records::ANNOUNCE, &[]) {
                return Response::text(503, &format!("database: {error}"));
            }
        }

        match transaction.commit() {
            Ok(()) => Response::json(200, &report),
            Err(error) => Response::text(503, &format!("database: {error}")),
        }
    }
}

/// One value out of a query string.
///
/// Deliberately tiny: this server has one route with one parameter, and a
/// general parser would be more code than the thing it parses.
fn query_parameter(path: &str, name: &str) -> Option<String> {
    let (_, query) = path.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| percent_decoded(value))
    })
}

/// Enough unescaping for an RFC 3339 timestamp, whose only awkward character
/// is the `+` in an offset.
fn percent_decoded(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                match u8::from_str_radix(&raw[index + 1..index + 3], 16) {
                    Ok(byte) => {
                        out.push(byte as char);
                        index += 3;
                    }
                    Err(_) => {
                        out.push('%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(' ');
                index += 1;
            }
            byte => {
                out.push(byte as char);
                index += 1;
            }
        }
    }
    out
}

/// Whether a token is long enough to be worth having.
///
/// The server refuses to start without one rather than inventing a default:
/// a shared secret nobody chose is a shared secret everybody has.
pub const MIN_TOKEN: usize = 32;

pub fn check_token(token: &str) -> Result<(), String> {
    if token.len() < MIN_TOKEN {
        return Err(format!(
            "the token must be at least {MIN_TOKEN} characters; generate one with `openssl rand \
             -hex 32`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_is_read_out_of_the_query_string() {
        assert_eq!(
            query_parameter("/changes?since=2026-08-05T12:00:00Z", "since").as_deref(),
            Some("2026-08-05T12:00:00Z")
        );
    }

    #[test]
    fn no_cursor_is_none_rather_than_an_empty_string() {
        // None bootstraps the client; an empty string would fail to parse and
        // turn a first call into an error.
        assert_eq!(query_parameter("/changes", "since"), None);
        assert_eq!(query_parameter("/changes?other=1", "since"), None);
    }

    #[test]
    fn an_escaped_offset_survives_the_trip() {
        // The one character that actually bites: `+` in `+01:00` is a space in
        // a query string, so a client sends it escaped.
        assert_eq!(
            query_parameter("/changes?since=2026-08-05T12%3A00%3A00%2B01%3A00", "since").as_deref(),
            Some("2026-08-05T12:00:00+01:00")
        );
    }

    #[test]
    fn a_waiter_is_woken_by_an_announcement() {
        use std::sync::Arc;

        let changes = Arc::new(Changes::default());
        let mark = changes.mark();

        let waker = Arc::clone(&changes);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            waker.announce();
        });

        // Far longer than the wake needs, so finishing early is the wake and
        // not the timeout.
        let started = Instant::now();
        changes.wait_past(mark, Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_secs(4), "it timed out");
        assert_ne!(changes.mark(), mark);
    }

    #[test]
    fn a_change_that_lands_before_the_wait_does_not_get_slept_through() {
        // The race the counter exists for: a flag set and cleared between the
        // read and the wait would leave the client asleep on stale data.
        let changes = Changes::default();
        let mark = changes.mark();
        changes.announce();

        let started = Instant::now();
        changes.wait_past(mark, Duration::from_secs(5));
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "it waited for a change it had already been told about"
        );
    }

    #[test]
    fn a_short_token_is_refused_at_startup() {
        assert!(check_token("hunter2").is_err());
        assert!(check_token(&"a".repeat(MIN_TOKEN)).is_ok());
    }

    #[test]
    fn a_deletion_arrives_without_a_body() {
        let incoming: Vec<Incoming> = serde_json::from_str(
            r#"[{"kind":"task","id":"t1","updated_at":"2026-08-04T12:00:00Z"}]"#,
        )
        .expect("a deletion");
        assert_eq!(incoming.len(), 1);
        assert!(incoming[0].body.is_none());
        assert_eq!(incoming[0].kind, RecordKind::Task);
    }

    #[test]
    fn a_record_arrives_with_one() {
        let incoming: Vec<Incoming> = serde_json::from_str(
            r#"[{"kind":"project","id":"p1","updated_at":"2026-08-04T12:00:00Z","body":{"name":"Work"}}]"#,
        )
        .expect("a record");
        assert_eq!(incoming[0].body.as_ref().unwrap()["name"], "Work");
    }
}

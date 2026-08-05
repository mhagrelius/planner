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

use std::sync::Mutex;

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

/// Anything the server needs to answer a request.
pub struct Server {
    /// One connection behind a lock. This serves one person's three machines
    /// on a timer, so contention is a few requests a minute — a pool would be
    /// machinery for load that does not exist. A thread per connection still
    /// means requests overlap; they queue here, briefly.
    client: Mutex<postgres::Client>,
    token: String,
}

impl Server {
    pub fn new(client: postgres::Client, token: String) -> Self {
        Self {
            client: Mutex::new(client),
            token,
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

        match transaction.commit() {
            Ok(()) => Response::json(200, &report),
            Err(error) => Response::text(503, &format!("database: {error}")),
        }
    }
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

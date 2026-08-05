//! Talking to `planner-server`, and running a pass without blocking the app.
//!
//! # The rule that matters
//!
//! **The worker does network and nothing else.** It is handed a snapshot and
//! gives back records; the main thread merges them through
//! [`PlannerApplication::mutate`](crate::ui::application::PlannerApplication)
//! and lets the existing dirty flag and save tick do the write. No code in here
//! opens `planner.json`.
//!
//! Brain had to split `gather` from `apply` because a pull writes files that
//! its save tick is also writing. Planner's document is held whole in memory
//! and written by one tick, which makes the rule simpler and stricter rather
//! than looser: break it and two writers race over the whole document instead
//! of over one note.
//!
//! # The transport
//!
//! Plain HTTP over `std::net`, no dependency. The server speaks four routes
//! with JSON bodies and no TLS, on a tailnet — request line, headers, body,
//! read the reply, close. Adding an HTTP client crate to a GTK app to send
//! four kinds of request is more moving parts than the protocol has.
//!
//! Every call here blocks, and every one of them happens on a worker thread.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::model::sync::{Key, Record, Remote, Snapshot, SyncError};

/// How long a request may take before the pass gives up and tries later.
///
/// Nothing on screen is waiting for this, so the only thing a long timeout buys
/// is a worker thread parked on a NAS that is asleep.
const TIMEOUT: Duration = Duration::from_secs(20);

/// How long to let the server hold a `/changes` request.
///
/// Comfortably past the server's own fifty seconds, because the server giving
/// up is the normal end of a quiet wait and must not look like the network
/// failing.
const WAIT_TIMEOUT: Duration = Duration::from_secs(75);

/// The server, over HTTP.
pub struct HttpRemote {
    host: String,
    port: u16,
    token: String,
}

impl HttpRemote {
    /// Take a `http://host:port` URL apart.
    ///
    /// Deliberately narrow: this speaks to one server whose address the user
    /// wrote in a config file, so anything that is not a plain host and port
    /// is a typo worth reporting rather than a case to support.
    pub fn new(url: &str, token: impl Into<String>) -> Result<Self, SyncError> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| SyncError(format!("{url} must start with http://")))?;
        let rest = rest.trim_end_matches('/');

        let (host, port) = match rest.split_once(':') {
            Some((host, port)) => (
                host.to_string(),
                port.parse()
                    .map_err(|_| SyncError(format!("{port} is not a port")))?,
            ),
            None => (rest.to_string(), 80),
        };
        if host.is_empty() {
            return Err(SyncError(format!("{url} has no host")));
        }

        Ok(Self {
            host,
            port,
            token: token.into(),
        })
    }

    fn request(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<Vec<u8>, SyncError> {
        self.request_within(method, path, body, TIMEOUT)
    }

    fn request_within(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<Vec<u8>, SyncError> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .map_err(|error| SyncError(format!("could not reach {}: {error}", self.host)))?;
        stream.set_read_timeout(Some(timeout)).ok();
        stream.set_write_timeout(Some(TIMEOUT)).ok();

        let body = body.unwrap_or(&[]);
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: \
             application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.host,
            self.token,
            body.len()
        );
        stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(body))
            .and_then(|()| stream.flush())
            .map_err(|error| SyncError(format!("could not send: {error}")))?;

        let mut reply = Vec::new();
        stream
            .read_to_end(&mut reply)
            .map_err(|error| SyncError(format!("could not read the reply: {error}")))?;

        split_response(&reply)
    }

    fn get(&self, path: &str) -> Result<Vec<u8>, SyncError> {
        self.request("GET", path, None)
    }

    /// A request the server is expected to sit on.
    ///
    /// The ordinary timeout would fire long before the server gave up waiting,
    /// turning every quiet minute into a reported failure.
    fn get_slowly(&self, path: &str) -> Result<Vec<u8>, SyncError> {
        self.request_within("GET", path, None, WAIT_TIMEOUT)
    }

    fn post(&self, path: &str, value: &impl serde::Serialize) -> Result<Vec<u8>, SyncError> {
        let body = serde_json::to_vec(value)
            .map_err(|error| SyncError(format!("could not encode the request: {error}")))?;
        self.request("POST", path, Some(&body))
    }
}

/// Pull the status and the body apart, refusing anything that is not a 200.
///
/// A 401 is worth its own sentence: it is the one failure a user can fix, and
/// "unexpected status 401" would send them looking at the network instead of
/// at their token.
fn split_response(reply: &[u8]) -> Result<Vec<u8>, SyncError> {
    let text = String::from_utf8_lossy(reply);
    let mut lines = text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| SyncError("the server said nothing".into()))?;

    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| SyncError(format!("could not read the status in {status_line:?}")))?;

    let separator = reply
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| SyncError("the reply had no body".into()))?;
    let body = reply[separator + 4..].to_vec();

    match status {
        200 => Ok(body),
        401 => Err(SyncError(
            "the server refused the token — check sync_token in the config".into(),
        )),
        _ => Err(SyncError(format!(
            "the server said {status}: {}",
            String::from_utf8_lossy(&body).trim()
        ))),
    }
}

impl Remote for HttpRemote {
    fn snapshot(&self) -> Result<Snapshot, SyncError> {
        let body = self.get("/snapshot")?;
        let entries: Vec<WireEntry> = serde_json::from_slice(&body)
            .map_err(|error| SyncError(format!("could not read the snapshot: {error}")))?;
        Ok(entries.into_iter().map(WireEntry::into_pair).collect())
    }

    fn fetch(&self, keys: &[Key]) -> Result<Vec<Record>, SyncError> {
        let body = self.post("/fetch", &keys)?;
        serde_json::from_slice(&body)
            .map_err(|error| SyncError(format!("could not read the records: {error}")))
    }

    fn push(&self, records: &[Record]) -> Result<(), SyncError> {
        self.post("/records", &records).map(|_| ())
    }

    fn delete(&self, records: &[Record]) -> Result<(), SyncError> {
        self.post("/deletions", &records).map(|_| ())
    }

    fn wait_for_change(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<(bool, chrono::DateTime<chrono::Utc>), SyncError> {
        // The `+` in an offset would be read as a space in a query string, so
        // the cursor goes over the wire in the one format that has none.
        let since = since.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        let body = self.get_slowly(&format!("/changes?since={since}"))?;
        let answer: Changed = serde_json::from_slice(&body)
            .map_err(|error| SyncError(format!("could not read the answer: {error}")))?;
        Ok((answer.changed, answer.now))
    }
}

/// What the server says when it stops holding a request.
#[derive(serde::Deserialize)]
struct Changed {
    changed: bool,
    now: chrono::DateTime<chrono::Utc>,
}

/// A snapshot row as the server writes it.
#[derive(serde::Deserialize)]
struct WireEntry {
    kind: crate::model::RecordKind,
    id: String,
    updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl WireEntry {
    fn into_pair(self) -> (Key, crate::model::sync::Version) {
        use crate::model::sync::Version;
        let version = match self.deleted_at {
            Some(at) => Version::Deleted(at),
            None => Version::Live(self.updated_at),
        };
        (Key::new(self.kind, self.id), version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_taken_apart() {
        let remote = HttpRemote::new("http://nas:8083", "t").expect("a url");
        assert_eq!(remote.host, "nas");
        assert_eq!(remote.port, 8083);
    }

    #[test]
    fn a_trailing_slash_is_not_part_of_the_port() {
        let remote = HttpRemote::new("http://nas:8083/", "t").expect("a url");
        assert_eq!(remote.port, 8083);
    }

    #[test]
    fn https_is_refused_rather_than_quietly_downgraded() {
        // Accepting it and connecting in the clear would be the worst of the
        // three possible behaviours.
        assert!(HttpRemote::new("https://nas:8083", "t").is_err());
    }

    #[test]
    fn a_missing_port_means_eighty() {
        assert_eq!(HttpRemote::new("http://nas", "t").expect("a url").port, 80);
    }

    #[test]
    fn a_body_is_taken_from_after_the_blank_line() {
        let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]";
        assert_eq!(split_response(reply).expect("a body"), b"[]");
    }

    #[test]
    fn a_refused_token_says_so_in_words() {
        let reply = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 12\r\n\r\nunauthorized";
        let error = split_response(reply).expect_err("401");
        assert!(error.0.contains("token"), "{}", error.0);
    }

    #[test]
    fn another_failure_carries_what_the_server_said() {
        let reply = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 8\r\n\r\ndatabase";
        let error = split_response(reply).expect_err("503");
        assert!(error.0.contains("503"), "{}", error.0);
        assert!(error.0.contains("database"), "{}", error.0);
    }

    #[test]
    fn a_snapshot_row_with_a_deletion_reads_as_deleted() {
        use crate::model::sync::Version;
        let entries: Vec<WireEntry> = serde_json::from_str(
            r#"[{"kind":"task","id":"t1","updated_at":"2026-08-05T12:00:00Z",
                 "deleted_at":"2026-08-05T13:00:00Z"}]"#,
        )
        .expect("a row");
        let (key, version) = entries.into_iter().next().unwrap().into_pair();
        assert_eq!(key.id, "t1");
        assert!(matches!(version, Version::Deleted(_)));
    }
}

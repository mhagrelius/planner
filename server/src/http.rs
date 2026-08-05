//! Enough HTTP to serve four routes on a private network.
//!
//! Request line, headers to the blank line, a body whose length the headers
//! gave, one response, close. No keep-alive, no chunked encoding, no TLS —
//! this listens on a tailnet address and speaks to one person's machines, and
//! WireGuard is doing the encrypting. If any of those stops being true this
//! should take a real server rather than grow into one.
//!
//! The parsing is the part worth testing, so it is a function over bytes
//! rather than something tangled up with a socket.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};

/// A parsed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    /// A header, matched without regard to case as the spec requires.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// The bearer token, if one was offered.
    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization")?
            .strip_prefix("Bearer ")
            .map(str::trim)
    }
}

/// Why a request could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadRequest(pub String);

/// The largest body this will read.
///
/// A whole task list is tens of kilobytes and a sync batch is smaller, so this
/// is generous by two orders of magnitude. It exists so that a malformed
/// `Content-Length` cannot ask the server to allocate a gigabyte.
pub const MAX_BODY: usize = 16 * 1024 * 1024;

/// Read one request from a stream.
pub fn read_request<R: Read>(stream: R) -> Result<Request, BadRequest> {
    let mut reader = BufReader::new(stream);

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| BadRequest(error.to_string()))?;

    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| BadRequest("empty request line".into()))?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| BadRequest("no path".into()))?
        .to_string();

    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| BadRequest(error.to_string()))?;
        // End of stream before the blank line: the client hung up mid-request.
        if read == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let length: usize = match headers.get("content-length") {
        Some(value) => value
            .parse()
            .map_err(|_| BadRequest(format!("content-length is not a number: {value}")))?,
        None => 0,
    };
    if length > MAX_BODY {
        return Err(BadRequest(format!("body of {length} bytes is too large")));
    }

    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| BadRequest(error.to_string()))?;

    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

/// What to send back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: &'static str,
}

impl Response {
    pub fn json(status: u16, value: &impl serde::Serialize) -> Self {
        // A response that will not serialise is a bug here rather than
        // anything the client did, and saying so beats sending half of one.
        let body = serde_json::to_vec(value)
            .unwrap_or_else(|error| format!(r#"{{"error":"{error}"}}"#).into_bytes());
        Self {
            status,
            body,
            content_type: "application/json",
        }
    }

    pub fn text(status: u16, message: &str) -> Self {
        Self {
            status,
            body: message.as_bytes().to_vec(),
            content_type: "text/plain; charset=utf-8",
        }
    }
}

/// Write a response and finish with the connection.
pub fn write_response<W: Write>(mut stream: W, response: &Response) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    };

    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(raw: &str) -> Result<Request, BadRequest> {
        read_request(raw.as_bytes())
    }

    #[test]
    fn a_plain_get_parses() {
        let parsed = request("GET /health HTTP/1.1\r\nHost: nas\r\n\r\n").expect("a request");
        assert_eq!(parsed.method, "GET");
        assert_eq!(parsed.path, "/health");
        assert_eq!(parsed.header("host"), Some("nas"));
        assert!(parsed.body.is_empty());
    }

    #[test]
    fn a_body_is_read_to_the_length_the_headers_gave() {
        let parsed = request("POST /records HTTP/1.1\r\nContent-Length: 7\r\n\r\n{\"a\":1}")
            .expect("a request");
        assert_eq!(parsed.body, b"{\"a\":1}");
    }

    #[test]
    fn header_names_match_whatever_case_they_arrive_in() {
        let parsed =
            request("GET / HTTP/1.1\r\nCONTENT-length: 0\r\nAuthorization: Bearer hunter2\r\n\r\n")
                .expect("a request");
        assert_eq!(parsed.header("content-length"), Some("0"));
        assert_eq!(parsed.bearer(), Some("hunter2"));
    }

    #[test]
    fn no_authorization_header_is_no_token_rather_than_an_error() {
        let parsed = request("GET / HTTP/1.1\r\n\r\n").expect("a request");
        assert_eq!(parsed.bearer(), None);
    }

    #[test]
    fn a_content_length_that_is_not_a_number_is_refused() {
        let error = request("POST / HTTP/1.1\r\nContent-Length: lots\r\n\r\n")
            .expect_err("that is not a length");
        assert!(error.0.contains("lots"), "{}", error.0);
    }

    #[test]
    fn an_absurd_content_length_is_refused_before_anything_is_allocated() {
        let raw = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        let error = request(&raw).expect_err("too large");
        assert!(error.0.contains("too large"), "{}", error.0);
    }

    #[test]
    fn a_body_shorter_than_its_length_is_refused_rather_than_padded() {
        // Otherwise a client that hangs up mid-write would look like one that
        // sent an empty record.
        let error =
            request("POST / HTTP/1.1\r\nContent-Length: 40\r\n\r\nshort").expect_err("truncated");
        assert!(!error.0.is_empty());
    }

    #[test]
    fn a_response_carries_its_own_length() {
        let mut written = Vec::new();
        write_response(&mut written, &Response::text(404, "no such route")).expect("write");
        let text = String::from_utf8(written).expect("utf-8");

        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"), "{text}");
        assert!(text.contains("Content-Length: 13\r\n"), "{text}");
        assert!(text.ends_with("\r\n\r\nno such route"), "{text}");
    }
}

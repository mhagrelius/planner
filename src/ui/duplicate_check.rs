//! The socket for [`crate::model::duplicate`].
//!
//! This is the only file in the app that opens a TLS connection, and the only
//! one that talks to anything outside the machine other than `ui::sync`. It
//! follows the shape `PlannerApplication::start_sync` established: a blocking
//! call on a worker thread, answered by polling a channel on the main loop, so
//! nothing here ever touches a widget off-thread.
//!
//! **Every reply carries the generation that asked for it.** Quick-add re-runs
//! this as the title changes, and a slow answer to a title the user has since
//! finished editing is worse than no answer — it would put a verdict about
//! "Buy mil" underneath "Buy milk for Tuesday". The caller bumps a counter per
//! request and drops anything that comes back stale, which is cancellation in
//! the only sense that matters: the reply is ignored and the thread is left to
//! finish on its own.

use std::time::Duration;

use gtk::glib;

use crate::model::duplicate::{
    parse_reply, request_body, CheckError, Judgements, API_VERSION, BETA, ENDPOINT,
};
use crate::model::similar::Candidate;

/// Whole-request budget. This sits in front of the most frequent action in the
/// app; past a few seconds the answer has stopped being useful, and the local
/// candidates are already on screen either way.
const TIMEOUT: Duration = Duration::from_secs(8);

/// How often the main loop asks whether the worker is done.
///
/// Shorter than sync's 250ms because someone is watching this one: a debounced
/// call already costs most of a second before it starts, and a further quarter
/// on top is visible.
const POLL: Duration = Duration::from_millis(50);

/// A non-2xx body is worth showing but not worth holding — the interesting part
/// is at the front, and an HTML error page from a proxy could be a megabyte.
const MAX_ERROR_BODY: usize = 400;

/// Ask the model, blocking. Call this on a worker thread.
pub fn check(key: &str, title: &str, candidates: &[Candidate]) -> Result<Judgements, CheckError> {
    // Configured on the agent rather than the request: `RequestBuilder::config`
    // erases the body-carrying type parameter that `send_json` needs.
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            // A 4xx must come back as a response rather than an error, because
            // the body explains what is wrong with the key and that is the one
            // thing worth putting in front of a person here.
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build(),
    );

    let response = agent
        .post(ENDPOINT)
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .header("anthropic-beta", BETA)
        .header("content-type", "application/json")
        .send_json(request_body(title, candidates))
        .map_err(|error| CheckError::Unreachable(error.to_string()))?;

    let status = response.status().as_u16();
    let mut response = response;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| CheckError::Unreachable(error.to_string()))?;

    if !(200..300).contains(&status) {
        let mut body = body;
        body.truncate(MAX_ERROR_BODY);
        return Err(CheckError::Http { status, body });
    }

    parse_reply(&body)
}

/// Run [`check`] off the main loop and hand the answer back on it.
///
/// `generation` is echoed to `finished` untouched; the caller compares it
/// against its own counter and ignores anything stale. `finished` always runs
/// exactly once unless the caller is dropped first.
pub fn spawn<F>(
    key: String,
    title: String,
    candidates: Vec<Candidate>,
    generation: u64,
    finished: F,
) where
    F: Fn(u64, Result<Judgements, CheckError>) + 'static,
{
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(check(&key, &title, &candidates));
    });

    glib::timeout_add_local(POLL, move || match receiver.try_recv() {
        Ok(outcome) => {
            finished(generation, outcome);
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        // The worker died without answering. Nothing was shown, and the local
        // candidates are still on screen, so there is nothing to undo.
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            finished(
                generation,
                Err(CheckError::Unreachable("the worker stopped".into())),
            );
            glib::ControlFlow::Break
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key that is not there is not a network problem, and asking anyway
    /// would send a request guaranteed to fail. The caller checks first; this
    /// asserts the error it uses to say so is distinguishable from the rest.
    #[test]
    fn a_missing_key_is_its_own_failure_not_a_network_one() {
        assert_ne!(
            CheckError::NotConfigured,
            CheckError::Unreachable(String::new())
        );
    }

    #[test]
    fn the_timeout_is_short_enough_to_sit_in_front_of_typing() {
        assert!(TIMEOUT <= Duration::from_secs(10));
        assert!(POLL <= Duration::from_millis(100));
    }
}

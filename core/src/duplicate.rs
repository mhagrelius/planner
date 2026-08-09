//! Asking a language model whether two tasks are the same errand.
//!
//! [`crate::similar`] finds tasks that share words. This finds tasks that share
//! *meaning* — "Call the plumber" and "Ring the plumber", which no amount of
//! word comparison will ever join. It is the only thing in this crate that
//! needs a network, and it is off unless a key has been put in the config file
//! on purpose.
//!
//! **The local check runs first and narrows the field.** A prompt cannot hold
//! every task, and re-sending the whole list on every keystroke would be absurd
//! even if it fit. So `similar` picks a handful of candidates and only those
//! are described here. That ordering is not an optimisation; it is what makes
//! the feature affordable at all.
//!
//! **Everything here is pure.** Building the request body, reading the reply,
//! and deciding what a verdict means are functions over strings — the socket
//! lives in the UI half, next to the other thing that already opens one. That
//! keeps the part with the interesting bugs testable with no display and no
//! network.

use serde::{Deserialize, Serialize};

use crate::similar::Candidate;

/// The model this asks. Opus 5 — thinking is on by default there, and the
/// request below turns it off, which is allowed at `high` effort or lower.
pub const MODEL: &str = "claude-opus-5";

/// The wire version, fixed for the life of the format.
pub const API_VERSION: &str = "2023-06-01";

/// Opting in to server-side fallbacks: a request the safety classifiers
/// decline is re-served by another model inside the same call rather than
/// coming back as a refusal. A task title is unlikely to trip them, but the
/// failure would be invisible without this and the cost of asking is nothing.
pub const BETA: &str = "server-side-fallback-2026-07-01";

pub const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";

/// Enough for a verdict and a short reason per candidate, and no more. The
/// reply is schema-constrained, so this bounds latency rather than truncating
/// anything anyone wanted.
const MAX_TOKENS: u32 = 1024;

/// How many candidates are worth describing to the model.
///
/// The local check has already ranked them; past a handful the extra ones are
/// noise that costs tokens and latency.
pub const MAX_CANDIDATES: usize = 8;

/// What the model decided about one candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// The same errand written twice. Worth interrupting over.
    Same,
    /// About the same thing, but not the same piece of work. Worth showing,
    /// not worth blocking on — "Buy milk" and "Buy bread" both being shopping
    /// is not a reason to refuse to add the second one.
    Related,
    /// Not a duplicate.
    Different,
}

impl Verdict {
    /// Whether this alone justifies stopping to ask.
    pub fn blocks(self) -> bool {
        matches!(self, Self::Same)
    }
}

/// One candidate, judged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Judgement {
    /// The candidate's task id, echoed back so the answer can be matched to
    /// the row it is about.
    pub id: String,
    pub verdict: Verdict,
    /// One short clause, shown to the user. The prompt asks for the reason
    /// rather than just the label because "is this the same?" is a question
    /// the person is better placed to answer than the model, and they can only
    /// answer it if they can see what the model thought.
    #[serde(default)]
    pub reason: String,
}

/// The whole reply.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Judgements {
    #[serde(default)]
    pub duplicates: Vec<Judgement>,
}

impl Judgements {
    /// The ids the model called outright duplicates.
    pub fn blocking(&self) -> Vec<&str> {
        self.duplicates
            .iter()
            .filter(|judgement| judgement.verdict.blocks())
            .map(|judgement| judgement.id.as_str())
            .collect()
    }

    /// What it said about one candidate, if it said anything.
    pub fn for_id(&self, id: &str) -> Option<&Judgement> {
        self.duplicates.iter().find(|judgement| judgement.id == id)
    }
}

/// Why a check produced no verdict.
///
/// Every variant means "carry on with the local answer". None of them is worth
/// a dialog: the user is trying to add a task, not administer an API key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// No key in the config. The ordinary state for anyone who has not asked
    /// for this.
    NotConfigured,
    /// The socket, DNS, TLS, or a timeout.
    Unreachable(String),
    /// A non-2xx reply.
    Http { status: u16, body: String },
    /// The classifiers declined and the fallback model declined too.
    Refused,
    /// A 2xx reply that was not the shape promised.
    Malformed(String),
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(formatter, "no API key configured"),
            Self::Unreachable(why) => write!(formatter, "could not reach the API: {why}"),
            Self::Http { status, body } => write!(formatter, "API returned {status}: {body}"),
            Self::Refused => write!(formatter, "the request was declined"),
            Self::Malformed(why) => write!(formatter, "unexpected reply: {why}"),
        }
    }
}

const SYSTEM: &str = "\
You compare a task a person is about to add against tasks already on their \
list, and say which are the same piece of work.

Judge the errand, not the wording. \"Call the plumber\" and \"Ring the plumber\" \
are the same task. \"Buy milk\" and \"Buy bread\" are two errands that happen to \
be shopping, and are not.

Use these labels:
- same: doing one of these would make the other pointless.
- related: the same topic or project, but each is its own piece of work.
- different: not a duplicate.

Be conservative with \"same\". A wrong \"same\" stops someone adding a task they \
wanted; a wrong \"different\" costs them nothing but a second look. Two tasks \
that differ in a number, a date, a name, or a recipient are different tasks \
even when the sentence is otherwise identical.

Give a reason of at most twelve words, addressed to the person, saying what \
made you decide. Return every candidate you were given, once.";

/// The `output_config.format` schema. The reply is constrained to this, so
/// parsing it cannot fail on a stray sentence of preamble.
fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "duplicates": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "verdict": {
                            "type": "string",
                            "enum": ["same", "related", "different"]
                        },
                        "reason": { "type": "string" }
                    },
                    "required": ["id", "verdict", "reason"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["duplicates"],
        "additionalProperties": false
    })
}

/// What the model is asked to look at.
pub fn user_turn(title: &str, candidates: &[Candidate]) -> String {
    let mut prompt = format!("New task:\n{}\n\nAlready on the list:\n", title.trim());
    for candidate in candidates.iter().take(MAX_CANDIDATES) {
        prompt.push_str(&format!(
            "- id: {}\n  title: {}\n  where: {}\n",
            candidate.id.as_str(),
            candidate.title,
            if candidate.context.is_empty() {
                "—"
            } else {
                &candidate.context
            },
        ));
    }
    prompt
}

/// The request body, ready to POST.
///
/// Thinking is off and effort is `low`: this is a short classification with a
/// constrained output shape, and it sits in front of the most frequent action
/// in the app, so latency is the thing to optimise. The schema also removes the
/// one documented hazard of running Opus 5 without thinking — internal tags
/// leaking into the visible reply — because the reply cannot be anything but
/// the object above.
pub fn request_body(title: &str, candidates: &[Candidate]) -> serde_json::Value {
    serde_json::json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "thinking": { "type": "disabled" },
        "fallbacks": "default",
        "output_config": {
            "effort": "low",
            "format": { "type": "json_schema", "schema": schema() }
        },
        "system": SYSTEM,
        "messages": [{ "role": "user", "content": user_turn(title, candidates) }]
    })
}

/// Read a 2xx body.
///
/// `stop_reason` is checked before `content`, because a declined request is a
/// successful HTTP response with an empty `content` array and indexing into it
/// would panic on the one case this most needs to survive.
pub fn parse_reply(body: &str) -> Result<Judgements, CheckError> {
    let reply: serde_json::Value =
        serde_json::from_str(body).map_err(|error| CheckError::Malformed(error.to_string()))?;

    if reply.get("stop_reason").and_then(|stop| stop.as_str()) == Some("refusal") {
        return Err(CheckError::Refused);
    }

    let text = reply
        .get("content")
        .and_then(|content| content.as_array())
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|block| block.get("type").and_then(|kind| kind.as_str()) == Some("text"))
        })
        .and_then(|block| block.get("text"))
        .and_then(|text| text.as_str())
        .ok_or_else(|| CheckError::Malformed("no text block in the reply".into()))?;

    serde_json::from_str(text).map_err(|error| CheckError::Malformed(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TaskId;

    fn candidate(id: &str, title: &str) -> Candidate {
        Candidate {
            id: TaskId::from_raw(id),
            title: title.to_string(),
            context: "Inbox".into(),
            checked: false,
            score: 0.5,
        }
    }

    fn reply_with(text: &str) -> String {
        serde_json::json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": text }]
        })
        .to_string()
    }

    #[test]
    fn the_request_names_the_model_and_turns_thinking_off() {
        let body = request_body("Buy milk", &[candidate("t1", "buy some milk")]);
        assert_eq!(body["model"], MODEL);
        assert_eq!(body["thinking"]["type"], "disabled");
        // Disabled thinking is only legal at `high` effort or below.
        assert_eq!(body["output_config"]["effort"], "low");
    }

    #[test]
    fn the_reply_shape_is_pinned_by_a_schema() {
        let body = request_body("Buy milk", &[candidate("t1", "buy some milk")]);
        let format = &body["output_config"]["format"];
        assert_eq!(format["type"], "json_schema");
        assert_eq!(format["schema"]["additionalProperties"], false);
        let item = &format["schema"]["properties"]["duplicates"]["items"];
        assert_eq!(item["properties"]["verdict"]["enum"][0], "same");
        assert_eq!(item["additionalProperties"], false);
    }

    #[test]
    fn a_fallback_is_asked_for_so_a_decline_is_not_a_dead_end() {
        let body = request_body("Buy milk", &[candidate("t1", "milk")]);
        assert_eq!(body["fallbacks"], "default");
    }

    #[test]
    fn every_candidate_reaches_the_prompt_with_its_id() {
        let prompt = user_turn(
            "Buy milk",
            &[
                candidate("t1", "buy some milk"),
                candidate("t2", "get milk"),
            ],
        );
        assert!(prompt.contains("Buy milk"));
        assert!(prompt.contains("id: t1"));
        assert!(prompt.contains("id: t2"));
        assert!(prompt.contains("buy some milk"));
    }

    #[test]
    fn the_prompt_never_grows_past_the_candidate_cap() {
        let many: Vec<Candidate> = (0..30)
            .map(|index| candidate(&format!("t{index}"), &format!("task {index}")))
            .collect();
        let prompt = user_turn("Buy milk", &many);
        assert_eq!(prompt.matches("  id: ").count(), 0);
        assert_eq!(prompt.matches("- id: ").count(), MAX_CANDIDATES);
    }

    #[test]
    fn a_verdict_is_read_back_off_the_wire() {
        let body = reply_with(
            r#"{"duplicates":[{"id":"t1","verdict":"same","reason":"the same errand"}]}"#,
        );
        let judgements = parse_reply(&body).expect("a verdict");
        assert_eq!(judgements.duplicates.len(), 1);
        assert_eq!(judgements.duplicates[0].verdict, Verdict::Same);
        assert_eq!(judgements.blocking(), vec!["t1"]);
        assert_eq!(judgements.for_id("t1").unwrap().reason, "the same errand");
        assert!(judgements.for_id("nope").is_none());
    }

    #[test]
    fn only_same_is_worth_stopping_for() {
        assert!(Verdict::Same.blocks());
        assert!(!Verdict::Related.blocks());
        assert!(!Verdict::Different.blocks());

        let body = reply_with(
            r#"{"duplicates":[
                {"id":"t1","verdict":"related","reason":"both shopping"},
                {"id":"t2","verdict":"different","reason":"unrelated"}
            ]}"#,
        );
        assert!(parse_reply(&body).expect("a verdict").blocking().is_empty());
    }

    #[test]
    fn a_refusal_is_read_before_the_content_array_is_touched() {
        // A declined request is a 200 with an empty `content`. Reaching for
        // content[0] first would panic on exactly this reply.
        let body = serde_json::json!({
            "stop_reason": "refusal",
            "content": [],
            "stop_details": { "type": "refusal", "category": "cyber" }
        })
        .to_string();
        assert_eq!(parse_reply(&body), Err(CheckError::Refused));
    }

    #[test]
    fn a_reply_that_is_not_the_promised_shape_is_an_error_not_a_panic() {
        assert!(matches!(
            parse_reply("not json at all"),
            Err(CheckError::Malformed(_))
        ));
        assert!(matches!(
            parse_reply(r#"{"stop_reason":"end_turn","content":[]}"#),
            Err(CheckError::Malformed(_))
        ));
        assert!(matches!(
            parse_reply(&reply_with("{\"duplicates\": \"not an array\"}")),
            Err(CheckError::Malformed(_))
        ));
    }

    #[test]
    fn a_thinking_block_before_the_text_does_not_confuse_the_parse() {
        let body = serde_json::json!({
            "stop_reason": "end_turn",
            "content": [
                { "type": "thinking", "thinking": "" },
                { "type": "text", "text": r#"{"duplicates":[]}"# }
            ]
        })
        .to_string();
        assert_eq!(parse_reply(&body), Ok(Judgements::default()));
    }

    #[test]
    fn every_failure_says_something_a_person_could_act_on() {
        for error in [
            CheckError::NotConfigured,
            CheckError::Unreachable("timed out".into()),
            CheckError::Http {
                status: 401,
                body: "bad key".into(),
            },
            CheckError::Refused,
            CheckError::Malformed("trailing comma".into()),
        ] {
            assert!(!error.to_string().is_empty());
        }
    }
}

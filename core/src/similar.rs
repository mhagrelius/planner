//! Which existing tasks look like the one being typed.
//!
//! This is the half of duplicate detection that runs on every keystroke, so it
//! is pure, local, and measured in microseconds. It answers a narrower question
//! than [`crate::search`]: not "which tasks contain this text" but "which tasks
//! are trying to be this task".
//!
//! **Word sets, not substrings.** `search` ranks a substring hit and is right
//! to — you type a fragment and want what contains it. A duplicate is the other
//! shape: "Buy milk" and "buy some milk" share no useful substring ordering but
//! are the same errand, while "milk" is a substring of "milkshake recipe" and is
//! not. So the comparison is over normalised word sets, and word order does not
//! matter.
//!
//! **Two ratios, averaged.** Jaccard alone punishes a short title against a long
//! one: "Milk" against "Buy milk for the party" is 1/5 even though the first is
//! wholly contained in the second. Containment alone does the opposite and calls
//! every one-word title a duplicate of every task mentioning that word. Averaging
//! them keeps a fully-contained short title respectable without letting it reach
//! the certainty of a real match.
//!
//! **It cannot see synonyms, and does not pretend to.** "Call the plumber" and
//! "Ring the plumber" score 0.42 here — too low to show on this evidence alone,
//! and obviously the same errand to anyone reading them. That is the gap
//! [`crate::duplicate`] exists to close, and the reason this module returns
//! *candidates* rather than verdicts.
//!
//! **Which is why there are two floors, not one.** [`FLOOR`] is the score at
//! which a pair is worth showing on word overlap alone. [`RECALL_FLOOR`] is the
//! much lower score at which a pair is worth *asking about* — because a
//! prefilter that only forwards what it was already sure of makes the model
//! unable to tell it anything it did not know. "Ring the plumber" clears the
//! second and not the first, which is exactly the case that has to survive the
//! journey to be worth making it.
//!
//! The honest limit of both, stated plainly: two titles sharing no content word
//! at all — "Repair the dripping tap" and "Sort out the leaking tap" — score
//! near zero here and would never be retrieved by anything in this module.
//! That is what `ui::embedding` is for: it ranks the whole list by sentence
//! embedding and contributes its top few candidates alongside these, when a
//! model is installed. [`rank_by_vector`] is the pure half of it, and its
//! documentation is where the "rank, never threshold" rule is written down.

use crate::store::Store;
use crate::TaskId;

/// Below this, two titles are not worth showing side by side.
///
/// Tuned against the case that decides it: two short titles sharing only a
/// verb — "Book flights" and "Book dentist", "Email Sam" and "Email the
/// landlord" — score 0.42 and must not appear.
pub const FLOOR: f32 = 0.5;

/// At or above this, the titles are the same sentence in different clothes and
/// the app may say so without asking anything over the network.
///
/// Tuned against the other decisive case: a title wholly contained in a longer
/// one — "Buy bread" against "Buy milk and bread" — reaches 0.83 and is a
/// different errand, so certainty has to sit above that.
pub const STRONG: f32 = 0.9;

/// Worth asking a model about, even though the words alone do not justify
/// showing it.
///
/// Low on purpose. Everything above this is forwarded for judgement, and the
/// judgement is what decides whether it reaches the screen — a prefilter tuned
/// to the same precision as the display would only ever hand over pairs it had
/// already decided, which is a prefilter that cannot be told it was wrong.
/// "Ring the plumber" against "Call the plumber" is 0.42 and is the case this
/// number exists to let through.
pub const RECALL_FLOOR: f32 = 0.2;

/// A completed task is worth mentioning but is rarely the duplicate you meant,
/// so it never outranks an open one.
const COMPLETED_PENALTY: f32 = 0.6;

/// How many tasks the embedding ranker contributes, when there is one.
///
/// A rank cutoff rather than a score cutoff — see [`rank_by_vector`] for why
/// there is no such thing as a meaningful cosine threshold here.
pub const SEMANTIC_TOP_K: usize = 5;

/// Cosine similarity of two unit-length vectors.
///
/// Both sides are normalised at the point they are produced, so this is a dot
/// product. Mismatched lengths are zero rather than a panic: a vector cache
/// written by a different model is stale data, not a crash.
pub fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

/// The `limit` tasks whose vectors sit closest to `query`, closest first.
///
/// **Rank, never threshold.** Measured on this task, sentence-embedding cosines
/// carry almost no absolute meaning: over a real list, "Pay invoice 42" against
/// "Pay invoice 43" scores 0.96 and is not a duplicate, while "Call the
/// plumber" against "Ring the plumber" scores 0.88 and is. Two unrelated
/// errands sit at 0.75 simply because both are short imperative sentences. No
/// cutoff separates those populations.
///
/// What *is* reliable is the ordering within one query: across a twenty-task
/// list the true duplicate came first every time, and by a clear margin over
/// the runner-up. So this returns a ranked shortlist and nothing else decides
/// anything — [`crate::duplicate`] judges what comes back. Anyone tempted to
/// add a `>= 0.8` here should re-read this paragraph.
///
/// `vectors` is `(task index, vector)`; indices are returned, so the caller
/// keeps ownership of whatever it is indexing.
pub fn rank_by_vector(query: &[f32], vectors: &[(usize, Vec<f32>)], limit: usize) -> Vec<usize> {
    let mut scored: Vec<(f32, usize)> = vectors
        .iter()
        .map(|(index, vector)| (cosine(query, vector), *index))
        .filter(|(score, _)| *score > 0.0)
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Index breaks ties, so the same query twice ranks the same way.
            .then_with(|| a.1.cmp(&b.1))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(_, index)| index).collect()
}

/// An existing task that resembles the one being added.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: TaskId,
    pub title: String,
    /// The project name, and whether it is done — what the row says underneath.
    pub context: String,
    pub checked: bool,
    /// `0.0..=1.0`, higher being more alike.
    pub score: f32,
}

impl Candidate {
    /// Whether this is close enough to interrupt over without asking anything
    /// else first.
    pub fn is_strong(&self) -> bool {
        self.score >= STRONG
    }
}

/// Words that carry no information about what a task *is*.
///
/// Deliberately short. Every word removed here is a word two unrelated tasks
/// can no longer be told apart by, so this holds function words and the few
/// verbs that mean "perform this task" rather than naming it.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "the", "to", "of", "for", "or", "in", "on", "at", "by", "with", "from",
    "into", "up", "off", "out", "about", "some", "any", "my", "me", "i", "we", "our", "it", "its",
    "this", "that", "these", "those", "is", "are", "be", "been", "am", "do", "does", "did", "so",
    "then", "than", "please", "need", "needs", "must", "should", "will", "would", "can", "could",
    "get", "got", "make", "made", "go", "going", "again", "also", "just", "new",
];

/// The comparable words in a title, lowercased, stemmed, deduplicated, sorted.
///
/// Sorted so the result is a set with a stable order — two titles with the same
/// words compare equal regardless of how they were written.
pub fn normalise(title: &str) -> Vec<String> {
    let mut words: Vec<String> = title
        .to_lowercase()
        .chars()
        // Keep digits: "Pay invoice 42" and "Pay invoice 43" are different
        // tasks, and dropping the number would merge them.
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .filter(|word| !STOPWORDS.contains(word))
        .map(stem)
        .filter(|word| !word.is_empty())
        .collect();

    words.sort();
    words.dedup();
    words
}

/// Fold the endings that make one word look like two.
///
/// Not a real stemmer — a real one needs a dictionary and would flatten words
/// this has to keep apart. It handles the endings that actually differ between
/// two people writing down the same errand: a plural, a gerund, a past tense.
///
/// **It does not undouble a final consonant**, so `shopping` becomes `shopp`
/// rather than `shop`. Undoubling is ambiguous without a dictionary — the rule
/// that turns `shopping` into `shop` also turns `calling` into `cal`, and
/// `call` is far more common in a task list than the `shopping`/`shop` pair it
/// would rescue. Two people who both write "shopping" still match; one who
/// writes "shop" against another's "shopping" does not.
fn stem(word: &str) -> String {
    let word = word.trim();

    // "groceries" -> "grocery", before the bare plural rule can make
    // "grocerie" out of it.
    if word.len() > 4 {
        if let Some(root) = word.strip_suffix("ies") {
            return format!("{root}y");
        }
    }
    if word.len() > 5 {
        if let Some(root) = word.strip_suffix("ing") {
            return root.to_string();
        }
    }
    if word.len() > 4 {
        if let Some(root) = word.strip_suffix("ed") {
            return root.to_string();
        }
    }
    // "ss" is not a plural: "address" must not become "addres".
    if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") {
        return word[..word.len() - 1].to_string();
    }
    word.to_string()
}

/// How alike two already-normalised word sets are, `0.0..=1.0`.
///
/// The mean of Jaccard (shared / total) and containment (shared / smaller).
/// See the module docs for why neither alone does the job.
pub fn similarity(left: &[String], right: &[String]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }

    let shared = left.iter().filter(|word| right.contains(word)).count() as f32;
    if shared == 0.0 {
        return 0.0;
    }

    let union = (left.len() + right.len()) as f32 - shared;
    let smaller = left.len().min(right.len()) as f32;

    let jaccard = shared / union;
    let containment = shared / smaller;
    (jaccard + containment) / 2.0
}

/// How alike two raw titles are. Convenience over [`normalise`] + [`similarity`].
pub fn compare(left: &str, right: &str) -> f32 {
    similarity(&normalise(left), &normalise(right))
}

/// The existing tasks most like `title`, best first, never more than `limit`.
///
/// `exclude` is the task being edited, if any — a task is not its own
/// duplicate. Quick-add tokens should already be stripped from `title`; pass
/// the parser's title, not the raw line.
///
/// `floor` is the caller's choice of how much evidence is enough: [`FLOOR`] to
/// gather what is worth showing on words alone, [`RECALL_FLOOR`] to gather what
/// is worth putting in front of a model. See the module docs for why those are
/// two different questions.
pub fn candidates(
    store: &Store,
    title: &str,
    exclude: Option<&TaskId>,
    limit: usize,
    floor: f32,
) -> Vec<Candidate> {
    let target = normalise(title);
    if target.is_empty() {
        return Vec::new();
    }

    let mut found: Vec<Candidate> = store
        .tasks()
        .iter()
        .filter(|task| exclude != Some(&task.id))
        .filter_map(|task| {
            let mut score = similarity(&target, &normalise(&task.content));
            if task.checked {
                score *= COMPLETED_PENALTY;
            }
            if score < floor {
                return None;
            }

            let project = store
                .project(&task.project_id)
                .map(|project| project.name.clone())
                .unwrap_or_default();

            Some(Candidate {
                id: task.id.clone(),
                title: task.content.clone(),
                context: if task.checked {
                    format!("{project} · completed")
                } else {
                    project
                },
                checked: task.checked,
                score,
            })
        })
        .collect();

    // Score descending, then title, so equal scores come out in a stable order
    // rather than in whatever order the store happens to hold them.
    found.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    found.truncate(limit);
    found
}

/// Build a candidate for one task, scored against `title` by words.
///
/// Used for tasks the embedding ranker found and the word comparison did not.
/// The score stays the *lexical* one — deliberately, and it will usually be
/// low. It is what [`Candidate::is_strong`] reads, and a strong score is
/// licence to interrupt without asking a model. A cosine hit has not earned
/// that: the ordering is trustworthy, the magnitude is not. So a
/// semantically-retrieved task can be shown and can be judged, but can never
/// block the Add button on its own.
pub fn candidate_for(store: &Store, id: &TaskId, title: &str) -> Option<Candidate> {
    let task = store.task(id)?;
    let mut score = compare(title, &task.content);
    if task.checked {
        score *= COMPLETED_PENALTY;
    }

    let project = store
        .project(&task.project_id)
        .map(|project| project.name.clone())
        .unwrap_or_default();

    Some(Candidate {
        id: task.id.clone(),
        title: task.content.clone(),
        context: if task.checked {
            format!("{project} · completed")
        } else {
            project
        },
        checked: task.checked,
        score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::project::Project;
    use crate::task::Task;
    use crate::ProjectId;
    use chrono::{DateTime, Utc};

    fn now() -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(2026, 8, 8)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
    }

    fn store() -> Store {
        let (store, _) = Store::open_at(
            tempfile::TempDir::new()
                .expect("a temporary directory")
                .keep()
                .join("planner.json"),
        );
        store
    }

    #[test]
    fn filler_words_do_not_make_two_titles_different() {
        assert_eq!(compare("Buy milk", "buy some milk"), 1.0);
        assert_eq!(compare("Call the dentist", "Call dentist"), 1.0);
        assert_eq!(compare("Book a flight", "Book the flight"), 1.0);
    }

    #[test]
    fn case_and_punctuation_are_not_meaning() {
        assert_eq!(compare("Buy milk", "BUY MILK!"), 1.0);
        assert_eq!(compare("Email Sam", "email  sam..."), 1.0);
    }

    #[test]
    fn word_order_does_not_matter() {
        assert_eq!(compare("milk buy", "buy milk"), 1.0);
    }

    #[test]
    fn plurals_and_tenses_fold_together() {
        assert_eq!(stem("groceries"), "grocery");
        assert_eq!(stem("notes"), "note");
        assert_eq!(stem("calling"), "call");
        assert_eq!(stem("booked"), "book");
        // The documented limit: a doubled consonant is left alone, because the
        // rule that would fix this one breaks "calling" -> "cal".
        assert_eq!(stem("shopping"), "shopp");
        assert_eq!(compare("Go shopping", "the shopping"), 1.0);
        // "ss" is not a plural.
        assert_eq!(stem("address"), "address");
        // Too short to be safely stripped.
        assert_eq!(stem("gas"), "gas");
    }

    #[test]
    fn a_shorter_title_inside_a_longer_one_is_similar_but_not_identical() {
        let score = compare("Milk", "Buy milk");
        assert!(score > FLOOR, "{score} should be worth showing");
        assert!(score < 1.0, "{score} should not be certain");
    }

    #[test]
    fn one_word_does_not_make_a_duplicate_out_of_an_unrelated_task() {
        // Sharing only a common verb is not similarity.
        assert!(compare("Book flights", "Book dentist") < FLOOR);
        assert!(compare("Email Sam", "Email the landlord") < FLOOR);
    }

    #[test]
    fn a_number_is_part_of_the_task() {
        // "Pay invoice 42" and "Pay invoice 43" are two different bills, and
        // dropping digits during normalisation would merge them.
        let score = compare("Pay invoice 42", "Pay invoice 43");
        assert!(score < STRONG, "{score} must not be treated as certain");
        assert!(normalise("Pay invoice 42").contains(&"42".to_string()));
    }

    #[test]
    fn synonyms_are_out_of_reach_and_that_is_the_documented_limit() {
        // This is exactly the case the language model exists to catch. If this
        // ever starts passing, the local check got better and the thresholds
        // should be revisited.
        assert!(compare("Call the plumber", "Ring the plumber") < FLOOR);
    }

    #[test]
    fn nothing_typed_matches_nothing() {
        let mut store = store();
        store.add_task(Task::new(ProjectId::inbox(), "Anything", now()));
        assert!(candidates(&store, "", None, 5, FLOOR).is_empty());
        assert!(candidates(&store, "   ", None, 5, FLOOR).is_empty());
        // A line of nothing but stopwords normalises to nothing.
        assert!(candidates(&store, "the a of", None, 5, FLOOR).is_empty());
    }

    #[test]
    fn the_task_being_edited_is_not_its_own_duplicate() {
        let mut store = store();
        let id = store.add_task(Task::new(ProjectId::inbox(), "Buy milk", now()));

        assert_eq!(candidates(&store, "Buy milk", None, 5, FLOOR).len(), 1);
        assert!(candidates(&store, "Buy milk", Some(&id), 5, FLOOR).is_empty());
    }

    #[test]
    fn a_completed_task_is_offered_but_never_beats_an_open_one() {
        let mut store = store();
        let done = store.add_task(Task::new(ProjectId::inbox(), "Buy milk", now()));
        store.add_task(Task::new(ProjectId::inbox(), "Buy milk", now()));
        store.complete_task(
            &done,
            now(),
            chrono::NaiveDate::from_ymd_opt(2026, 8, 8).unwrap(),
        );

        let found = candidates(&store, "buy milk", None, 5, FLOOR);
        assert_eq!(found.len(), 2, "a finished task is still worth mentioning");
        assert!(!found[0].checked, "the open one comes first");
        assert!(found[1].checked);
        assert!(found[1].context.contains("completed"));
    }

    #[test]
    fn a_candidate_says_which_project_it_is_in() {
        let mut store = store();
        let work = store.add_project(Project::new("Work", Color::Blue), now());
        store.add_task(Task::new(work, "Draft the report", now()));

        let found = candidates(&store, "draft report", None, 5, FLOOR);
        assert_eq!(found[0].context, "Work");
    }

    #[test]
    fn an_exact_retype_is_strong_enough_to_interrupt_over() {
        let mut store = store();
        store.add_task(Task::new(
            ProjectId::inbox(),
            "Email Sam about the lease",
            now(),
        ));

        let found = candidates(&store, "email sam about lease", None, 5, FLOOR);
        assert!(found[0].is_strong(), "scored {}", found[0].score);
    }

    #[test]
    fn a_merely_related_task_is_shown_but_does_not_interrupt() {
        let mut store = store();
        store.add_task(Task::new(ProjectId::inbox(), "Buy milk and bread", now()));

        let found = candidates(&store, "Buy bread", None, 5, FLOOR);
        assert_eq!(found.len(), 1);
        assert!(!found[0].is_strong(), "scored {}", found[0].score);
    }

    #[test]
    fn the_limit_is_honoured_and_the_best_survive_it() {
        let mut store = store();
        store.add_task(Task::new(ProjectId::inbox(), "Buy milk", now()));
        for index in 0..10 {
            store.add_task(Task::new(
                ProjectId::inbox(),
                format!("Buy milk and item {index}"),
                now(),
            ));
        }

        let found = candidates(&store, "Buy milk", None, 3, FLOOR);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].title, "Buy milk", "the exact match survives");
    }

    #[test]
    fn equal_scores_come_out_in_a_stable_order() {
        let mut store = store();
        for name in ["Buy milk today", "Buy milk later", "Buy milk now"] {
            store.add_task(Task::new(ProjectId::inbox(), name, now()));
        }
        let first = candidates(&store, "Buy milk", None, 5, FLOOR);
        let second = candidates(&store, "Buy milk", None, 5, FLOOR);
        assert_eq!(first, second);
    }

    #[test]
    fn cosine_of_unit_vectors_is_their_dot_product() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), 1.0);
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert!((cosine(&[0.6, 0.8], &[0.6, 0.8]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_vector_from_another_model_is_ignored_rather_than_fatal() {
        // A cache written by a different model has a different width. That is
        // stale data on disk, and the app must open anyway.
        assert_eq!(cosine(&[1.0, 0.0, 0.0], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
    }

    #[test]
    fn ranking_returns_the_nearest_vectors_closest_first() {
        let query = vec![1.0, 0.0];
        let vectors = vec![
            (0, vec![0.0, 1.0]),   // orthogonal
            (1, vec![1.0, 0.0]),   // identical
            (2, vec![0.7, 0.714]), // close
        ];
        assert_eq!(rank_by_vector(&query, &vectors, 3), vec![1, 2]);
        assert_eq!(rank_by_vector(&query, &vectors, 1), vec![1]);
    }

    #[test]
    fn ranking_is_stable_for_equal_scores() {
        let query = vec![1.0, 0.0];
        let vectors = vec![(7, vec![1.0, 0.0]), (3, vec![1.0, 0.0])];
        // Ties break on index, so the same query twice ranks the same way.
        assert_eq!(rank_by_vector(&query, &vectors, 2), vec![3, 7]);
    }

    #[test]
    fn a_semantically_found_task_is_never_strong_on_its_own() {
        // The ranker can surface a pair sharing almost no words. Its score
        // stays the lexical one, so it can be shown and judged but can never
        // license interrupting without a model — cosine ordering is reliable,
        // cosine magnitude is not.
        let mut store = store();
        let id = store.add_task(Task::new(
            ProjectId::inbox(),
            "Sort out the leaking tap",
            now(),
        ));

        let candidate = candidate_for(&store, &id, "Repair the dripping tap").expect("a candidate");
        assert_eq!(candidate.title, "Sort out the leaking tap");
        assert!(
            !candidate.is_strong(),
            "scored {} — a cosine hit must not block Add by itself",
            candidate.score
        );
    }

    #[test]
    fn a_candidate_for_a_task_that_is_gone_is_nothing() {
        let store = store();
        assert!(candidate_for(&store, &TaskId::from_raw("nope"), "anything").is_none());
    }

    #[test]
    fn similarity_is_symmetric() {
        for (left, right) in [
            ("Buy milk", "buy some milk"),
            ("Milk", "Buy milk for the party"),
            ("Call the plumber", "Ring the plumber"),
        ] {
            assert_eq!(
                compare(left, right),
                compare(right, left),
                "{left} / {right}"
            );
        }
    }
}

//! Quick add: one line of text becomes a task.
//!
//! ```text
//! Email Sam about the lease #Work /Admin @email p2 friday 9am !30m
//! ```
//!
//! The parse is reported as spans over the original string, not just as a
//! finished task, because the entry highlights each token as you type and has
//! to know exactly which characters to colour. It also means the title can be
//! rebuilt by deleting the spans, so nothing is stripped twice or missed.
//!
//! **Known names win over guesses.** `#My Big Project` is three words, and
//! only the store knows that. Passing the existing names in as a
//! [`Vocabulary`] lets a multi-word name match; without one, a token is a
//! single word and a new project by that name will be created.
//!
//! **A bare number is never a date.** "Buy 3 apples" must not become a task
//! called "Buy apples" due on the 3rd. The date scanner therefore refuses a
//! lone run of digits, even though `due: 3` is meaningful in a filter query
//! where there is nothing else the text could be.

use std::ops::Range;

use chrono::NaiveDate;

use super::date::{parse_date, parse_time};
use super::recurrence::parse_recurrence;
use crate::due::Due;
use crate::priority::Priority;

/// The names already in the store, so multi-word ones can be recognised.
#[derive(Debug, Default, Clone)]
pub struct Vocabulary {
    pub projects: Vec<String>,
    pub sections: Vec<String>,
    pub labels: Vec<String>,
}

/// What part of the entry a span covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Project,
    Section,
    Label,
    Priority,
    Date,
    Recurrence,
    Reminder,
}

/// A recognised token, and where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: SpanKind,
}

/// The result of reading one line of quick-add.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuickAdd {
    /// What is left once every token is removed. May be empty, which the
    /// caller should treat as "nothing to add yet" rather than as an error.
    pub title: String,
    pub project: Option<String>,
    pub section: Option<String>,
    pub labels: Vec<String>,
    pub priority: Option<Priority>,
    pub due: Option<Due>,
    /// Minutes before the due time, one per `!` token. Absolute reminders are
    /// set in the detail panel; there is no unambiguous way to type "at 9am on
    /// a day I have not named yet".
    pub reminders: Vec<i64>,
    pub spans: Vec<Span>,
}

/// One whitespace-separated word, and where it came from.
#[derive(Debug, Clone)]
struct Word {
    text: String,
    lowered: String,
    range: Range<usize>,
    taken: bool,
}

/// Read a quick-add line.
pub fn parse_quick_add(text: &str, today: NaiveDate, vocabulary: &Vocabulary) -> QuickAdd {
    let mut words = split(text);
    let mut result = QuickAdd::default();

    take_prefixed(&mut words, &mut result, vocabulary);
    take_recurrence(&mut words, &mut result, today);
    take_date(&mut words, &mut result, today);

    result.title = remaining(&words);
    result.spans.sort_by_key(|span| span.range.start);
    result
}

fn split(text: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut start = None;

    for (index, character) in text.char_indices() {
        match (character.is_whitespace(), start) {
            (false, None) => start = Some(index),
            (true, Some(begin)) => {
                words.push(word(text, begin..index));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(begin) = start {
        words.push(word(text, begin..text.len()));
    }
    words
}

fn word(text: &str, range: Range<usize>) -> Word {
    let slice = &text[range.clone()];
    Word {
        text: slice.to_string(),
        lowered: slice.to_ascii_lowercase(),
        range,
        taken: false,
    }
}

/// `#project`, `/section`, `@label`, `p1`–`p4`, `!30m`.
fn take_prefixed(words: &mut [Word], result: &mut QuickAdd, vocabulary: &Vocabulary) {
    let mut index = 0;
    while index < words.len() {
        if words[index].taken {
            index += 1;
            continue;
        }

        let (prefix, kind, known): (char, SpanKind, &[String]) =
            match words[index].text.chars().next() {
                Some('#') => ('#', SpanKind::Project, &vocabulary.projects),
                Some('/') => ('/', SpanKind::Section, &vocabulary.sections),
                Some('@') => ('@', SpanKind::Label, &vocabulary.labels),
                Some('!') => {
                    if let Some(minutes) = reminder(&words[index].lowered) {
                        result.reminders.push(minutes);
                        result.spans.push(Span {
                            range: words[index].range.clone(),
                            kind: SpanKind::Reminder,
                        });
                        words[index].taken = true;
                    }
                    index += 1;
                    continue;
                }
                _ => {
                    if let Some(priority) = Priority::from_token(&words[index].lowered) {
                        result.priority = Some(priority);
                        result.spans.push(Span {
                            range: words[index].range.clone(),
                            kind: SpanKind::Priority,
                        });
                        words[index].taken = true;
                    }
                    index += 1;
                    continue;
                }
            };

        let Some((name, span, consumed)) = named(words, index, prefix, known) else {
            index += 1;
            continue;
        };

        match kind {
            SpanKind::Project => result.project = Some(name),
            SpanKind::Section => result.section = Some(name),
            SpanKind::Label => {
                if !result.labels.iter().any(|l| l.eq_ignore_ascii_case(&name)) {
                    result.labels.push(name);
                }
            }
            _ => unreachable!("only these three kinds are prefixed"),
        }
        result.spans.push(Span { range: span, kind });
        for word in words.iter_mut().skip(index).take(consumed) {
            word.taken = true;
        }
        index += consumed;
    }
}

/// Read a `#`/`/`/`@` token, preferring the longest known name.
///
/// Returns the name, the span covering the whole token, and how many words it
/// used. An empty token — a bare `@` — is not a token at all; the user is
/// probably mid-keystroke and the picker is about to open.
fn named(
    words: &[Word],
    index: usize,
    prefix: char,
    known: &[String],
) -> Option<(String, Range<usize>, usize)> {
    let first = words[index].text.strip_prefix(prefix)?;

    // Try the longest run of words that spells a name we already have.
    let mut best: Option<(String, usize)> = None;
    for length in 1..=words.len() - index {
        if words[index + 1..index + length].iter().any(|w| w.taken) {
            break;
        }
        let mut candidate = first.to_string();
        for word in &words[index + 1..index + length] {
            candidate.push(' ');
            candidate.push_str(&word.text);
        }
        if known
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&candidate))
        {
            best = Some((candidate, length));
        }
    }

    if let Some((name, length)) = best {
        let span = words[index].range.start..words[index + length - 1].range.end;
        return Some((name, span, length));
    }

    if first.is_empty() {
        return None;
    }
    Some((first.to_string(), words[index].range.clone(), 1))
}

/// `!30m`, `!2h`, `!1d`, `!90`.
fn reminder(token: &str) -> Option<i64> {
    let body = token.strip_prefix('!')?;
    let body = body.strip_suffix("before").unwrap_or(body).trim();
    if body.is_empty() {
        return None;
    }

    let (digits, multiplier) = match body.chars().last()? {
        'm' => (&body[..body.len() - 1], 1),
        'h' => (&body[..body.len() - 1], 60),
        'd' => (&body[..body.len() - 1], 60 * 24),
        // A bare number is minutes, which is what `!30` obviously means.
        _ => (body, 1),
    };
    let count: i64 = digits.parse().ok()?;
    (count > 0).then_some(count * multiplier)
}

/// Find a repeat phrase, longest match first.
///
/// Only `every` starts one here, even though [`parse_recurrence`] also accepts
/// `daily`, `weekly` and friends. "Weekly review" and "Daily standup" are
/// ordinary task titles and far more common than anyone typing the bare
/// adverb; `every week` says the same thing and cannot be mistaken for prose.
/// The adverbs stay available to the repeat field in the detail panel, where
/// the whole string is known to be a rule.
fn take_recurrence(words: &mut [Word], result: &mut QuickAdd, today: NaiveDate) {
    let starts: Vec<usize> = (0..words.len())
        .filter(|index| {
            !words[*index].taken && matches!(words[*index].lowered.as_str(), "every" | "every!")
        })
        .collect();

    for start in starts {
        let Some((rule, length)) = longest(words, start, |phrase| parse_recurrence(phrase, today))
        else {
            continue;
        };
        let due = result
            .due
            .get_or_insert_with(|| Due::on(rule.first_occurrence(today)));
        due.recurrence = Some(rule);
        mark(words, start, length, result, SpanKind::Recurrence);
        return;
    }
}

/// Find a date phrase, and a time phrase next to it.
fn take_date(words: &mut [Word], result: &mut QuickAdd, today: NaiveDate) {
    let mut found_date = None;

    for start in 0..words.len() {
        if words[start].taken {
            continue;
        }
        if let Some((date, length)) = longest(words, start, |phrase| scan_date(phrase, today)) {
            found_date = Some((date, start, length));
            break;
        }
    }

    if let Some((date, start, length)) = found_date {
        mark(words, start, length, result, SpanKind::Date);
        match result.due.as_mut() {
            // A repeat phrase already set a first occurrence; an explicit date
            // is more specific, so it wins.
            Some(due) => due.date = date,
            None => result.due = Some(Due::on(date)),
        }
    }

    // A time is only meaningful once something is dated. `9am` on its own
    // means today at nine.
    for start in 0..words.len() {
        if words[start].taken {
            continue;
        }
        let Some((time, length)) = longest(words, start, scan_time) else {
            continue;
        };
        mark(words, start, length, result, SpanKind::Date);
        result
            .due
            .get_or_insert_with(|| Due::on(today))
            .time
            .replace(time);
        return;
    }
}

/// A date, but refusing the matches that would eat ordinary prose.
fn scan_date(phrase: &str, today: NaiveDate) -> Option<NaiveDate> {
    // "Buy 3 apples" is not due on the 3rd.
    if phrase.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    parse_date(phrase, today)
}

/// A time, but refusing a bare hour. "Chapter 9" is not nine o'clock; `9am`
/// and `at 9` are, because both say so.
fn scan_time(phrase: &str) -> Option<chrono::NaiveTime> {
    if phrase.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    parse_time(phrase)
}

/// Try phrases of decreasing length from `start`, returning the longest that
/// parses. Longest-first is what makes `next friday` beat `friday`.
fn longest<T>(
    words: &[Word],
    start: usize,
    parse: impl Fn(&str) -> Option<T>,
) -> Option<(T, usize)> {
    let limit = words[start..]
        .iter()
        .position(|word| word.taken)
        .unwrap_or(words.len() - start)
        .min(5);

    for length in (1..=limit).rev() {
        let phrase = words[start..start + length]
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(value) = parse(&phrase) {
            return Some((value, length));
        }
    }
    None
}

fn mark(words: &mut [Word], start: usize, length: usize, result: &mut QuickAdd, kind: SpanKind) {
    result.spans.push(Span {
        range: words[start].range.start..words[start + length - 1].range.end,
        kind,
    });
    for word in words.iter_mut().skip(start).take(length) {
        word.taken = true;
    }
}

/// Everything that was not a token, rejoined.
fn remaining(words: &[Word]) -> String {
    words
        .iter()
        .filter(|word| !word.taken)
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recurrence::Unit;
    use chrono::{NaiveTime, Weekday};

    /// Thursday, 30 July 2026.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn parse(text: &str) -> QuickAdd {
        parse_quick_add(text, today(), &Vocabulary::default())
    }

    fn with_vocabulary(text: &str, vocabulary: &Vocabulary) -> QuickAdd {
        parse_quick_add(text, today(), vocabulary)
    }

    #[test]
    fn a_line_with_no_tokens_is_all_title() {
        let parsed = parse("Email Sam about the lease");
        assert_eq!(parsed.title, "Email Sam about the lease");
        assert!(parsed.spans.is_empty());
        assert_eq!(parsed.due, None);
        assert_eq!(parsed.priority, None);
    }

    #[test]
    fn every_token_is_stripped_from_the_title() {
        let parsed = parse("Email Sam #Work /Admin @email p2 friday !30m");
        assert_eq!(parsed.title, "Email Sam");
        assert_eq!(parsed.project.as_deref(), Some("Work"));
        assert_eq!(parsed.section.as_deref(), Some("Admin"));
        assert_eq!(parsed.labels, vec!["email"]);
        assert_eq!(parsed.priority, Some(Priority::P2));
        assert_eq!(parsed.due.as_ref().unwrap().date, date(2026, 7, 31));
        assert_eq!(parsed.reminders, vec![30]);
    }

    #[test]
    fn tokens_are_reported_as_spans_over_the_original_text() {
        let text = "Call Sam p1 tomorrow";
        let parsed = parse(text);

        let kinds: Vec<SpanKind> = parsed.spans.iter().map(|span| span.kind).collect();
        assert_eq!(kinds, vec![SpanKind::Priority, SpanKind::Date]);
        assert_eq!(&text[parsed.spans[0].range.clone()], "p1");
        assert_eq!(&text[parsed.spans[1].range.clone()], "tomorrow");
    }

    #[test]
    fn spans_come_back_in_the_order_they_appear() {
        let parsed = parse("tomorrow Call Sam p1 @work");
        let starts: Vec<usize> = parsed.spans.iter().map(|s| s.range.start).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted);
    }

    #[test]
    fn a_multi_word_name_needs_the_vocabulary_to_be_recognised() {
        let vocabulary = Vocabulary {
            projects: vec!["My Big Project".into()],
            ..Vocabulary::default()
        };

        let parsed = with_vocabulary("Do the thing #My Big Project", &vocabulary);
        assert_eq!(parsed.project.as_deref(), Some("My Big Project"));
        assert_eq!(parsed.title, "Do the thing");
    }

    #[test]
    fn without_the_vocabulary_a_token_is_one_word_and_names_a_new_project() {
        let parsed = parse("Do the thing #My Big Project");
        assert_eq!(parsed.project.as_deref(), Some("My"));
        assert_eq!(parsed.title, "Do the thing Big Project");
    }

    #[test]
    fn a_known_name_beats_a_shorter_prefix_of_itself() {
        let vocabulary = Vocabulary {
            projects: vec!["Work".into(), "Work Admin".into()],
            ..Vocabulary::default()
        };
        let parsed = with_vocabulary("File it #Work Admin", &vocabulary);
        assert_eq!(parsed.project.as_deref(), Some("Work Admin"));
        assert_eq!(parsed.title, "File it");
    }

    #[test]
    fn several_labels_are_all_collected_and_duplicates_ignored() {
        let parsed = parse("Shop @errand @town @ERRAND");
        assert_eq!(parsed.labels, vec!["errand", "town"]);
    }

    #[test]
    fn a_bare_prefix_is_not_a_token() {
        // Mid-keystroke: the picker is opening, there is no name yet.
        let parsed = parse("Think about it @");
        assert_eq!(parsed.labels, Vec::<String>::new());
        assert_eq!(parsed.title, "Think about it @");
    }

    #[test]
    fn dates_and_times_combine() {
        let parsed = parse("Standup friday 9am");
        let due = parsed.due.expect("a due date");
        assert_eq!(due.date, date(2026, 7, 31));
        assert_eq!(due.time, NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(parsed.title, "Standup");
    }

    #[test]
    fn a_time_on_its_own_means_today() {
        let parsed = parse("Standup 9am");
        let due = parsed.due.expect("a due date");
        assert_eq!(due.date, today());
        assert_eq!(due.time, NaiveTime::from_hms_opt(9, 0, 0));
    }

    #[test]
    fn the_longest_date_phrase_wins() {
        let parsed = parse("Review next friday");
        assert_eq!(parsed.due.unwrap().date, date(2026, 8, 7));
        assert_eq!(parse("Review friday").due.unwrap().date, date(2026, 7, 31));
    }

    #[test]
    fn a_bare_number_is_not_a_date() {
        let parsed = parse("Buy 3 apples");
        assert_eq!(parsed.title, "Buy 3 apples");
        assert_eq!(parsed.due, None);
    }

    #[test]
    fn a_bare_number_is_not_a_time_either() {
        let parsed = parse("Read chapter 9");
        assert_eq!(parsed.title, "Read chapter 9");
        assert_eq!(parsed.due, None);
    }

    #[test]
    fn an_ordinal_is_a_date_because_it_says_so() {
        let parsed = parse("Rent 1st");
        assert_eq!(parsed.due.unwrap().date, date(2026, 8, 1));
        assert_eq!(parsed.title, "Rent");
    }

    #[test]
    fn a_repeat_phrase_sets_the_rule_and_the_first_occurrence() {
        let parsed = parse("Bins every monday");
        let due = parsed.due.expect("a due date");
        assert_eq!(due.date, date(2026, 8, 3));
        assert_eq!(
            due.recurrence,
            Some(crate::Recurrence::weekly_on(1, [Weekday::Mon]))
        );
        assert_eq!(parsed.title, "Bins");
    }

    #[test]
    fn an_explicit_date_beats_the_repeats_own_first_occurrence() {
        let parsed = parse("Bins every week 3 august");
        let due = parsed.due.expect("a due date");
        assert_eq!(due.date, date(2026, 8, 3));
        assert_eq!(due.recurrence.unwrap().unit, Unit::Week);
        assert_eq!(parsed.title, "Bins");
    }

    #[test]
    fn a_bare_adverb_is_a_title_not_a_repeat() {
        // "Weekly review" is a task, not a rule. `every week` is how you say
        // the rule, and it cannot be mistaken for prose.
        let parsed = parse("Weekly review");
        assert_eq!(parsed.title, "Weekly review");
        assert_eq!(parsed.due, None);

        let parsed = parse("Daily standup every weekday");
        assert_eq!(parsed.title, "Daily standup");
        assert_eq!(
            parsed.due.unwrap().recurrence,
            Some(crate::Recurrence::every_weekday())
        );
    }

    #[test]
    fn the_bang_form_survives_the_round_trip() {
        let parsed = parse("Water the plants every! 10 days");
        let rule = parsed.due.unwrap().recurrence.expect("a rule");
        assert!(rule.from_completion);
        assert_eq!(rule.interval, 10);
        assert_eq!(parsed.title, "Water the plants");
    }

    #[test]
    fn reminders_in_every_unit() {
        assert_eq!(parse("A !30m").reminders, vec![30]);
        assert_eq!(parse("A !2h").reminders, vec![120]);
        assert_eq!(parse("A !1d").reminders, vec![1440]);
        assert_eq!(parse("A !45").reminders, vec![45]);
        assert_eq!(parse("A !30m !2h").reminders, vec![30, 120]);
    }

    #[test]
    fn a_meaningless_reminder_is_left_in_the_title() {
        let parsed = parse("Shout !!! today");
        assert_eq!(parsed.title, "Shout !!!");
        assert!(parsed.reminders.is_empty());
    }

    #[test]
    fn priority_is_recognised_anywhere_and_only_as_its_own_word() {
        assert_eq!(parse("p1 Call Sam").priority, Some(Priority::P1));
        assert_eq!(parse("Call Sam p4").priority, Some(Priority::P4));
        // Part of a word is not a token.
        let parsed = parse("Review p1s report");
        assert_eq!(parsed.priority, None);
        assert_eq!(parsed.title, "Review p1s report");
    }

    #[test]
    fn an_empty_line_parses_to_nothing_rather_than_failing() {
        let parsed = parse("   ");
        assert_eq!(parsed.title, "");
        assert_eq!(parsed.due, None);
        assert!(parsed.spans.is_empty());
    }

    #[test]
    fn a_line_that_is_only_tokens_leaves_an_empty_title() {
        let parsed = parse("#Work p1 tomorrow");
        assert_eq!(parsed.title, "");
        assert_eq!(parsed.project.as_deref(), Some("Work"));
    }

    #[test]
    fn spans_line_up_with_the_original_bytes_when_the_title_is_not_ascii() {
        let text = "Café review tomorrow p1";
        let parsed = parse_quick_add(text, today(), &Vocabulary::default());
        for span in &parsed.spans {
            // Slicing must not land inside a character.
            let slice = &text[span.range.clone()];
            assert!(matches!(slice, "tomorrow" | "p1"), "got {slice:?}");
        }
        assert_eq!(parsed.title, "Café review");
    }
}

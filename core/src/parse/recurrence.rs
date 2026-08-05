//! Repeat rules written in English.
//!
//! `every other monday`, `every 3 days until 1 september`, `every! 10 days`.
//!
//! The trailing `!` is the whole reason this is worth parsing rather than
//! offering a dialog: it is the one piece of recurrence semantics that has no
//! obvious widget. `every 10 days` and `every! 10 days` differ only in whether
//! the clock restarts when you finish, and typing one character is a better
//! way to say that than a radio button labelled "relative to completion".

use chrono::{NaiveDate, Weekday};

use super::date::{normalise, parse_date, singular, weekday_from_name};
use crate::recurrence::{End, Recurrence, Unit};

/// Parse a complete repeat phrase. The whole string must be consumed.
pub fn parse_recurrence(text: &str, today: NaiveDate) -> Option<Recurrence> {
    let words = normalise(text);
    let (lead, rest) = words.split_first()?;

    let from_completion = match lead.as_str() {
        "every" => false,
        "every!" => true,
        // "daily", "weekly" and friends are the same rules said differently.
        "daily" => return finished(Recurrence::every(1, Unit::Day), rest, today),
        "weekly" => return finished(Recurrence::every(1, Unit::Week), rest, today),
        "monthly" => return finished(Recurrence::every(1, Unit::Month), rest, today),
        "yearly" | "annually" => return finished(Recurrence::every(1, Unit::Year), rest, today),
        _ => return None,
    };

    let (interval, rest) = interval(rest);
    let (mut rule, rest) = body(interval, rest)?;
    rule.from_completion = from_completion;

    finished(rule, rest, today)
}

/// `other` means two; a bare number means itself; anything else means one and
/// consumes nothing.
fn interval(words: &[String]) -> (u32, &[String]) {
    match words.split_first() {
        Some((first, rest)) if first == "other" => (2, rest),
        Some((first, rest)) => match first.parse::<u32>() {
            Ok(count) if count >= 1 => (count, rest),
            _ => (1, words),
        },
        None => (1, words),
    }
}

/// The unit or weekday list at the heart of the rule.
fn body(interval: u32, words: &[String]) -> Option<(Recurrence, &[String])> {
    let (first, rest) = words.split_first()?;

    if singular(first) == "weekday" {
        let mut rule = Recurrence::every_weekday();
        rule.interval = interval;
        return Some((rule, rest));
    }

    if weekday_from_name(first).is_some() {
        return weekday_list(interval, words);
    }

    let unit = match singular(first) {
        "day" => Unit::Day,
        "week" => Unit::Week,
        "month" => Unit::Month,
        "year" => Unit::Year,
        _ => return None,
    };
    Some((Recurrence::every(interval, unit), rest))
}

/// `monday`, `mon and fri`, `mon, tue, fri`.
fn weekday_list(interval: u32, words: &[String]) -> Option<(Recurrence, &[String])> {
    let mut days: Vec<Weekday> = Vec::new();
    let mut rest = words;

    while let Some((first, tail)) = rest.split_first() {
        // "and" only joins; it never starts or ends a list.
        if first == "and" && !days.is_empty() {
            rest = tail;
            continue;
        }
        match weekday_from_name(first) {
            Some(day) => {
                days.push(day);
                rest = tail;
            }
            None => break,
        }
    }

    if days.is_empty() {
        return None;
    }
    Some((Recurrence::weekly_on(interval, days), rest))
}

/// Apply a trailing end clause, if there is one. Anything left over that is
/// not an end clause means the phrase was not a repeat rule after all.
fn finished(mut rule: Recurrence, rest: &[String], today: NaiveDate) -> Option<Recurrence> {
    if rest.is_empty() {
        return Some(rule);
    }

    let (first, tail) = rest.split_first()?;

    // "x3", "×3"
    if let Some(count) = first.strip_prefix('x').and_then(|n| n.parse::<u32>().ok()) {
        if !tail.is_empty() || count == 0 {
            return None;
        }
        rule.end = End::After {
            remaining: count - 1,
        };
        return Some(rule);
    }

    match first.as_str() {
        "until" | "till" | "ending" | "ends" => {
            let phrase = tail.join(" ");
            let date = parse_date(&phrase, today)?;
            rule.end = End::OnDate { date };
            Some(rule)
        }
        "for" => match tail {
            [count, unit] if singular(unit) == "time" => {
                let count: u32 = count.parse().ok()?;
                if count == 0 {
                    return None;
                }
                rule.end = End::After {
                    remaining: count - 1,
                };
                Some(rule)
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thursday, 30 July 2026.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
    }

    fn parse(text: &str) -> Option<Recurrence> {
        parse_recurrence(text, today())
    }

    #[test]
    fn plain_intervals() {
        assert_eq!(parse("every day"), Some(Recurrence::every(1, Unit::Day)));
        assert_eq!(parse("every 3 days"), Some(Recurrence::every(3, Unit::Day)));
        assert_eq!(parse("every week"), Some(Recurrence::every(1, Unit::Week)));
        assert_eq!(
            parse("every month"),
            Some(Recurrence::every(1, Unit::Month))
        );
        assert_eq!(parse("every year"), Some(Recurrence::every(1, Unit::Year)));
    }

    #[test]
    fn other_means_every_second_one() {
        assert_eq!(
            parse("every other day"),
            Some(Recurrence::every(2, Unit::Day))
        );
        assert_eq!(
            parse("every other week"),
            Some(Recurrence::every(2, Unit::Week))
        );
    }

    #[test]
    fn the_adverbs_are_the_same_rules_said_differently() {
        assert_eq!(parse("daily"), parse("every day"));
        assert_eq!(parse("weekly"), parse("every week"));
        assert_eq!(parse("monthly"), parse("every month"));
        assert_eq!(parse("yearly"), parse("every year"));
    }

    #[test]
    fn named_weekdays() {
        assert_eq!(
            parse("every monday"),
            Some(Recurrence::weekly_on(1, [Weekday::Mon]))
        );
        assert_eq!(
            parse("every other tuesday"),
            Some(Recurrence::weekly_on(2, [Weekday::Tue]))
        );
    }

    #[test]
    fn several_weekdays_joined_either_way() {
        let expected = Some(Recurrence::weekly_on(1, [Weekday::Mon, Weekday::Fri]));
        assert_eq!(parse("every mon, fri"), expected);
        assert_eq!(parse("every mon and fri"), expected);
        assert_eq!(parse("every monday friday"), expected);
    }

    #[test]
    fn every_weekday_is_monday_to_friday() {
        assert_eq!(parse("every weekday"), Some(Recurrence::every_weekday()));
        assert_eq!(parse("every weekdays"), Some(Recurrence::every_weekday()));
    }

    #[test]
    fn the_bang_marks_a_rule_that_runs_from_completion() {
        let rule = parse("every! 10 days").expect("a rule");
        assert!(rule.from_completion);
        assert_eq!(rule.interval, 10);
        assert_eq!(rule.unit, Unit::Day);

        assert!(!parse("every 10 days").unwrap().from_completion);
    }

    #[test]
    fn an_until_clause_sets_an_end_date() {
        let rule = parse("every week until 1 september").expect("a rule");
        assert_eq!(
            rule.end,
            End::OnDate {
                date: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
            }
        );
    }

    #[test]
    fn a_count_clause_counts_the_occurrences_not_the_repeats() {
        // Three occurrences: the due date, plus two more.
        assert_eq!(
            parse("every day x3").unwrap().end,
            End::After { remaining: 2 }
        );
        assert_eq!(
            parse("every day for 3 times").unwrap().end,
            End::After { remaining: 2 }
        );
    }

    #[test]
    fn a_first_occurrence_is_the_next_named_weekday() {
        // Today is Thursday.
        let rule = parse("every monday").unwrap();
        assert_eq!(
            rule.first_occurrence(today()),
            NaiveDate::from_ymd_opt(2026, 8, 3).unwrap()
        );
        // Today itself counts if it is one of the named days.
        let rule = parse("every thursday").unwrap();
        assert_eq!(rule.first_occurrence(today()), today());
        // With no day named there is nothing to wait for.
        assert_eq!(
            parse("every week").unwrap().first_occurrence(today()),
            today()
        );
    }

    #[test]
    fn nonsense_is_not_a_rule() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("every"), None);
        assert_eq!(parse("every fortnight"), None);
        assert_eq!(parse("evening"), None);
        assert_eq!(parse("every day and a half"), None);
        assert_eq!(parse("every day until lunchtime"), None);
        assert_eq!(parse("every day x0"), None);
    }
}

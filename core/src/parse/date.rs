//! English dates and times, parsed from what a person types.
//!
//! Shared deliberately between quick-add and the filter query language. If
//! `due:friday` in a filter meant something different from `friday` typed into
//! the entry, the app would be lying to the user about its own vocabulary —
//! so there is one parser and one set of rules.
//!
//! Everything takes `today` as an argument. Nothing here reads the clock.
//!
//! A bare weekday means the coming one, and today if today *is* that day.
//! `next friday` always means the one after that. People who mean "a week
//! tomorrow" say "next"; people who say "friday" on a Friday morning mean
//! today.

use chrono::{Datelike, Days, Months, NaiveDate, NaiveTime, Weekday};

/// Parse a complete date phrase. The whole string must be consumed.
pub fn parse_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let words = normalise(text);
    if words.is_empty() {
        return None;
    }
    date_from_words(&words, today)
}

/// Parse a complete time phrase, such as `9am`, `17:30`, `noon`.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let words = normalise(text);
    if words.is_empty() {
        return None;
    }
    time_from_words(&words)
}

/// Lower-case, split on whitespace, and drop punctuation that carries no
/// meaning. `Friday,` and `friday` are the same word.
pub(crate) fn normalise(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| c == ',' || c == '.' || c == ';')
                .to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn date_from_words(words: &[String], today: NaiveDate) -> Option<NaiveDate> {
    let joined = words.join(" ");

    match joined.as_str() {
        "today" | "tod" => return Some(today),
        "tomorrow" | "tom" | "tmr" => return today.checked_add_days(Days::new(1)),
        "yesterday" => return today.checked_sub_days(Days::new(1)),
        "next week" => return today.checked_add_days(Days::new(7)),
        "next month" => return today.checked_add_months(Months::new(1)),
        "next year" => return today.checked_add_months(Months::new(12)),
        "end of week" => return end_of_week(today),
        "end of month" => return end_of_month(today),
        "end of year" => return NaiveDate::from_ymd_opt(today.year(), 12, 31),
        _ => {}
    }

    // ISO, the one unambiguous numeric form. `27/07` is not accepted: whether
    // it means July or the 7th of the month depends on where you grew up, and
    // guessing wrong silently files the task five months away.
    if let Ok(date) = NaiveDate::parse_from_str(&joined, "%Y-%m-%d") {
        return Some(date);
    }

    // "friday", "next friday"
    if let Some(weekday) = weekday_from_words(words) {
        let (weekday, skip_a_week) = weekday;
        return next_weekday(today, weekday, skip_a_week);
    }

    // "in 3 days", "in 2 weeks"
    if let Some(date) = relative(words, today) {
        return Some(date);
    }

    // "27th", "3 july", "july 3", "3 jul 2027"
    day_and_month(words, today)
}

/// `friday` -> (Fri, false); `next friday` -> (Fri, true).
fn weekday_from_words(words: &[String]) -> Option<(Weekday, bool)> {
    match words {
        [day] => weekday_from_name(day).map(|weekday| (weekday, false)),
        [first, day] if first == "next" => weekday_from_name(day).map(|weekday| (weekday, true)),
        [first, second, day] if first == "this" && second == "coming" => {
            weekday_from_name(day).map(|weekday| (weekday, false))
        }
        _ => None,
    }
}

pub(crate) fn weekday_from_name(word: &str) -> Option<Weekday> {
    Some(match word {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" | "tues" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" | "thur" | "thurs" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// The next `weekday` on or after `today`, or a week later again if
/// `skip_a_week`.
fn next_weekday(today: NaiveDate, weekday: Weekday, skip_a_week: bool) -> Option<NaiveDate> {
    let current = today.weekday().num_days_from_monday() as i64;
    let target = weekday.num_days_from_monday() as i64;
    let mut ahead = (target - current).rem_euclid(7);
    if skip_a_week {
        // "next friday" on a Thursday is eight days off, not tomorrow; on a
        // Friday it is seven, not today.
        ahead += 7;
    }
    today.checked_add_days(Days::new(ahead as u64))
}

/// "in 3 days", "in 2 weeks", "in a month".
fn relative(words: &[String], today: NaiveDate) -> Option<NaiveDate> {
    let [lead, count, unit] = words else {
        return None;
    };
    if lead != "in" {
        return None;
    }
    let count: u32 = if count == "a" || count == "an" {
        1
    } else {
        count.parse().ok()?
    };

    match singular(unit) {
        "day" => today.checked_add_days(Days::new(count as u64)),
        "week" => today.checked_add_days(Days::new(count as u64 * 7)),
        "month" => today.checked_add_months(Months::new(count)),
        "year" => today.checked_add_months(Months::new(count.checked_mul(12)?)),
        _ => None,
    }
}

pub(crate) fn singular(word: &str) -> &str {
    word.strip_suffix('s').unwrap_or(word)
}

/// "27th", "3 july", "july 3", "27 jul 2027".
///
/// A bare day number means the next time that day comes round — typing `27th`
/// on the 30th means next month, not three days ago.
fn day_and_month(words: &[String], today: NaiveDate) -> Option<NaiveDate> {
    let (day, month, year) = match words {
        [day] => (day_number(day)?, None, None),
        [a, b] => match (day_number(a), month_from_name(b)) {
            (Some(day), Some(month)) => (day, Some(month), None),
            _ => {
                let month = month_from_name(a)?;
                (day_number(b)?, Some(month), None)
            }
        },
        [a, b, c] => {
            let year: i32 = c.parse().ok()?;
            match (day_number(a), month_from_name(b)) {
                (Some(day), Some(month)) => (day, Some(month), Some(year)),
                _ => {
                    let month = month_from_name(a)?;
                    (day_number(b)?, Some(month), Some(year))
                }
            }
        }
        _ => return None,
    };

    match (month, year) {
        (Some(month), Some(year)) => NaiveDate::from_ymd_opt(year, month, day),
        (Some(month), None) => {
            let candidate = NaiveDate::from_ymd_opt(today.year(), month, day)?;
            if candidate < today {
                NaiveDate::from_ymd_opt(today.year() + 1, month, day)
            } else {
                Some(candidate)
            }
        }
        (None, _) => {
            let candidate = NaiveDate::from_ymd_opt(today.year(), today.month(), day);
            match candidate {
                Some(date) if date >= today => Some(date),
                // Either the day has passed this month or this month is too
                // short for it. Both mean "next month".
                _ => {
                    let next = today.checked_add_months(Months::new(1))?;
                    NaiveDate::from_ymd_opt(next.year(), next.month(), day)
                }
            }
        }
    }
}

/// `27`, `27th`, `3rd`, `1st`, `2nd`.
fn day_number(word: &str) -> Option<u32> {
    let digits = word
        .strip_suffix("st")
        .or_else(|| word.strip_suffix("nd"))
        .or_else(|| word.strip_suffix("rd"))
        .or_else(|| word.strip_suffix("th"))
        .unwrap_or(word);
    let day: u32 = digits.parse().ok()?;
    (1..=31).contains(&day).then_some(day)
}

fn month_from_name(word: &str) -> Option<u32> {
    Some(match word {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    })
}

fn end_of_week(today: NaiveDate) -> Option<NaiveDate> {
    let ahead = 6 - today.weekday().num_days_from_monday() as u64;
    today.checked_add_days(Days::new(ahead))
}

fn end_of_month(today: NaiveDate) -> Option<NaiveDate> {
    let first_of_next = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?
        .checked_add_months(Months::new(1))?;
    first_of_next.checked_sub_days(Days::new(1))
}

fn time_from_words(words: &[String]) -> Option<NaiveTime> {
    let joined = words.join(" ");

    match joined.as_str() {
        "noon" | "midday" => return NaiveTime::from_hms_opt(12, 0, 0),
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        "morning" | "in the morning" => return NaiveTime::from_hms_opt(9, 0, 0),
        "afternoon" | "in the afternoon" => return NaiveTime::from_hms_opt(14, 0, 0),
        "evening" | "in the evening" => return NaiveTime::from_hms_opt(18, 0, 0),
        "night" | "tonight" | "at night" => return NaiveTime::from_hms_opt(20, 0, 0),
        _ => {}
    }

    // "at 9", "at 9am", "@ 5pm"
    let rest = match words {
        [lead, rest] if lead == "at" || lead == "@" => rest.as_str(),
        [only] => only.as_str(),
        _ => return None,
    };

    clock(rest)
}

/// `9`, `9am`, `9pm`, `9:30`, `9.30pm`, `21:00`.
fn clock(text: &str) -> Option<NaiveTime> {
    let (digits, suffix) = if let Some(stripped) = text.strip_suffix("am") {
        (stripped, Some(false))
    } else if let Some(stripped) = text.strip_suffix("pm") {
        (stripped, Some(true))
    } else {
        (text, None)
    };
    let digits = digits.trim();

    let (hour, minute) = match digits.split_once([':', '.']) {
        Some((hour, minute)) => (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?),
        None => (digits.parse::<u32>().ok()?, 0),
    };

    if minute > 59 {
        return None;
    }

    let hour = match suffix {
        // 12am is midnight and 12pm is noon; every other hour just shifts.
        Some(true) if hour == 12 => 12,
        Some(true) if hour < 12 => hour + 12,
        Some(true) => return None,
        Some(false) if hour == 12 => 0,
        Some(false) if hour < 12 => hour,
        Some(false) => return None,
        // A bare number with no am/pm is only a time if it could be one.
        // `27` is a day of the month, not 27 o'clock.
        None if hour < 24 => hour,
        None => return None,
    };

    NaiveTime::from_hms_opt(hour, minute, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thursday, 30 July 2026.
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn parse(text: &str) -> Option<NaiveDate> {
        parse_date(text, today())
    }

    #[test]
    fn the_obvious_words() {
        assert_eq!(parse("today"), Some(date(2026, 7, 30)));
        assert_eq!(parse("tod"), Some(date(2026, 7, 30)));
        assert_eq!(parse("tomorrow"), Some(date(2026, 7, 31)));
        assert_eq!(parse("tom"), Some(date(2026, 7, 31)));
        assert_eq!(parse("yesterday"), Some(date(2026, 7, 29)));
    }

    #[test]
    fn parsing_ignores_case_and_trailing_punctuation() {
        assert_eq!(parse("Tomorrow,"), Some(date(2026, 7, 31)));
        assert_eq!(parse("  FRIDAY  "), parse("friday"));
    }

    #[test]
    fn a_bare_weekday_means_the_coming_one() {
        // Today is Thursday.
        assert_eq!(parse("friday"), Some(date(2026, 7, 31)));
        assert_eq!(parse("fri"), Some(date(2026, 7, 31)));
        assert_eq!(parse("monday"), Some(date(2026, 8, 3)));
        assert_eq!(parse("wednesday"), Some(date(2026, 8, 5)));
    }

    #[test]
    fn todays_own_weekday_means_today_not_a_week_away() {
        assert_eq!(parse("thursday"), Some(date(2026, 7, 30)));
    }

    #[test]
    fn next_weekday_always_skips_a_week() {
        assert_eq!(parse("next friday"), Some(date(2026, 8, 7)));
        // And on the day itself, "next thursday" is a week off, not today.
        assert_eq!(parse("next thursday"), Some(date(2026, 8, 6)));
    }

    #[test]
    fn relative_offsets() {
        assert_eq!(parse("in 3 days"), Some(date(2026, 8, 2)));
        assert_eq!(parse("in 2 weeks"), Some(date(2026, 8, 13)));
        assert_eq!(parse("in 1 month"), Some(date(2026, 8, 30)));
        assert_eq!(parse("in a month"), Some(date(2026, 8, 30)));
        assert_eq!(parse("in 1 year"), Some(date(2027, 7, 30)));
    }

    #[test]
    fn next_week_and_next_month() {
        assert_eq!(parse("next week"), Some(date(2026, 8, 6)));
        assert_eq!(parse("next month"), Some(date(2026, 8, 30)));
        assert_eq!(parse("next year"), Some(date(2027, 7, 30)));
    }

    #[test]
    fn end_of_period() {
        // The week ends on Sunday.
        assert_eq!(parse("end of week"), Some(date(2026, 8, 2)));
        assert_eq!(parse("end of month"), Some(date(2026, 7, 31)));
        assert_eq!(parse("end of year"), Some(date(2026, 12, 31)));
    }

    #[test]
    fn end_of_month_handles_february() {
        let february = date(2026, 2, 10);
        assert_eq!(
            parse_date("end of month", february),
            Some(date(2026, 2, 28))
        );
        let leap = date(2028, 2, 10);
        assert_eq!(parse_date("end of month", leap), Some(date(2028, 2, 29)));
    }

    #[test]
    fn a_bare_day_number_never_lands_in_the_past() {
        // The 27th has gone, so it means next month.
        assert_eq!(parse("27th"), Some(date(2026, 8, 27)));
        // The 31st has not.
        assert_eq!(parse("31st"), Some(date(2026, 7, 31)));
        assert_eq!(parse("30th"), Some(date(2026, 7, 30)));
    }

    #[test]
    fn a_day_number_too_large_for_this_month_rolls_to_the_next() {
        // 31 September does not exist, so "31st" in September means October.
        assert_eq!(
            parse_date("31st", date(2026, 9, 15)),
            Some(date(2026, 10, 31))
        );
    }

    #[test]
    fn day_and_month_in_either_order() {
        assert_eq!(parse("3 august"), Some(date(2026, 8, 3)));
        assert_eq!(parse("august 3"), Some(date(2026, 8, 3)));
        assert_eq!(parse("3rd aug"), Some(date(2026, 8, 3)));
        assert_eq!(parse("aug 3rd"), Some(date(2026, 8, 3)));
    }

    #[test]
    fn a_month_that_has_passed_means_next_year() {
        assert_eq!(parse("3 january"), Some(date(2027, 1, 3)));
    }

    #[test]
    fn an_explicit_year_is_taken_at_face_value() {
        assert_eq!(parse("3 january 2026"), Some(date(2026, 1, 3)));
        assert_eq!(parse("2026-01-03"), Some(date(2026, 1, 3)));
    }

    #[test]
    fn ambiguous_numeric_dates_are_refused_rather_than_guessed() {
        // 03/07 is the 3rd of July or the 7th of March depending on where you
        // are. Filing a task five months out silently is worse than refusing.
        assert_eq!(parse("03/07/2026"), None);
        assert_eq!(parse("3/7"), None);
    }

    #[test]
    fn nonsense_is_not_a_date() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("lunch"), None);
        assert_eq!(parse("in 3 fortnights"), None);
        assert_eq!(parse("32nd"), None);
        assert_eq!(parse("next lunchtime"), None);
    }

    #[test]
    fn times_of_day() {
        assert_eq!(parse_time("9am"), NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(parse_time("9pm"), NaiveTime::from_hms_opt(21, 0, 0));
        assert_eq!(parse_time("at 5pm"), NaiveTime::from_hms_opt(17, 0, 0));
        assert_eq!(parse_time("9:30"), NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(parse_time("9.30pm"), NaiveTime::from_hms_opt(21, 30, 0));
        assert_eq!(parse_time("17:00"), NaiveTime::from_hms_opt(17, 0, 0));
    }

    #[test]
    fn twelve_o_clock_goes_the_right_way_round() {
        assert_eq!(parse_time("12am"), NaiveTime::from_hms_opt(0, 0, 0));
        assert_eq!(parse_time("12pm"), NaiveTime::from_hms_opt(12, 0, 0));
        assert_eq!(parse_time("noon"), NaiveTime::from_hms_opt(12, 0, 0));
        assert_eq!(parse_time("midnight"), NaiveTime::from_hms_opt(0, 0, 0));
    }

    #[test]
    fn vague_times_of_day_get_a_sensible_hour() {
        assert_eq!(parse_time("morning"), NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(parse_time("afternoon"), NaiveTime::from_hms_opt(14, 0, 0));
        assert_eq!(parse_time("evening"), NaiveTime::from_hms_opt(18, 0, 0));
        assert_eq!(parse_time("tonight"), NaiveTime::from_hms_opt(20, 0, 0));
    }

    #[test]
    fn nonsense_is_not_a_time() {
        assert_eq!(parse_time("27"), None, "a day of the month is not an hour");
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("9:75"), None);
        assert_eq!(parse_time("13pm"), None);
        assert_eq!(parse_time("lunch"), None);
    }
}

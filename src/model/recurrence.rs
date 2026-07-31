//! Recurrence rules.
//!
//! Deliberately not RFC 5545. An `RRULE` can express things no user will ever
//! type and that this app has no way to enter — the rule set here is exactly
//! what the quick-add syntax can produce and nothing more. If a CalDAV source
//! is ever added it will translate at the seam, which is the right place for
//! the impedance mismatch to live.
//!
//! **`every` and `every!` are different rules.** `every monday` is anchored to
//! the due date: complete it three weeks late and the next one is still the
//! coming Monday. `every! 10 days` is anchored to *completion*: water the
//! plants ten days after you last actually did it, not ten days after you were
//! meant to. Getting this wrong makes a recurring task either nag forever or
//! silently drift, and which one the user wants is not guessable — so it is
//! recorded, not inferred.
//!
//! **Everything is a date, never a timestamp.** The clock time on a recurring
//! task is carried alongside and never recomputed, so a rule cannot walk an
//! 09:00 task to 08:00 across a daylight-saving boundary.

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

/// The unit a recurrence steps in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unit {
    Day,
    Week,
    Month,
    Year,
}

/// When a recurrence stops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum End {
    /// Repeat indefinitely.
    #[default]
    Never,
    /// Repeat until this date, inclusive. An occurrence falling after it is
    /// not produced.
    OnDate { date: NaiveDate },
    /// Repeat this many more times.
    ///
    /// The count is what *remains* — occurrences still to come, not counting
    /// the one currently due — so it is decremented as the rule advances and
    /// needs no separate tally. A parser turning "every day ×3" into a rule
    /// therefore sets `remaining` to 2, because the due date is the first of
    /// the three. Zero means the rule is spent.
    After { remaining: u32 },
}

/// A repeat rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recurrence {
    /// How many units to step. `2` with [`Unit::Week`] is "every other week".
    pub interval: u32,
    pub unit: Unit,
    /// Which weekdays the rule lands on. Only meaningful with [`Unit::Week`];
    /// empty means "the same weekday as the anchor". Kept sorted and unique.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weekdays: Vec<Weekday>,
    /// `every!` rather than `every`: step from the completion date.
    #[serde(default, skip_serializing_if = "is_false")]
    pub from_completion: bool,
    #[serde(default, skip_serializing_if = "is_never")]
    pub end: End,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_never(end: &End) -> bool {
    *end == End::Never
}

impl Recurrence {
    /// A simple "every N units" rule with no weekday set and no end.
    pub fn every(interval: u32, unit: Unit) -> Self {
        Self {
            interval: interval.max(1),
            unit,
            weekdays: Vec::new(),
            from_completion: false,
            end: End::Never,
        }
    }

    /// "Every Monday", "every Mon and Fri", "every other Tuesday".
    pub fn weekly_on(interval: u32, weekdays: impl IntoIterator<Item = Weekday>) -> Self {
        let mut rule = Self::every(interval, Unit::Week);
        rule.set_weekdays(weekdays);
        rule
    }

    /// Every Monday through Friday.
    pub fn every_weekday() -> Self {
        Self::weekly_on(
            1,
            [
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
            ],
        )
    }

    /// Replace the weekday set, normalising it. Sorting and de-duplicating on
    /// the way in means every later comparison and every rendered rule string
    /// is stable, whatever order the parser happened to see the days in.
    pub fn set_weekdays(&mut self, weekdays: impl IntoIterator<Item = Weekday>) {
        let mut days: Vec<Weekday> = weekdays.into_iter().collect();
        days.sort_by_key(|d| d.num_days_from_monday());
        days.dedup();
        self.weekdays = days;
    }

    /// Step from `anchor` to the next date this rule produces, ignoring the
    /// end condition. Always strictly after `anchor`.
    pub fn next_after(&self, anchor: NaiveDate) -> Option<NaiveDate> {
        let interval = self.interval.max(1);
        match self.unit {
            Unit::Day => anchor.checked_add_days(Days::new(interval as u64)),
            Unit::Week if self.weekdays.is_empty() => {
                anchor.checked_add_days(Days::new(interval as u64 * 7))
            }
            Unit::Week => self.next_weekday_after(anchor, interval),
            Unit::Month => add_months_clamped(anchor, interval),
            Unit::Year => add_months_clamped(anchor, interval.saturating_mul(12)),
        }
    }

    /// The weekday-set case: the next listed day later in the same week, or
    /// else the first listed day `interval` weeks on.
    ///
    /// Handling the same-week case first is what makes "every Mon, Fri" land
    /// on Friday rather than skipping to next Monday. The week-jump is
    /// measured from the *start* of the anchor's week, so a rule with several
    /// days in it cannot creep forward by an extra week each time it wraps.
    fn next_weekday_after(&self, anchor: NaiveDate, interval: u32) -> Option<NaiveDate> {
        let anchor_index = anchor.weekday().num_days_from_monday();

        if let Some(next) = self
            .weekdays
            .iter()
            .map(|d| d.num_days_from_monday())
            .find(|index| *index > anchor_index)
        {
            return anchor.checked_add_days(Days::new((next - anchor_index) as u64));
        }

        let week_start = anchor.checked_sub_days(Days::new(anchor_index as u64))?;
        let first = self.weekdays.first()?.num_days_from_monday();
        week_start.checked_add_days(Days::new(interval as u64 * 7 + first as u64))
    }

    /// The first date this rule should land on, for a task given a repeat but
    /// no due date of its own.
    ///
    /// "Every Monday" typed on a Thursday means the coming Monday; "every
    /// week" with no day named means starting today, because there is nothing
    /// else it could mean.
    pub fn first_occurrence(&self, today: NaiveDate) -> NaiveDate {
        if self.unit != Unit::Week || self.weekdays.is_empty() {
            return today;
        }
        let current = today.weekday().num_days_from_monday();
        self.weekdays
            .iter()
            .map(|day| day.num_days_from_monday())
            .map(|target| (target as i64 - current as i64).rem_euclid(7) as u64)
            .min()
            .and_then(|ahead| today.checked_add_days(Days::new(ahead)))
            .unwrap_or(today)
    }

    /// The rule written back out in the language it is typed in.
    ///
    /// The output is always re-parseable by
    /// [`parse_recurrence`](crate::model::parse::parse_recurrence) into an
    /// equal rule, and there is a test that says so. That is what lets the
    /// editor prefill its box with the current rule: what you are shown is
    /// what you would have typed, so editing a repeat is editing a phrase
    /// rather than decoding a summary someone wrote for display.
    ///
    /// Lower case, and no leading capital, because it is a fragment — it
    /// reads as "Mon · every week" on a row and as "every week" in the box.
    pub fn describe(&self) -> String {
        let mut text = if self.from_completion {
            "every!"
        } else {
            "every"
        }
        .to_string();

        match self.interval.max(1) {
            1 => {}
            2 => text.push_str(" other"),
            n => text.push_str(&format!(" {n}")),
        }
        let plural = self.interval > 2;

        text.push(' ');
        text.push_str(&self.body(plural));

        match &self.end {
            End::Never => {}
            // The year is always written, even for a date this year, so the
            // phrase means the same thing read back in January.
            End::OnDate { date } => text.push_str(&format!(" until {}", date.format("%-d %b %Y"))),
            // `remaining` counts what is still to come; the phrase counts
            // occurrences including the one currently due.
            End::After { remaining } => text.push_str(&format!(" x{}", remaining + 1)),
        }

        text
    }

    /// The unit or weekday list, without the interval or the end clause.
    fn body(&self, plural: bool) -> String {
        if self.unit == Unit::Week && !self.weekdays.is_empty() {
            if self.weekdays == Self::every_weekday().weekdays {
                return "weekday".to_string();
            }
            return join_days(&self.weekdays);
        }

        let unit = match self.unit {
            Unit::Day => "day",
            Unit::Week => "week",
            Unit::Month => "month",
            Unit::Year => "year",
        };
        if plural {
            format!("{unit}s")
        } else {
            unit.to_string()
        }
    }

    /// Advance the rule past one completion.
    ///
    /// `due` is the occurrence being completed and `completed_on` the day the
    /// user actually ticked it. Returns the next due date together with the
    /// rule as it now stands — [`End::After`] counts down, so the rule that
    /// comes back is not always the rule that went in. `None` means the
    /// recurrence has run out and the task is simply done.
    pub fn advance(&self, due: NaiveDate, completed_on: NaiveDate) -> Option<(NaiveDate, Self)> {
        let anchor = if self.from_completion {
            // Never step backwards: completing early must not resurrect an
            // occurrence that has already passed.
            completed_on.max(due)
        } else {
            due
        };

        let next = self.next_after(anchor)?;

        match self.end {
            End::Never => Some((next, self.clone())),
            End::OnDate { date } if next > date => None,
            End::OnDate { .. } => Some((next, self.clone())),
            End::After { remaining: 0 } => None,
            End::After { remaining } => {
                let mut rule = self.clone();
                rule.end = End::After {
                    remaining: remaining - 1,
                };
                Some((next, rule))
            }
        }
    }
}

/// `monday`, `mon and fri`, `mon, tue and fri` — the shapes the parser's
/// weekday list already accepts.
fn join_days(days: &[Weekday]) -> String {
    let names: Vec<&str> = days.iter().map(weekday_name).collect();
    match names.split_last() {
        None => String::new(),
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// Chrono's `Display` gives "Mon"; the full name reads better in a phrase and
/// parses just the same.
fn weekday_name(day: &Weekday) -> &'static str {
    match day {
        Weekday::Mon => "monday",
        Weekday::Tue => "tuesday",
        Weekday::Wed => "wednesday",
        Weekday::Thu => "thursday",
        Weekday::Fri => "friday",
        Weekday::Sat => "saturday",
        Weekday::Sun => "sunday",
    }
}

/// Add whole months, clamping to the end of the target month.
///
/// The 31st of January plus one month is the 28th of February, not the 3rd of
/// March. `chrono`'s `Months` already does this; the wrapper exists so the
/// intent is stated once and the year case can reuse it.
fn add_months_clamped(date: NaiveDate, months: u32) -> Option<NaiveDate> {
    date.checked_add_months(Months::new(months))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_daily_rule_steps_by_its_interval() {
        let rule = Recurrence::every(3, Unit::Day);
        assert_eq!(rule.next_after(date(2026, 7, 30)), Some(date(2026, 8, 2)));
    }

    #[test]
    fn an_interval_of_zero_is_treated_as_one_rather_than_looping_forever() {
        let rule = Recurrence {
            interval: 0,
            ..Recurrence::every(1, Unit::Day)
        };
        let anchor = date(2026, 7, 30);
        let next = rule.next_after(anchor).unwrap();
        assert!(next > anchor, "a rule must always move forward");
        assert_eq!(next, date(2026, 7, 31));
    }

    #[test]
    fn a_weekly_rule_without_weekdays_keeps_the_anchors_own_day() {
        let rule = Recurrence::every(2, Unit::Week);
        // 30 July 2026 is a Thursday.
        let next = rule.next_after(date(2026, 7, 30)).unwrap();
        assert_eq!(next, date(2026, 8, 13));
        assert_eq!(next.weekday(), Weekday::Thu);
    }

    #[test]
    fn a_multi_day_rule_lands_later_in_the_same_week_before_wrapping() {
        let rule = Recurrence::weekly_on(1, [Weekday::Mon, Weekday::Fri]);
        // Monday 27 July 2026 -> Friday the 31st, not next Monday.
        assert_eq!(rule.next_after(date(2026, 7, 27)), Some(date(2026, 7, 31)));
        // Friday 31 July -> Monday 3 August.
        assert_eq!(rule.next_after(date(2026, 7, 31)), Some(date(2026, 8, 3)));
    }

    #[test]
    fn a_multi_day_rule_does_not_creep_forward_when_it_wraps() {
        // Every other Monday and Friday, starting Monday 27 July 2026.
        let rule = Recurrence::weekly_on(2, [Weekday::Mon, Weekday::Fri]);
        let first = rule.next_after(date(2026, 7, 27)).unwrap();
        assert_eq!(first, date(2026, 7, 31)); // same week, Friday
        let second = rule.next_after(first).unwrap();
        // Two weeks on from the week beginning the 27th, not from the 31st.
        assert_eq!(second, date(2026, 8, 10));
        assert_eq!(second.weekday(), Weekday::Mon);
    }

    #[test]
    fn every_weekday_skips_the_weekend() {
        let rule = Recurrence::every_weekday();
        // Friday 31 July 2026 -> Monday 3 August.
        assert_eq!(rule.next_after(date(2026, 7, 31)), Some(date(2026, 8, 3)));
        // Monday -> Tuesday.
        assert_eq!(rule.next_after(date(2026, 8, 3)), Some(date(2026, 8, 4)));
    }

    #[test]
    fn weekdays_are_normalised_however_the_parser_supplies_them() {
        let rule = Recurrence::weekly_on(1, [Weekday::Fri, Weekday::Mon, Weekday::Fri]);
        assert_eq!(rule.weekdays, vec![Weekday::Mon, Weekday::Fri]);
    }

    #[test]
    fn a_monthly_rule_clamps_to_the_end_of_a_short_month() {
        let rule = Recurrence::every(1, Unit::Month);
        assert_eq!(rule.next_after(date(2026, 1, 31)), Some(date(2026, 2, 28)));
        assert_eq!(rule.next_after(date(2026, 3, 31)), Some(date(2026, 4, 30)));
    }

    #[test]
    fn clamping_does_not_permanently_lose_the_original_day() {
        // Stepping 31 January by two months reaches 31 March: the clamp
        // applies to the target month, it does not rewrite the anchor.
        let rule = Recurrence::every(2, Unit::Month);
        assert_eq!(rule.next_after(date(2026, 1, 31)), Some(date(2026, 3, 31)));
    }

    #[test]
    fn a_yearly_rule_clamps_the_twenty_ninth_of_february() {
        let rule = Recurrence::every(1, Unit::Year);
        assert_eq!(rule.next_after(date(2028, 2, 29)), Some(date(2029, 2, 28)));
    }

    #[test]
    fn every_is_anchored_to_the_due_date_however_late_it_is_completed() {
        let rule = Recurrence::every(1, Unit::Week);
        let (next, _) = rule
            .advance(date(2026, 7, 6), date(2026, 7, 27))
            .expect("no end condition");
        assert_eq!(next, date(2026, 7, 13));
    }

    #[test]
    fn every_bang_is_anchored_to_the_completion_date() {
        let rule = Recurrence {
            from_completion: true,
            ..Recurrence::every(10, Unit::Day)
        };
        let (next, _) = rule
            .advance(date(2026, 7, 6), date(2026, 7, 27))
            .expect("no end condition");
        assert_eq!(next, date(2026, 8, 6));
    }

    #[test]
    fn completing_early_never_steps_backwards() {
        let rule = Recurrence {
            from_completion: true,
            ..Recurrence::every(10, Unit::Day)
        };
        // Ticked a week before it was due: the next one is still measured
        // from the due date, otherwise it would land in the past.
        let (next, _) = rule
            .advance(date(2026, 7, 30), date(2026, 7, 23))
            .expect("no end condition");
        assert_eq!(next, date(2026, 8, 9));
    }

    #[test]
    fn an_end_date_stops_the_rule_rather_than_overshooting_it() {
        let rule = Recurrence {
            end: End::OnDate {
                date: date(2026, 8, 5),
            },
            ..Recurrence::every(1, Unit::Week)
        };
        // The next weekly occurrence after 30 July is 6 August, past the end
        // date, so the recurrence is over rather than clamped onto the 5th.
        assert_eq!(rule.advance(date(2026, 7, 30), date(2026, 7, 30)), None);
    }

    #[test]
    fn an_end_date_falling_exactly_on_an_occurrence_still_produces_it() {
        let rule = Recurrence {
            end: End::OnDate {
                date: date(2026, 8, 6),
            },
            ..Recurrence::every(1, Unit::Week)
        };
        let (next, _) = rule.advance(date(2026, 7, 30), date(2026, 7, 30)).unwrap();
        assert_eq!(next, date(2026, 8, 6));
    }

    #[test]
    fn a_spent_count_produces_nothing() {
        let rule = Recurrence {
            end: End::After { remaining: 0 },
            ..Recurrence::every(1, Unit::Day)
        };
        assert_eq!(rule.advance(date(2026, 7, 30), date(2026, 7, 30)), None);
    }

    #[test]
    fn a_count_limited_rule_produces_exactly_that_many_more_occurrences() {
        let mut rule = Recurrence {
            end: End::After { remaining: 3 },
            ..Recurrence::every(1, Unit::Day)
        };
        let mut due = date(2026, 7, 30);
        let mut produced = Vec::new();
        while let Some((next, advanced)) = rule.advance(due, due) {
            produced.push(next);
            due = next;
            rule = advanced;
        }
        assert_eq!(
            produced,
            vec![date(2026, 7, 31), date(2026, 8, 1), date(2026, 8, 2)]
        );
    }

    #[test]
    fn a_rule_reads_as_the_phrase_that_would_have_produced_it() {
        assert_eq!(Recurrence::every(1, Unit::Day).describe(), "every day");
        assert_eq!(
            Recurrence::every(2, Unit::Day).describe(),
            "every other day"
        );
        assert_eq!(Recurrence::every(3, Unit::Day).describe(), "every 3 days");
        assert_eq!(Recurrence::every(1, Unit::Month).describe(), "every month");
        assert_eq!(Recurrence::every_weekday().describe(), "every weekday");
        assert_eq!(
            Recurrence::weekly_on(1, [Weekday::Mon]).describe(),
            "every monday"
        );
        assert_eq!(
            Recurrence::weekly_on(2, [Weekday::Mon, Weekday::Fri]).describe(),
            "every other monday and friday"
        );
        assert_eq!(
            Recurrence::weekly_on(1, [Weekday::Mon, Weekday::Wed, Weekday::Fri]).describe(),
            "every monday, wednesday and friday"
        );
    }

    #[test]
    fn completion_anchoring_and_the_end_clause_survive_being_written_out() {
        let rule = Recurrence {
            from_completion: true,
            ..Recurrence::every(10, Unit::Day)
        };
        assert_eq!(rule.describe(), "every! 10 days");

        let rule = Recurrence {
            end: End::OnDate {
                date: date(2026, 9, 1),
            },
            ..Recurrence::every(1, Unit::Week)
        };
        assert_eq!(rule.describe(), "every week until 1 Sep 2026");

        // Two occurrences left to come is three in the phrase, counting the
        // one currently due.
        let rule = Recurrence {
            end: End::After { remaining: 2 },
            ..Recurrence::every(1, Unit::Day)
        };
        assert_eq!(rule.describe(), "every day x3");
    }

    /// The property the editor depends on: showing a rule and re-reading it
    /// changes nothing. Without this, prefilling the box with the current rule
    /// would silently rewrite it the moment the user pressed Enter.
    #[test]
    fn every_rule_survives_a_round_trip_through_its_own_description() {
        let rules = [
            Recurrence::every(1, Unit::Day),
            Recurrence::every(2, Unit::Week),
            Recurrence::every(3, Unit::Month),
            Recurrence::every(5, Unit::Year),
            Recurrence::every_weekday(),
            Recurrence::weekly_on(1, [Weekday::Sat, Weekday::Sun]),
            Recurrence::weekly_on(2, [Weekday::Mon, Weekday::Wed, Weekday::Fri]),
            Recurrence {
                from_completion: true,
                ..Recurrence::every(10, Unit::Day)
            },
            Recurrence {
                end: End::OnDate {
                    date: date(2027, 3, 4),
                },
                ..Recurrence::every(1, Unit::Week)
            },
            Recurrence {
                end: End::After { remaining: 4 },
                ..Recurrence::every_weekday()
            },
            Recurrence {
                from_completion: true,
                end: End::After { remaining: 1 },
                ..Recurrence::weekly_on(2, [Weekday::Tue])
            },
        ];

        for rule in rules {
            let phrase = rule.describe();
            let parsed = crate::model::parse::parse_recurrence(&phrase, date(2026, 7, 30));
            assert_eq!(
                parsed.as_ref(),
                Some(&rule),
                "‘{phrase}’ did not round-trip"
            );
        }
    }

    /// A spent rule is never shown — the task is simply done — but describing
    /// one must not claim it repeats zero more times.
    #[test]
    fn a_spent_count_still_describes_its_last_occurrence() {
        let rule = Recurrence {
            end: End::After { remaining: 0 },
            ..Recurrence::every(1, Unit::Day)
        };
        assert_eq!(rule.describe(), "every day x1");
    }

    #[test]
    fn a_rule_with_no_end_serialises_without_the_noise() {
        let json = serde_json::to_string(&Recurrence::every(1, Unit::Day)).unwrap();
        assert_eq!(json, r#"{"interval":1,"unit":"day"}"#);
    }

    #[test]
    fn a_full_rule_round_trips_through_json() {
        let rule = Recurrence {
            from_completion: true,
            end: End::After { remaining: 5 },
            ..Recurrence::weekly_on(2, [Weekday::Mon, Weekday::Thu])
        };
        let json = serde_json::to_string(&rule).unwrap();
        assert_eq!(serde_json::from_str::<Recurrence>(&json).unwrap(), rule);
    }
}

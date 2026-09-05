//! Where a record sits in a hand-sorted list.
//!
//! A key that sorts lexicographically and can always be generated *between*
//! two neighbours, so moving one task rewrites one record. That is the whole
//! reason this exists.
//!
//! The obvious representation — a position per record, renumbered on every
//! move — cannot survive sync, and neither of its variants helps. Renumbering
//! while bumping `updated_at` sends the entire list on every drag and makes
//! "recently changed" meaningless. Renumbering *without* bumping it, which is
//! what this used to do, means a reorder reaches nobody at all; and when two
//! machines reorder the same list, merging their records field by field
//! produces positions that were never a valid ordering on either of them —
//! duplicates, gaps, a list neither user arranged.
//!
//! With a key like this there is nothing to renumber. A move changes exactly
//! one record, whose `updated_at` then honestly reflects that somebody moved
//! it, and two machines moving different tasks merge cleanly because their
//! edits do not overlap.
//!
//! Two machines dropping a task in the *same* gap can generate the same key.
//! That is a tie, not a collision: the list sorts by key and then by id, so
//! both tasks are present and in a stable order everywhere. Nothing is lost,
//! which is the only property that matters.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// The digits a key is written in, in sort order.
///
/// Base 36 rather than base 62 so that keys are case-insensitively unambiguous
/// when read in a JSON file by a human, which is the only thing that ever reads
/// one directly.
const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// A position in a hand-sorted list.
///
/// Ordered by the bytes of the key, which is why every digit is ASCII: this is
/// the same comparison Postgres will make on a `text` column, so the client and
/// the server agree about order without either sorting the other's way.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Order(String);

impl Order {
    /// The key for the first record in an empty list.
    pub fn start() -> Self {
        Self::between(None, None)
    }

    /// A key that sorts after `before` and before `after`.
    ///
    /// `None` on either side means "the end of the list in that direction", so
    /// appending is `between(Some(last), None)` and prepending is
    /// `between(None, Some(first))`.
    pub fn between(before: Option<&Order>, after: Option<&Order>) -> Self {
        let before = before.map(|order| order.0.as_str()).unwrap_or("");
        let after = after.map(|order| order.0.as_str());

        // Neighbours arriving out of order means a caller sorted the list one
        // way and read it another. Appending after the larger of the two keeps
        // the list usable rather than panicking in front of the user, and the
        // assertion catches it in the suite.
        if let Some(after) = after {
            debug_assert!(
                before < after,
                "neighbours out of order: {before:?}, {after:?}"
            );
            if before >= after {
                return Self(midpoint(after.max(before), None));
            }
        }

        Self(midpoint(before, after))
    }

    /// The key as it is stored and sorted.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key for a position written by schema v1, which numbered lists
    /// `0, 1, 2 …`.
    ///
    /// A fixed-width encoding, so that comparing the keys gives the same
    /// answer as comparing the numbers did and a file upgrades without anyone
    /// having to reorder anything. The digits deliberately start one above the
    /// lowest, both so that no migrated key ends in it and so that every one of
    /// them sorts below a freshly minted key — a v1 list stays in its order and
    /// new tasks land after it.
    fn from_legacy_position(position: i64) -> Self {
        const WIDTH: u32 = 6;
        let base = DIGITS.len() as u64 - 1;
        let ceiling = base.pow(WIDTH) - 1;

        let mut value = (position.max(0) as u64).min(ceiling);
        let mut digits = [DIGITS[1]; WIDTH as usize];
        for slot in digits.iter_mut().rev() {
            *slot = DIGITS[1 + (value % base) as usize];
            value /= base;
        }
        Self(String::from_utf8(digits.to_vec()).expect("the alphabet is ASCII"))
    }
}

/// Accepts what v2 writes and what v1 wrote.
///
/// A v1 file holds `"order": 3`, and refusing it would not be a failed field —
/// the whole document would fail to parse, and `Store::open_at` would set a
/// perfectly good task list aside as corrupt. Reading the number and placing it
/// is the difference between an upgrade and an apparent data loss.
impl<'de> Deserialize<'de> for Order {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Either;

        impl Visitor<'_> for Either {
            type Value = Order;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an ordering key, or a v1 position")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Order, E> {
                Ok(Order(value.to_string()))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Order, E> {
                Ok(Order::from_legacy_position(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Order, E> {
                Ok(Order::from_legacy_position(value as i64))
            }
        }

        deserializer.deserialize_any(Either)
    }
}

impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Default for Order {
    fn default() -> Self {
        Self::start()
    }
}

/// A key strictly between `a` and `b`, where `a < b` and either may be open.
///
/// The invariant that makes this terminate is that no key ends in the lowest
/// digit. Without it there would be no room left below a key like `"10"` and
/// prepending would have no answer.
fn midpoint(a: &str, b: Option<&str>) -> String {
    // An exhausted upper bound means there is genuinely no room below it, which
    // nothing this generates can produce — that is exactly what the
    // no-trailing-lowest-digit invariant buys. It can still arrive from a
    // hand-edited file holding `"order": "0"`, so treat it as open rather than
    // indexing off the end of it.
    let b = b.filter(|bound| !bound.is_empty());

    if let Some(b) = b {
        // A shared prefix contributes nothing to the comparison, so take it off
        // and solve the smaller problem. `a` is padded with the lowest digit
        // because a shorter key is a key with implicit zeroes after it.
        let mut shared = 0;
        while shared < b.len() {
            let from_a = a.as_bytes().get(shared).copied().unwrap_or(DIGITS[0]);
            if from_a != b.as_bytes()[shared] {
                break;
            }
            shared += 1;
        }
        if shared > 0 {
            let rest_of_a = a.get(shared..).unwrap_or("");
            return format!(
                "{}{}",
                &b[..shared],
                midpoint(rest_of_a, Some(&b[shared..]))
            );
        }
    }

    let low = a.as_bytes().first().map_or(0, |digit| index_of(*digit));
    let high = match b {
        Some(b) => index_of(b.as_bytes()[0]),
        None => DIGITS.len(),
    };

    if high - low > 1 {
        // There is a digit spare between them, which is the whole answer.
        return (DIGITS[(low + high) / 2] as char).to_string();
    }

    // The digits are adjacent, so the key has to get longer.
    match b {
        // `b` has more to it, so borrowing its first digit already sorts below
        // it — and above `a`, whose first digit is lower.
        Some(b) if b.len() > 1 => b[..1].to_string(),
        // Otherwise keep `a`'s first digit and find room in what follows it.
        _ => {
            let rest = a.get(1..).unwrap_or("");
            format!("{}{}", DIGITS[low] as char, midpoint(rest, None))
        }
    }
}

fn index_of(digit: u8) -> usize {
    DIGITS
        .iter()
        .position(|candidate| *candidate == digit)
        // A key that is not in the alphabet can only come from a hand-edited
        // file. Treating it as the lowest digit sorts it to the top of its
        // list, which is visible and harmless, rather than panicking.
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Order {
        Order(text.to_string())
    }

    #[test]
    fn a_first_key_sits_in_the_middle_of_the_alphabet() {
        // Room to insert roughly as many times before it as after it, which is
        // the point of not starting at one end.
        let start = Order::start();
        assert!(start.as_str() > "0");
        assert!(start.as_str() < "z");
    }

    #[test]
    fn a_key_between_two_others_sorts_between_them() {
        let first = Order::start();
        let last = Order::between(Some(&first), None);
        let middle = Order::between(Some(&first), Some(&last));

        assert!(first < middle);
        assert!(middle < last);
    }

    #[test]
    fn appending_repeatedly_keeps_going_up() {
        let mut keys = vec![Order::start()];
        for _ in 0..200 {
            let next = Order::between(keys.last(), None);
            assert!(*keys.last().unwrap() < next);
            keys.push(next);
        }

        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn prepending_repeatedly_keeps_going_down() {
        let mut keys = vec![Order::start()];
        for _ in 0..200 {
            let next = Order::between(None, keys.first());
            assert!(next < keys[0]);
            keys.insert(0, next);
        }

        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    /// The case that would break a scheme with a fixed number of digits:
    /// dropping into the same gap over and over.
    #[test]
    fn splitting_the_same_gap_two_hundred_times_still_fits() {
        let low = Order::start();
        let mut high = Order::between(Some(&low), None);

        for _ in 0..200 {
            let next = Order::between(Some(&low), Some(&high));
            assert!(low < next, "{low} < {next}");
            assert!(next < high, "{next} < {high}");
            high = next;
        }
    }

    #[test]
    fn no_key_ever_ends_in_the_lowest_digit() {
        // The invariant the algorithm depends on: a key ending in the lowest
        // digit would leave nowhere to insert below it.
        let mut key = Order::start();
        for _ in 0..200 {
            key = Order::between(None, Some(&key));
            assert!(!key.as_str().ends_with('0'), "{key}");
        }
    }

    #[test]
    fn a_key_round_trips_as_a_plain_json_string() {
        let order = key("i");
        let json = serde_json::to_string(&order).expect("serialise");
        assert_eq!(json, r#""i""#);
        assert_eq!(
            serde_json::from_str::<Order>(&json).expect("deserialise"),
            order
        );
    }

    #[test]
    fn a_v1_position_still_reads_and_keeps_its_place() {
        // What a schema v1 file holds. Refusing it would fail the whole
        // document, not one field, and the store would quarantine a task list
        // that was never corrupt.
        let positions: Vec<Order> = (0..50)
            .map(|position| serde_json::from_str(&position.to_string()).expect("a v1 position"))
            .collect();

        let mut sorted = positions.clone();
        sorted.sort();
        assert_eq!(positions, sorted, "v1 order has to survive the upgrade");
    }

    #[test]
    fn a_task_added_after_an_upgrade_lands_at_the_end() {
        let last_of_the_old: Order = serde_json::from_str("41").expect("a v1 position");
        let brand_new = Order::between(Some(&last_of_the_old), None);
        assert!(
            last_of_the_old < brand_new,
            "{last_of_the_old} < {brand_new}"
        );
    }

    #[test]
    fn a_v1_position_never_ends_in_the_lowest_digit() {
        // Otherwise there would be nowhere to drop a task above the first one
        // in an upgraded list.
        for position in 0..2000 {
            let order = Order::from_legacy_position(position);
            assert!(!order.as_str().ends_with('0'), "{order}");
        }
    }

    #[test]
    fn a_hand_edited_key_with_no_room_below_it_does_not_panic() {
        // Nothing here generates a key ending in the lowest digit, so this can
        // only come from someone editing the JSON. It must not take the app
        // down on the next drag.
        let hand_edited = key("0");
        let below = Order::between(None, Some(&hand_edited));
        assert!(!below.as_str().is_empty());
    }

    #[test]
    fn neighbours_in_the_wrong_order_do_not_panic_in_release() {
        // debug_assert fires in the suite, so this checks the release path by
        // going through midpoint directly.
        let recovered = midpoint("z", None);
        assert!(recovered.as_str() > "z");
    }
}

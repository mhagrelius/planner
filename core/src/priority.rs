//! Task priority.
//!
//! Four levels, named and ordered as Todoist names them, because that is what
//! the `p1`–`p4` quick-add syntax means to anyone who has used a task manager
//! before. P1 is the most urgent and P4 means "unset" rather than "least
//! urgent" — it is the default, and it renders without a colour at all.

use serde::{Deserialize, Serialize};

/// How urgent a task is. `P4` is the absence of a priority, not a low one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Priority {
    P1,
    P2,
    P3,
    #[default]
    P4,
}

impl Priority {
    /// Every level, most urgent first. Iteration order for menus and sorting.
    pub const ALL: [Priority; 4] = [Priority::P1, Priority::P2, Priority::P3, Priority::P4];

    /// Parse a `p1`–`p4` token, case-insensitively. Used by quick-add and by
    /// the filter query language, which is why it lives here and not in either.
    pub fn from_token(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "p1" => Some(Self::P1),
            "p2" => Some(Self::P2),
            "p3" => Some(Self::P3),
            "p4" => Some(Self::P4),
            _ => None,
        }
    }

    /// The token that round-trips back through [`from_token`](Self::from_token).
    pub fn token(self) -> &'static str {
        match self {
            Self::P1 => "p1",
            Self::P2 => "p2",
            Self::P3 => "p3",
            Self::P4 => "p4",
        }
    }

    /// The user-facing name.
    pub fn label(self) -> &'static str {
        match self {
            Self::P1 => "Urgent",
            Self::P2 => "High",
            Self::P3 => "Medium",
            Self::P4 => "None",
        }
    }

    /// The CSS class the flag icon takes. `P4` has none: an unset priority
    /// should look unset, not like a fourth colour competing for attention.
    pub fn css_class(self) -> Option<&'static str> {
        match self {
            Self::P1 => Some("priority-1"),
            Self::P2 => Some("priority-2"),
            Self::P3 => Some("priority-3"),
            Self::P4 => None,
        }
    }

    /// Whether this priority is worth drawing a flag for at all.
    pub fn is_set(self) -> bool {
        self != Self::P4
    }

    /// Sort key, most urgent first. `Ord` is deliberately not derived: the
    /// declaration order would make P1 sort *before* P2 numerically but the
    /// intent at every call site is "most urgent first", and spelling it out
    /// stops that from being an accident of field order.
    pub fn rank(self) -> u8 {
        match self {
            Self::P1 => 0,
            Self::P2 => 1,
            Self::P3 => 2,
            Self::P4 => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_priority_is_p4() {
        assert_eq!(Priority::default(), Priority::P4);
        assert!(!Priority::default().is_set());
        assert!(Priority::default().css_class().is_none());
    }

    #[test]
    fn tokens_round_trip_and_ignore_case() {
        for priority in Priority::ALL {
            assert_eq!(Priority::from_token(priority.token()), Some(priority));
        }
        assert_eq!(Priority::from_token("P2"), Some(Priority::P2));
        assert_eq!(Priority::from_token("p5"), None);
        assert_eq!(Priority::from_token("priority"), None);
    }

    #[test]
    fn sorting_by_rank_puts_the_most_urgent_first() {
        let mut levels = vec![Priority::P3, Priority::P1, Priority::P4, Priority::P2];
        levels.sort_by_key(|p| p.rank());
        assert_eq!(levels, Priority::ALL);
    }
}

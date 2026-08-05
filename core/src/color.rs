//! The colour palette shared by projects, sections and labels.
//!
//! A fixed named set rather than a free colour picker. Three reasons: names
//! survive a light/dark switch where a stored hex value does not, a fixed set
//! keeps a sidebar of twenty projects legible instead of muddy, and it means
//! there is no colour-picker dialog to build or to make accessible.

use serde::{Deserialize, Serialize};

/// A colour a project, section or label can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Color {
    #[default]
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
    Brown,
    Slate,
}

impl Color {
    /// Every colour, in the order they appear in the picker.
    pub const ALL: [Color; 10] = [
        Color::Blue,
        Color::Teal,
        Color::Green,
        Color::Yellow,
        Color::Orange,
        Color::Red,
        Color::Pink,
        Color::Purple,
        Color::Brown,
        Color::Slate,
    ];

    /// The stable identifier, matching the serialised form and the CSS class
    /// suffix. Kept in one place so the stylesheet and the JSON cannot drift.
    pub fn id(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Teal => "teal",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Orange => "orange",
            Self::Red => "red",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Brown => "brown",
            Self::Slate => "slate",
        }
    }

    /// Look a colour up by its [`id`](Self::id).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    /// The user-facing name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Blue => "Blue",
            Self::Teal => "Teal",
            Self::Green => "Green",
            Self::Yellow => "Yellow",
            Self::Orange => "Orange",
            Self::Red => "Red",
            Self::Pink => "Pink",
            Self::Purple => "Purple",
            Self::Brown => "Brown",
            Self::Slate => "Slate",
        }
    }

    /// The CSS class carrying this colour's custom properties.
    pub fn css_class(self) -> String {
        format!("accent-{}", self.id())
    }
}

/// The colour a new project or label should take, given those that exist.
///
/// Picks the least-used colour, so a fresh one is chosen while any remain
/// unused and the spread stays even afterwards. Colour is only worth having if
/// it distinguishes things; ten identical blue projects tell you nothing.
/// Ties break toward [`Color::ALL`] order, so the sequence is deterministic.
pub fn least_used(existing: &[Color]) -> Color {
    Color::ALL
        .into_iter()
        .min_by_key(|color| existing.iter().filter(|c| *c == color).count())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for color in Color::ALL {
            assert_eq!(Color::from_id(color.id()), Some(color));
        }
        assert_eq!(Color::from_id("chartreuse"), None);
    }

    #[test]
    fn the_serialised_form_matches_the_id() {
        for color in Color::ALL {
            let json = serde_json::to_string(&color).unwrap();
            assert_eq!(json, format!("\"{}\"", color.id()));
        }
    }

    #[test]
    fn fresh_colours_are_used_before_any_is_repeated() {
        let mut used = Vec::new();
        for _ in 0..Color::ALL.len() {
            used.push(least_used(&used));
        }
        assert_eq!(used, Color::ALL.to_vec());
    }

    #[test]
    fn once_every_colour_is_taken_the_spread_stays_even() {
        let mut used = Color::ALL.to_vec();
        used.push(Color::Blue);
        // Blue is now used twice, so it must not be the next choice.
        assert_ne!(least_used(&used), Color::Blue);
        assert_eq!(least_used(&used), Color::Teal);
    }
}

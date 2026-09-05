//! Where to sync to, if anywhere.
//!
//! **Sync is off until a URL and a token are written here on purpose.** Unlike
//! the file the app already knows how to find, there is nothing sensible to
//! guess at: an address invented by the app is either wrong or somebody else's
//! server. A missing or unreadable config is "no sync", not an error, because
//! the app works perfectly well without one and refusing to start over it would
//! be the wrong trade in every direction.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Everything that is not in the document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// `http://host:port` of a `planner-server`. Absent means no sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_url: Option<String>,
    /// The bearer token that server was started with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_token: Option<String>,

    /// An Anthropic API key, enabling the semantic half of the duplicate
    /// check. Absent means quick-add still warns about near-identical titles,
    /// using the local word comparison only.
    ///
    /// In a file rather than the keyring for the same reason `sync_token` is:
    /// there is one place a person configures this app, and a second mechanism
    /// for the second secret would be a second thing to explain and to break.
    /// The file is the app's own, under `$XDG_CONFIG_HOME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_api_key: Option<String>,
}

impl Config {
    /// `$XDG_CONFIG_HOME/planner/config.json`, falling back to
    /// `$HOME/.config`.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("planner").join("config.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    /// Never fails. A file that will not parse is the same as no file: sync
    /// stays off and the app opens.
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// The two halves of a sync target, or nothing.
    ///
    /// Both or neither: a URL with no token cannot authenticate and a token
    /// with no URL has nowhere to go, and silently doing half of it is how you
    /// get a client that looks configured and never syncs.
    pub fn sync_target(&self) -> Option<(&str, &str)> {
        match (self.sync_url.as_deref(), self.sync_token.as_deref()) {
            (Some(url), Some(token)) if !url.is_empty() && !token.is_empty() => Some((url, token)),
            _ => None,
        }
    }

    /// The API key, if one was written. An empty string is not a setting, for
    /// the same reason a half-filled sync target is not one.
    pub fn anthropic_key(&self) -> Option<&str> {
        self.anthropic_api_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_file_means_no_sync_rather_than_an_error() {
        let config = Config::load_from(Path::new("/nowhere/at/all/config.json"));
        assert_eq!(config.sync_target(), None);
    }

    #[test]
    fn a_file_that_will_not_parse_is_the_same_as_no_file() {
        let dir = tempfile::TempDir::new().expect("a temp dir");
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").expect("write");

        // The app opens. A planner that refuses to start because of a stray
        // brace in a config file is worse than one that does not sync.
        assert_eq!(Config::load_from(&path), Config::default());
    }

    #[test]
    fn half_a_target_is_no_target() {
        let url_only = Config {
            sync_url: Some("http://nas:8083".into()),
            sync_token: None,
            ..Default::default()
        };
        assert_eq!(url_only.sync_target(), None);

        let token_only = Config {
            sync_url: None,
            sync_token: Some("t".into()),
            ..Default::default()
        };
        assert_eq!(token_only.sync_target(), None);
    }

    #[test]
    fn both_halves_are_a_target() {
        let config = Config {
            sync_url: Some("http://nas:8083".into()),
            sync_token: Some("t".into()),
            ..Default::default()
        };
        assert_eq!(config.sync_target(), Some(("http://nas:8083", "t")));
    }

    #[test]
    fn an_empty_string_is_not_a_setting() {
        // A half-filled config file is a common way to end up here.
        let config = Config {
            sync_url: Some(String::new()),
            sync_token: Some("t".into()),
            ..Default::default()
        };
        assert_eq!(config.sync_target(), None);
    }

    #[test]
    fn no_key_means_the_local_duplicate_check_only() {
        assert_eq!(Config::default().anthropic_key(), None);
        let blank = Config {
            anthropic_api_key: Some("   ".into()),
            ..Default::default()
        };
        assert_eq!(blank.anthropic_key(), None);
    }

    #[test]
    fn a_key_that_is_there_is_returned() {
        let config = Config {
            anthropic_api_key: Some("sk-ant-xyz".into()),
            ..Default::default()
        };
        assert_eq!(config.anthropic_key(), Some("sk-ant-xyz"));
    }

    #[test]
    fn a_config_written_for_sync_alone_still_loads() {
        // The field arrived after people already had config files. An older
        // one must not become unreadable, which would silently turn sync off.
        let dir = tempfile::TempDir::new().expect("a temp dir");
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"sync_url":"http://nas:8083","sync_token":"t"}"#).expect("write");

        let config = Config::load_from(&path);
        assert_eq!(config.sync_target(), Some(("http://nas:8083", "t")));
        assert_eq!(config.anthropic_key(), None);
    }
}

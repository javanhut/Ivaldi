//! Configuration system for Ivaldi VCS.
//!
//! Two levels of configuration:
//! - User: `~/.ivaldi/config` (global settings)
//! - Repository: `.ivaldi/config` (per-repo, overrides user)
//!
//! Config uses a simple `section.key = value` format stored as plain text.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A documented configuration key: (key, what it does, example value).
pub const KNOWN_KEYS: &[(&str, &str, &str)] = &[
    (
        "user.name",
        "Your name, recorded as the author of every seal",
        "\"Ada Lovelace\"",
    ),
    (
        "user.email",
        "Your email, recorded alongside the author name",
        "ada@example.com",
    ),
    ("color.ui", "Colored CLI output (true/false)", "true"),
    (
        "core.autoshelf",
        "Auto-shelve uncommitted changes on timeline switch (true/false)",
        "true",
    ),
    (
        "portal.default",
        "Default remote for upload/sync when several portals are configured",
        "owner/repo",
    ),
];

/// One line per known key, used in `config --help` and error hints.
pub fn known_keys_help() -> String {
    KNOWN_KEYS
        .iter()
        .map(|(key, desc, example)| format!("  {:<16} {} (e.g. {})", key, desc, example))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Loose email shape check: `local@domain.tld`.
pub fn is_email_like(s: &str) -> bool {
    let (local, rest) = match s.split_once('@') {
        Some(p) => p,
        None => return false,
    };
    if local.is_empty() {
        return false;
    }
    rest.contains('.') && !rest.starts_with('.') && !rest.ends_with('.')
}

/// Validate a `--set` request. `Err` blocks the write (malformed key or
/// value); `Ok(Some(warning))` allows it with a caveat (unknown key).
pub fn validate_set(key: &str, value: &str) -> Result<Option<String>, String> {
    if !key.contains('.') {
        return Err(format!(
            "config keys use the form 'section.field' (got '{}').\nKnown keys:\n{}",
            key,
            known_keys_help()
        ));
    }
    match key {
        "user.name" => {
            if value.trim().is_empty() {
                return Err("user.name cannot be empty".into());
            }
        }
        "user.email" => {
            if !is_email_like(value) {
                return Err(format!(
                    "'{}' doesn't look like an email address (expected name@domain.tld)",
                    value
                ));
            }
        }
        "color.ui" | "core.autoshelf" => {
            if value != "true" && value != "false" {
                return Err(format!(
                    "{} must be 'true' or 'false' (got '{}')",
                    key, value
                ));
            }
        }
        "portal.default" => {
            if crate::portal::parse_repo_spec(value).is_err() {
                return Err(format!(
                    "'{}' is not a valid repo spec (expected owner/repo or a full URL)",
                    value
                ));
            }
        }
        unknown => {
            return Ok(Some(format!(
                "'{}' is not a key ivaldi reads — saving it anyway.\nKnown keys:\n{}",
                unknown,
                known_keys_help()
            )));
        }
    }
    Ok(None)
}

/// Ivaldi configuration.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// All config values: "section.key" → "value".
    values: BTreeMap<String, String>,
}

impl Config {
    pub fn new() -> Self {
        let mut cfg = Self::default();
        // Sensible defaults
        cfg.set("color.ui", "true");
        cfg.set("core.autoshelf", "true");
        cfg
    }

    /// Get a config value by dotted key (e.g., "user.name").
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }

    /// Set a config value.
    pub fn set(&mut self, key: &str, value: &str) {
        self.values.insert(key.to_string(), value.to_string());
    }

    /// Remove a config value.
    pub fn remove(&mut self, key: &str) -> bool {
        self.values.remove(key).is_some()
    }

    /// List all config values as (key, value) pairs, sorted.
    pub fn list(&self) -> Vec<(&str, &str)> {
        self.values
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    /// Get the author string "Name <email>" or None if not configured.
    pub fn author(&self) -> Option<String> {
        let name = self.get("user.name")?;
        let email = self.get("user.email")?;
        if name.is_empty() || email.is_empty() {
            return None;
        }
        Some(format!("{} <{}>", name, email))
    }

    /// Merge another config into this one (other values override).
    pub fn merge(&mut self, other: &Config) {
        for (key, value) in &other.values {
            if !value.is_empty() {
                self.values.insert(key.clone(), value.clone());
            }
        }
    }

    /// Serialize to string format.
    pub fn to_string_repr(&self) -> String {
        let mut lines = Vec::new();
        let mut current_section = String::new();

        for (key, value) in &self.values {
            if let Some((section, field)) = key.split_once('.') {
                if section != current_section {
                    if !current_section.is_empty() {
                        lines.push(String::new());
                    }
                    lines.push(format!("[{}]", section));
                    current_section = section.to_string();
                }
                lines.push(format!("    {} = {}", field, value));
            }
        }

        lines.join("\n") + "\n"
    }

    /// Parse from string format.
    pub fn from_str_repr(s: &str) -> Self {
        let mut cfg = Self {
            values: BTreeMap::new(),
        };
        let mut current_section = String::new();

        for line in s.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                current_section = line[1..line.len() - 1].to_string();
            } else if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                if !current_section.is_empty() {
                    cfg.set(&format!("{}.{}", current_section, key), value);
                }
            }
        }

        cfg
    }

    /// Save to a file.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ConfigError::Io)?;
        }
        crate::atomic_io::atomic_write(path, self.to_string_repr().as_bytes())
            .map_err(ConfigError::Io)
    }

    /// Load from a file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path).map_err(ConfigError::Io)?;
        Ok(Self::from_str_repr(&content))
    }
}

/// Load the merged configuration (user + repo, repo overrides user).
pub fn load_config(ivaldi_dir: &Path) -> Config {
    let mut cfg = Config::new();

    // Load user config
    if let Some(home) = dirs_path() {
        let user_config = home.join(".ivaldi").join("config");
        if let Ok(user_cfg) = Config::load(&user_config) {
            cfg.merge(&user_cfg);
        }
    }

    // Load repo config (overrides user)
    let repo_config = ivaldi_dir.join("config");
    if let Ok(repo_cfg) = Config::load(&repo_config) {
        cfg.merge(&repo_cfg);
    }

    cfg
}

/// Get home directory path.
fn dirs_path() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Path to the global config file (`~/.ivaldi/config`), if $HOME is set.
pub fn global_config_path() -> Option<PathBuf> {
    dirs_path().map(|home| home.join(".ivaldi").join("config"))
}

/// Load only the global config (ignoring any repo config).
pub fn load_global() -> Config {
    let mut cfg = Config::new();
    if let Some(path) = global_config_path()
        && let Ok(user_cfg) = Config::load(&path)
    {
        cfg.merge(&user_cfg);
    }
    cfg
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config key not found: {0}")]
    NotFound(String),
    #[error("invalid config key format: {0} (expected section.key)")]
    InvalidKey(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = Config::new();
        assert_eq!(cfg.get("color.ui"), Some("true"));
        assert_eq!(cfg.get("core.autoshelf"), Some("true"));
    }

    #[test]
    fn validate_set_accepts_good_values() {
        assert_eq!(validate_set("user.name", "Ada Lovelace"), Ok(None));
        assert_eq!(validate_set("user.email", "ada@example.com"), Ok(None));
        assert_eq!(validate_set("color.ui", "false"), Ok(None));
        assert_eq!(validate_set("core.autoshelf", "true"), Ok(None));
        assert_eq!(validate_set("portal.default", "owner/repo"), Ok(None));
    }

    #[test]
    fn validate_set_rejects_bad_values() {
        // Dotless keys would be silently dropped by the serializer — hard error.
        let err = validate_set("username", "Ada").unwrap_err();
        assert!(err.contains("section.field"));
        assert!(err.contains("user.name")); // hint lists known keys

        assert!(validate_set("user.name", "  ").is_err());
        assert!(validate_set("user.email", "not-an-email").is_err());
        assert!(validate_set("user.email", "a@b").is_err());
        assert!(validate_set("color.ui", "yes").is_err());
        assert!(validate_set("core.autoshelf", "1").is_err());
        assert!(validate_set("portal.default", "not a spec").is_err());
    }

    #[test]
    fn validate_set_warns_on_unknown_dotted_key() {
        let warning = validate_set("custom.thing", "anything").unwrap();
        assert!(warning.unwrap().contains("not a key ivaldi reads"));
    }

    #[test]
    fn known_keys_help_mentions_every_key() {
        let help = known_keys_help();
        for (key, _, _) in KNOWN_KEYS {
            assert!(help.contains(key), "help missing {}", key);
        }
    }

    #[test]
    fn email_shape_check() {
        assert!(is_email_like("ada@example.com"));
        assert!(is_email_like("a.b+c@sub.domain.io"));
        assert!(!is_email_like("ada"));
        assert!(!is_email_like("@example.com"));
        assert!(!is_email_like("ada@example"));
        assert!(!is_email_like("ada@.com"));
    }

    #[test]
    fn get_set() {
        let mut cfg = Config::new();
        cfg.set("user.name", "Alice");
        assert_eq!(cfg.get("user.name"), Some("Alice"));
        assert_eq!(cfg.get("user.email"), None);
    }

    #[test]
    fn remove() {
        let mut cfg = Config::new();
        cfg.set("user.name", "Alice");
        assert!(cfg.remove("user.name"));
        assert_eq!(cfg.get("user.name"), None);
        assert!(!cfg.remove("nonexistent"));
    }

    #[test]
    fn list_sorted() {
        let mut cfg = Config::new();
        cfg.set("user.name", "Alice");
        cfg.set("user.email", "alice@test.com");
        let items = cfg.list();
        // Should be sorted by key
        let keys: Vec<&str> = items.iter().map(|(k, _)| *k).collect();
        let mut sorted_keys = keys.clone();
        sorted_keys.sort();
        assert_eq!(keys, sorted_keys);
    }

    #[test]
    fn author() {
        let mut cfg = Config::new();
        assert!(cfg.author().is_none());

        cfg.set("user.name", "Alice");
        assert!(cfg.author().is_none()); // email missing

        cfg.set("user.email", "alice@test.com");
        assert_eq!(cfg.author(), Some("Alice <alice@test.com>".to_string()));
    }

    #[test]
    fn author_empty_name() {
        let mut cfg = Config::new();
        cfg.set("user.name", "");
        cfg.set("user.email", "a@b.com");
        assert!(cfg.author().is_none());
    }

    #[test]
    fn merge_overrides() {
        let mut base = Config::new();
        base.set("user.name", "Base");
        base.set("user.email", "base@test.com");

        let mut override_cfg = Config {
            values: BTreeMap::new(),
        };
        override_cfg.set("user.name", "Override");

        base.merge(&override_cfg);
        assert_eq!(base.get("user.name"), Some("Override"));
        assert_eq!(base.get("user.email"), Some("base@test.com")); // not overridden
    }

    #[test]
    fn serialize_roundtrip() {
        let mut cfg = Config::new();
        cfg.set("user.name", "Alice");
        cfg.set("user.email", "alice@test.com");
        cfg.set("color.ui", "true");

        let serialized = cfg.to_string_repr();
        let parsed = Config::from_str_repr(&serialized);

        assert_eq!(parsed.get("user.name"), Some("Alice"));
        assert_eq!(parsed.get("user.email"), Some("alice@test.com"));
        assert_eq!(parsed.get("color.ui"), Some("true"));
    }

    #[test]
    fn parse_with_comments() {
        let input = r#"
# This is a comment
[user]
    name = Bob
    email = bob@test.com

# Another comment
[color]
    ui = false
"#;
        let cfg = Config::from_str_repr(input);
        assert_eq!(cfg.get("user.name"), Some("Bob"));
        assert_eq!(cfg.get("user.email"), Some("bob@test.com"));
        assert_eq!(cfg.get("color.ui"), Some("false"));
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");

        let mut cfg = Config::new();
        cfg.set("user.name", "Alice");
        cfg.set("user.email", "alice@test.com");
        cfg.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.get("user.name"), Some("Alice"));
        assert_eq!(loaded.get("user.email"), Some("alice@test.com"));
    }

    #[test]
    fn load_nonexistent() {
        let result = Config::load(Path::new("/nonexistent/config"));
        assert!(result.is_err());
    }
}

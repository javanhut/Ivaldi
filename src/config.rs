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
        fs::write(path, self.to_string_repr()).map_err(ConfigError::Io)
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

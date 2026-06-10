//! Authentication management for Ivaldi VCS.
//!
//! Handles OAuth token storage and multi-source credential resolution
//! for GitHub and GitLab platforms.
//!
//! Token priority:
//! 1. Ivaldi OAuth token (`~/.config/ivaldi/auth.json`)
//! 2. Environment variable (`GITHUB_TOKEN` / `GITLAB_TOKEN`)
//! 3. `.netrc` file
//! 4. GitHub CLI / GitLab CLI config
//!
//! Token storage: `~/.config/ivaldi/auth.json` (0600 permissions)

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::portal::Platform;

/// OAuth constants.
pub const GITHUB_CLIENT_ID: &str = "178c6fc778ccc68e1d6a"; // GitHub CLI's public OAuth App
pub const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
pub const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const GITHUB_SCOPES: &str = "repo,read:user,user:email";

/// GitLab OAuth (Device Authorization Grant — RFC 8628).
///
/// Defaults target gitlab.com. For self-hosted GitLab, override the host with
/// `gitlab_host` config or `IVALDI_GITLAB_HOST`, and the client id with
/// `IVALDI_GITLAB_CLIENT_ID`. The default client id below is glab CLI's
/// public OAuth application; replace via env if you ship your own.
pub const GITLAB_HOST: &str = "https://gitlab.com";
pub const GITLAB_CLIENT_ID: &str =
    "41d48f9422ebd655ee3b6e85a7b8f7560bb0b50ad08522bb720e15f93a072039"; // glab CLI public OAuth app
pub const GITLAB_DEVICE_AUTH_PATH: &str = "/oauth/authorize_device";
pub const GITLAB_TOKEN_PATH: &str = "/oauth/token";
pub const GITLAB_SCOPES: &str = "read_user read_api write_repository";

/// A stored OAuth token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default, deserialize_with = "deserialize_created_at")]
    pub created_at: i64,
}

/// Flexibly deserialize created_at from either an i64 or a datetime string.
fn deserialize_created_at<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct CreatedAtVisitor;

    impl<'de> de::Visitor<'de> for CreatedAtVisitor {
        type Value = i64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an integer or a datetime string")
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> {
            Ok(v)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> {
            Ok(v as i64)
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<i64, E> {
            Ok(v as i64)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
            // Try to parse as ISO 8601 datetime — extract unix timestamp
            // Simple approach: if it contains 'T', treat as datetime
            if v.contains('T') || v.contains('-') {
                // Return current time as fallback — the token is valid regardless
                Ok(std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64)
            } else if let Ok(n) = v.parse::<i64>() {
                Ok(n)
            } else {
                Ok(0)
            }
        }
    }

    deserializer.deserialize_any(CreatedAtVisitor)
}

/// Multi-platform token storage format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<Token>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitlab: Option<Token>,
}

/// Manages token persistence.
pub struct TokenStore {
    config_path: PathBuf,
}

impl TokenStore {
    /// Create a token store at the default location (`~/.config/ivaldi/auth.json`).
    pub fn new() -> Result<Self, AuthError> {
        let home = std::env::var("HOME").map_err(|_| AuthError::NoHome)?;
        let config_dir = PathBuf::from(home).join(".config").join("ivaldi");
        fs::create_dir_all(&config_dir).map_err(AuthError::Io)?;

        Ok(Self {
            config_path: config_dir.join("auth.json"),
        })
    }

    /// Create a token store at a custom path (for testing).
    pub fn at_path(path: impl AsRef<Path>) -> Self {
        Self {
            config_path: path.as_ref().to_path_buf(),
        }
    }

    /// Load all tokens.
    pub fn load_all(&self) -> Result<TokenStorage, AuthError> {
        let content = match fs::read_to_string(&self.config_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TokenStorage::default());
            }
            Err(e) => return Err(AuthError::Io(e)),
        };

        serde_json::from_str(&content).map_err(AuthError::Json)
    }

    /// Load token for a specific platform.
    pub fn load_token(&self, platform: Platform) -> Result<Option<Token>, AuthError> {
        let storage = self.load_all()?;
        Ok(match platform {
            Platform::GitHub => storage.github,
            Platform::GitLab => storage.gitlab,
        })
    }

    /// Save token for a specific platform.
    pub fn save_token(&self, platform: Platform, token: Token) -> Result<(), AuthError> {
        let mut storage = self.load_all()?;
        match platform {
            Platform::GitHub => storage.github = Some(token),
            Platform::GitLab => storage.gitlab = Some(token),
        }

        let data = serde_json::to_string_pretty(&storage).map_err(AuthError::Json)?;

        // Write with restricted permissions
        fs::write(&self.config_path, &data).map_err(AuthError::Io)?;

        // Set file permissions to 0600 (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&self.config_path, perms).map_err(AuthError::Io)?;
        }

        Ok(())
    }

    /// Delete token for a specific platform.
    pub fn delete_token(&self, platform: Platform) -> Result<(), AuthError> {
        let mut storage = self.load_all()?;
        match platform {
            Platform::GitHub => storage.github = None,
            Platform::GitLab => storage.gitlab = None,
        }

        if storage.github.is_none() && storage.gitlab.is_none() {
            // No tokens left — delete the file
            match fs::remove_file(&self.config_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(AuthError::Io(e)),
            }
        } else {
            let data = serde_json::to_string_pretty(&storage).map_err(AuthError::Json)?;
            fs::write(&self.config_path, &data).map_err(AuthError::Io)?;
        }

        Ok(())
    }
}

/// Describes which authentication method is active.
#[derive(Debug, Clone)]
pub struct AuthMethod {
    pub name: String,
    pub description: String,
    pub token: String,
}

/// Resolve the active authentication token for a platform.
/// Checks multiple sources in priority order.
pub fn resolve_auth(platform: Platform) -> Option<AuthMethod> {
    // 1. Ivaldi OAuth token (highest priority)
    if let Ok(store) = TokenStore::new()
        && let Ok(Some(token)) = store.load_token(platform)
        && !token.access_token.is_empty()
    {
        let platform_name = match platform {
            Platform::GitHub => "github",
            Platform::GitLab => "gitlab",
        };
        return Some(AuthMethod {
            name: "ivaldi".to_string(),
            description: format!("Authenticated via 'ivaldi auth login --{}'", platform_name),
            token: token.access_token,
        });
    }

    // 2. Environment variable
    let env_var = match platform {
        Platform::GitHub => "GITHUB_TOKEN",
        Platform::GitLab => "GITLAB_TOKEN",
    };
    if let Ok(token) = std::env::var(env_var)
        && !token.is_empty()
    {
        return Some(AuthMethod {
            name: "env".to_string(),
            description: format!("Authenticated via {} environment variable", env_var),
            token,
        });
    }

    // 3. .netrc file
    if let Some(token) = read_netrc_token(match platform {
        Platform::GitHub => "github.com",
        Platform::GitLab => "gitlab.com",
    }) {
        return Some(AuthMethod {
            name: "netrc".to_string(),
            description: "Authenticated via .netrc file".to_string(),
            token,
        });
    }

    // 4. Platform CLI config
    match platform {
        Platform::GitHub => {
            if let Some(token) = read_gh_cli_token() {
                return Some(AuthMethod {
                    name: "gh-cli".to_string(),
                    description: "Authenticated via 'gh auth login' (GitHub CLI)".to_string(),
                    token,
                });
            }
        }
        Platform::GitLab => {
            if let Some(token) = read_glab_cli_token() {
                return Some(AuthMethod {
                    name: "glab-cli".to_string(),
                    description: "Authenticated via 'glab auth login' (GitLab CLI)".to_string(),
                    token,
                });
            }
        }
    }

    None
}

/// Check if authenticated for a platform.
pub fn is_authenticated(platform: Platform) -> bool {
    resolve_auth(platform).is_some()
}

// ---------------------------------------------------------------------------
// Credential source readers
// ---------------------------------------------------------------------------

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

fn read_netrc_token(machine: &str) -> Option<String> {
    let content = fs::read_to_string(home_dir()?.join(".netrc")).ok()?;
    let mut in_machine = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("machine ") && line.contains(machine) {
            in_machine = true;
        } else if in_machine && line.starts_with("password ") {
            return Some(line.strip_prefix("password ")?.to_string());
        } else if line.starts_with("machine ") {
            in_machine = false;
        }
    }
    None
}

fn read_gh_cli_token() -> Option<String> {
    let content = fs::read_to_string(home_dir()?.join(".config/gh/hosts.yml")).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains("oauth_token:") {
            return line.split(':').nth(1).map(|s| s.trim().to_string());
        }
        if line.contains("token:") && i > 0 && lines[i - 1].contains("github.com") {
            return line.split(':').nth(1).map(|s| s.trim().to_string());
        }
    }
    None
}

fn read_glab_cli_token() -> Option<String> {
    let content = fs::read_to_string(home_dir()?.join(".config/glab-cli/config.yml")).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.contains("token:") && i > 0 && lines[i - 1].contains("gitlab.com") {
            return line.split(':').nth(1).map(|s| s.trim().to_string());
        }
    }
    None
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HOME directory not set")]
    NoHome,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_store_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::at_path(&path);

        let token = Token {
            access_token: "ghp_test123".to_string(),
            token_type: "bearer".to_string(),
            scope: "repo".to_string(),
            created_at: 1700000000,
        };

        store.save_token(Platform::GitHub, token.clone()).unwrap();

        let loaded = store.load_token(Platform::GitHub).unwrap().unwrap();
        assert_eq!(loaded.access_token, "ghp_test123");
        assert_eq!(loaded.token_type, "bearer");

        // GitLab should still be None
        assert!(store.load_token(Platform::GitLab).unwrap().is_none());
    }

    #[test]
    fn token_store_multi_platform() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::at_path(&path);

        store
            .save_token(
                Platform::GitHub,
                Token {
                    access_token: "gh_token".to_string(),
                    token_type: "bearer".to_string(),
                    scope: "repo".to_string(),
                    created_at: 0,
                },
            )
            .unwrap();

        store
            .save_token(
                Platform::GitLab,
                Token {
                    access_token: "gl_token".to_string(),
                    token_type: "bearer".to_string(),
                    scope: "api".to_string(),
                    created_at: 0,
                },
            )
            .unwrap();

        assert_eq!(
            store
                .load_token(Platform::GitHub)
                .unwrap()
                .unwrap()
                .access_token,
            "gh_token"
        );
        assert_eq!(
            store
                .load_token(Platform::GitLab)
                .unwrap()
                .unwrap()
                .access_token,
            "gl_token"
        );
    }

    #[test]
    fn token_store_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::at_path(&path);

        store
            .save_token(
                Platform::GitHub,
                Token {
                    access_token: "token".to_string(),
                    token_type: "".to_string(),
                    scope: "".to_string(),
                    created_at: 0,
                },
            )
            .unwrap();

        store.delete_token(Platform::GitHub).unwrap();
        assert!(store.load_token(Platform::GitHub).unwrap().is_none());

        // File should be deleted when no tokens remain
        assert!(!path.exists());
    }

    #[test]
    fn token_store_delete_one_of_two() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::at_path(&path);

        let token = Token {
            access_token: "t".to_string(),
            token_type: "".to_string(),
            scope: "".to_string(),
            created_at: 0,
        };

        store.save_token(Platform::GitHub, token.clone()).unwrap();
        store.save_token(Platform::GitLab, token).unwrap();

        store.delete_token(Platform::GitHub).unwrap();

        // File should still exist with GitLab token
        assert!(path.exists());
        assert!(store.load_token(Platform::GitHub).unwrap().is_none());
        assert!(store.load_token(Platform::GitLab).unwrap().is_some());
    }

    #[test]
    fn token_store_empty_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::at_path(&path);

        assert!(store.load_token(Platform::GitHub).unwrap().is_none());
        assert!(store.load_token(Platform::GitLab).unwrap().is_none());
    }

    #[test]
    fn resolve_auth_from_env() {
        // This test depends on env vars, which we can't reliably set in parallel tests.
        // Just verify the function doesn't panic with no auth configured.
        // The actual resolution depends on the environment.
        let _ = resolve_auth(Platform::GitHub);
        let _ = resolve_auth(Platform::GitLab);
    }

    #[test]
    fn token_serialization() {
        let token = Token {
            access_token: "abc123".to_string(),
            token_type: "bearer".to_string(),
            scope: "repo".to_string(),
            created_at: 1700000000,
        };

        let json = serde_json::to_string(&token).unwrap();
        let parsed: Token = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "abc123");
    }

    #[test]
    fn token_deserialize_datetime_string() {
        // This is the case that was failing — created_at as ISO 8601 string
        let json = r#"{
            "access_token": "gho_test",
            "token_type": "bearer",
            "scope": "repo",
            "created_at": "2026-02-27T07:34:31.241364287Z"
        }"#;
        let token: Token = serde_json::from_str(json).unwrap();
        assert_eq!(token.access_token, "gho_test");
        assert!(token.created_at > 0); // should be current time, not 0
    }

    #[test]
    fn token_deserialize_integer() {
        let json = r#"{"access_token":"t","token_type":"","scope":"","created_at":1700000000}"#;
        let token: Token = serde_json::from_str(json).unwrap();
        assert_eq!(token.created_at, 1700000000);
    }

    #[test]
    fn token_deserialize_missing_created_at() {
        let json = r#"{"access_token":"t"}"#;
        let token: Token = serde_json::from_str(json).unwrap();
        assert_eq!(token.created_at, 0); // default
    }

    #[test]
    fn token_storage_serialization() {
        let storage = TokenStorage {
            github: Some(Token {
                access_token: "gh".to_string(),
                token_type: "bearer".to_string(),
                scope: "repo".to_string(),
                created_at: 0,
            }),
            gitlab: None,
        };

        let json = serde_json::to_string_pretty(&storage).unwrap();
        assert!(json.contains("github"));
        assert!(!json.contains("gitlab")); // skip_serializing_if = None

        let parsed: TokenStorage = serde_json::from_str(&json).unwrap();
        assert!(parsed.github.is_some());
        assert!(parsed.gitlab.is_none());
    }
}

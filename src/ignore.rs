//! Ignore pattern matching for Ivaldi VCS.
//!
//! Loads patterns from `.ivaldiignore` files and provides efficient matching
//! via `PatternCache`. Used by workspace scanning and `gather` operations.
//!
//! Pattern syntax:
//! - `*.log` — glob match on basename
//! - `build/` — directory match (trailing slash)
//! - `**/*.tmp` — recursive glob match
//! - `node_modules` — literal match
//! - Lines starting with `#` are comments
//! - Empty lines are ignored
//!
//! `.ivaldiignore` itself is NEVER ignored.

use std::fs;
use std::path::Path;

/// Built-in ignore patterns always applied.
pub const DEFAULT_PATTERNS: &[&str] = &[
    ".git/",
    ".svn/",
    ".hg/",
    ".fossil/",
    ".claude/",
];

/// Auto-excluded files for security (always ignored regardless of .ivaldiignore).
pub const SECURITY_PATTERNS: &[&str] = &[
    ".env",
    ".env.*",
    ".venv",
    ".venv/",
];

/// Pre-compiled pattern cache for fast matching.
pub struct PatternCache {
    dir_patterns: Vec<String>,
    glob_patterns: Vec<String>,
    double_star_patterns: Vec<(String, String)>, // (prefix, suffix)
    literal_patterns: Vec<String>,
}

impl PatternCache {
    /// Create a new cache from a list of patterns.
    pub fn new(patterns: &[&str]) -> Self {
        let mut cache = Self {
            dir_patterns: Vec::new(),
            glob_patterns: Vec::new(),
            double_star_patterns: Vec::new(),
            literal_patterns: Vec::new(),
        };

        for &pattern in patterns {
            if pattern.ends_with('/') {
                cache
                    .dir_patterns
                    .push(pattern.trim_end_matches('/').to_string());
            } else if pattern.contains("**") {
                let parts: Vec<&str> = pattern.splitn(2, "**").collect();
                if parts.len() == 2 {
                    let prefix = parts[0].trim_start_matches('/').to_string();
                    let suffix = parts[1].trim_start_matches('/').to_string();
                    cache.double_star_patterns.push((prefix, suffix));
                }
            } else if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
                cache.glob_patterns.push(pattern.to_string());
            } else {
                cache.literal_patterns.push(pattern.to_string());
            }
        }

        cache
    }

    /// Create a cache from owned string patterns.
    pub fn from_strings(patterns: &[String]) -> Self {
        let refs: Vec<&str> = patterns.iter().map(|s| s.as_str()).collect();
        Self::new(&refs)
    }

    /// Check if a file path should be ignored.
    /// `.ivaldiignore` itself is never ignored.
    pub fn is_ignored(&self, path: &str) -> bool {
        let basename = path_basename(path);

        // .ivaldiignore is never ignored
        if basename == ".ivaldiignore" {
            return false;
        }

        // Literal match (full path or basename)
        for pattern in &self.literal_patterns {
            if path == pattern || basename == pattern {
                return true;
            }
        }

        // Security pattern match (basename with glob)
        for &sec_pattern in SECURITY_PATTERNS {
            if sec_pattern.contains('*') || sec_pattern.contains('?') {
                if glob_match(sec_pattern, basename) {
                    return true;
                }
            } else if sec_pattern.ends_with('/') {
                let dir = sec_pattern.trim_end_matches('/');
                if path == dir || path.starts_with(&format!("{}/", dir)) || basename == dir {
                    return true;
                }
            } else if basename == sec_pattern || path == sec_pattern {
                return true;
            }
        }

        // Directory patterns
        for dir_pattern in &self.dir_patterns {
            if path.starts_with(&format!("{}/", dir_pattern))
                || path == dir_pattern.as_str()
                || basename == dir_pattern.as_str()
            {
                return true;
            }
        }

        // Glob patterns (no **)
        for pattern in &self.glob_patterns {
            if glob_match(pattern, path) || glob_match(pattern, basename) {
                return true;
            }
        }

        // Double-star patterns
        for (prefix, suffix) in &self.double_star_patterns {
            if !prefix.is_empty() && !path.starts_with(prefix.as_str()) {
                continue;
            }
            if suffix.is_empty() {
                if prefix.is_empty() {
                    return true; // "**" matches everything
                }
                return true; // "prefix/**" matches anything under prefix
            }
            // Match suffix against basename
            if glob_match(suffix, basename) {
                return true;
            }
        }

        false
    }

    /// Check if a directory should be skipped during traversal.
    pub fn is_dir_ignored(&self, dir_path: &str) -> bool {
        let basename = path_basename(dir_path);

        for pattern in &self.literal_patterns {
            if dir_path == pattern || basename == pattern {
                return true;
            }
        }

        for dir_pattern in &self.dir_patterns {
            if dir_path == dir_pattern.as_str()
                || dir_path.starts_with(&format!("{}/", dir_pattern))
                || basename == dir_pattern.as_str()
            {
                return true;
            }
        }

        // Security directory patterns
        for &sec_pattern in SECURITY_PATTERNS {
            if sec_pattern.ends_with('/') {
                let dir = sec_pattern.trim_end_matches('/');
                if dir_path == dir || basename == dir {
                    return true;
                }
            }
        }

        false
    }
}

/// Load patterns from a `.ivaldiignore` file.
pub fn load_patterns(work_dir: &Path) -> Vec<String> {
    let ignore_file = work_dir.join(".ivaldiignore");
    let content = match fs::read_to_string(&ignore_file) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect()
}

/// Load a complete pattern cache: default patterns + .ivaldiignore + security patterns.
pub fn load_pattern_cache(work_dir: &Path) -> PatternCache {
    let user_patterns = load_patterns(work_dir);
    let mut all: Vec<String> = DEFAULT_PATTERNS.iter().map(|s| s.to_string()).collect();
    all.extend(user_patterns);
    PatternCache::from_strings(&all)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn path_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Simple glob matching supporting `*`, `?`, and `[...]` character classes.
fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_impl(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_impl(pattern: &[u8], text: &[u8]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = usize::MAX;
    let mut star_ti = 0;

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if pi < pattern.len() && pattern[pi] == b'[' {
            // Character class
            if let Some((matched, end)) = match_char_class(&pattern[pi..], text[ti]) {
                if matched {
                    pi += end;
                    ti += 1;
                } else if star_pi != usize::MAX {
                    pi = star_pi + 1;
                    star_ti += 1;
                    ti = star_ti;
                } else {
                    return false;
                }
            } else if star_pi != usize::MAX {
                pi = star_pi + 1;
                star_ti += 1;
                ti = star_ti;
            } else {
                return false;
            }
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }

    pi == pattern.len()
}

/// Match a character class `[...]`. Returns (matched, bytes_consumed) or None if invalid.
fn match_char_class(pattern: &[u8], ch: u8) -> Option<(bool, usize)> {
    if pattern.is_empty() || pattern[0] != b'[' {
        return None;
    }

    let mut i = 1;
    let negate = i < pattern.len() && pattern[i] == b'!';
    if negate {
        i += 1;
    }

    let mut matched = false;
    while i < pattern.len() && pattern[i] != b']' {
        if i + 2 < pattern.len() && pattern[i + 1] == b'-' {
            if ch >= pattern[i] && ch <= pattern[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if ch == pattern[i] {
                matched = true;
            }
            i += 1;
        }
    }

    if i >= pattern.len() {
        return None; // No closing ]
    }

    if negate {
        matched = !matched;
    }

    Some((matched, i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_match() {
        let cache = PatternCache::new(&["Thumbs.db", ".DS_Store"]);
        assert!(cache.is_ignored("Thumbs.db"));
        assert!(cache.is_ignored(".DS_Store"));
        assert!(!cache.is_ignored("README.md"));
    }

    #[test]
    fn dir_pattern() {
        let cache = PatternCache::new(&["node_modules/", "build/"]);
        assert!(cache.is_ignored("node_modules"));
        assert!(cache.is_ignored("node_modules/package.json"));
        assert!(cache.is_ignored("build/output.js"));
        assert!(!cache.is_ignored("builder.js"));
    }

    #[test]
    fn glob_star() {
        let cache = PatternCache::new(&["*.log", "*.tmp"]);
        assert!(cache.is_ignored("error.log"));
        assert!(cache.is_ignored("debug.tmp"));
        assert!(cache.is_ignored("src/debug.log"));
        assert!(!cache.is_ignored("log.txt"));
    }

    #[test]
    fn glob_question() {
        let cache = PatternCache::new(&["test?.txt"]);
        assert!(cache.is_ignored("test1.txt"));
        assert!(cache.is_ignored("testA.txt"));
        assert!(!cache.is_ignored("test12.txt"));
    }

    #[test]
    fn double_star() {
        let cache = PatternCache::new(&["**/*.tmp"]);
        assert!(cache.is_ignored("file.tmp"));
        assert!(cache.is_ignored("src/file.tmp"));
        assert!(cache.is_ignored("a/b/c/file.tmp"));
        assert!(!cache.is_ignored("file.log"));
    }

    #[test]
    fn double_star_with_prefix() {
        let cache = PatternCache::new(&["test/**"]);
        assert!(cache.is_ignored("test/file.txt"));
        assert!(cache.is_ignored("test/sub/file.txt"));
        assert!(!cache.is_ignored("src/file.txt"));
    }

    #[test]
    fn ivaldiignore_never_ignored() {
        let cache = PatternCache::new(&[".*", ".ivaldiignore"]);
        assert!(!cache.is_ignored(".ivaldiignore"));
        assert!(!cache.is_ignored("src/.ivaldiignore"));
    }

    #[test]
    fn security_auto_excluded() {
        let cache = PatternCache::new(&[]);
        assert!(cache.is_ignored(".env"));
        assert!(cache.is_ignored(".env.local"));
        assert!(cache.is_ignored(".env.production"));
        assert!(cache.is_ignored(".venv"));
        assert!(cache.is_ignored(".venv/lib/python3/site-packages"));
    }

    #[test]
    fn default_patterns() {
        let cache = PatternCache::new(DEFAULT_PATTERNS);
        assert!(cache.is_ignored(".git/config"));
        assert!(cache.is_ignored(".svn/entries"));
    }

    #[test]
    fn dir_ignored_for_pruning() {
        let cache = PatternCache::new(&["node_modules/", "dist/"]);
        assert!(cache.is_dir_ignored("node_modules"));
        assert!(cache.is_dir_ignored("dist"));
        assert!(!cache.is_dir_ignored("src"));
    }

    #[test]
    fn load_patterns_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let ignore_path = dir.path().join(".ivaldiignore");
        fs::write(
            &ignore_path,
            "# Comment\n\n*.log\nbuild/\nnode_modules/\n",
        )
        .unwrap();

        let patterns = load_patterns(dir.path());
        assert_eq!(patterns, vec!["*.log", "build/", "node_modules/"]);
    }

    #[test]
    fn load_patterns_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let patterns = load_patterns(dir.path());
        assert!(patterns.is_empty());
    }

    #[test]
    fn load_pattern_cache_includes_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cache = load_pattern_cache(dir.path());
        // Default patterns should work
        assert!(cache.is_dir_ignored(".git"));
    }

    #[test]
    fn glob_character_class() {
        let cache = PatternCache::new(&["file[0-9].txt"]);
        assert!(cache.is_ignored("file3.txt"));
        assert!(cache.is_ignored("file9.txt"));
        assert!(!cache.is_ignored("fileA.txt"));
    }

    #[test]
    fn combined_patterns() {
        let cache = PatternCache::new(&[
            "*.log",
            "build/",
            "node_modules/",
            ".DS_Store",
            "**/*.bak",
        ]);
        assert!(cache.is_ignored("error.log"));
        assert!(cache.is_ignored("build/output.js"));
        assert!(cache.is_ignored("node_modules/express/index.js"));
        assert!(cache.is_ignored(".DS_Store"));
        assert!(cache.is_ignored("src/old.bak"));
        assert!(!cache.is_ignored("README.md"));
    }
}

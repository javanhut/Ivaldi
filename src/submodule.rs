//! Submodule support for Ivaldi VCS.
//!
//! Detects Git submodules during forge/download and converts them
//! to Ivaldi submodule references.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A submodule configuration entry.
#[derive(Debug, Clone)]
pub struct Submodule {
    pub name: String,
    pub path: String,
    pub url: String,
    pub branch: Option<String>,
}

/// Parse `.gitmodules` file to extract submodule configurations.
pub fn parse_gitmodules(work_dir: &Path) -> Vec<Submodule> {
    let gitmodules_path = work_dir.join(".gitmodules");
    let content = match fs::read_to_string(&gitmodules_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut modules = Vec::new();
    let mut current: Option<Submodule> = None;

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with("[submodule") {
            // Save previous submodule
            if let Some(m) = current.take() {
                modules.push(m);
            }
            // Parse name from [submodule "name"]
            let name = line
                .trim_start_matches("[submodule \"")
                .trim_end_matches("\"]")
                .to_string();
            current = Some(Submodule {
                name,
                path: String::new(),
                url: String::new(),
                branch: None,
            });
        } else if let Some(ref mut m) = current {
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "path" => m.path = value.to_string(),
                    "url" => m.url = value.to_string(),
                    "branch" => m.branch = Some(value.to_string()),
                    _ => {}
                }
            }
        }
    }

    if let Some(m) = current {
        modules.push(m);
    }

    modules
}

/// Detect submodules in a working directory.
pub fn detect_submodules(work_dir: &Path) -> Vec<Submodule> {
    let mut found = parse_gitmodules(work_dir);

    // Also check for directories with their own .git
    if let Ok(entries) = fs::read_dir(work_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" || name == ".ivaldi" {
                    continue;
                }
                let sub_git = entry.path().join(".git");
                if sub_git.exists() {
                    // Check if already in gitmodules
                    if !found.iter().any(|m| m.path == name) {
                        found.push(Submodule {
                            name: name.clone(),
                            path: name,
                            url: String::new(),
                            branch: None,
                        });
                    }
                }
            }
        }
    }

    found
}

/// Manage submodule registrations for a repository.
pub struct SubmoduleManager {
    modules: BTreeMap<String, Submodule>,
    ivaldi_dir: PathBuf,
}

impl SubmoduleManager {
    pub fn new(ivaldi_dir: &Path) -> Self {
        let modules_file = ivaldi_dir.join("submodules");
        let mut modules = BTreeMap::new();

        if let Ok(content) = fs::read_to_string(&modules_file) {
            for line in content.lines() {
                let parts: Vec<&str> = line.splitn(4, '\t').collect();
                if parts.len() >= 3 {
                    modules.insert(
                        parts[0].to_string(),
                        Submodule {
                            name: parts[0].to_string(),
                            path: parts[1].to_string(),
                            url: parts[2].to_string(),
                            branch: parts.get(3).map(|s| s.to_string()),
                        },
                    );
                }
            }
        }

        Self {
            modules,
            ivaldi_dir: ivaldi_dir.to_path_buf(),
        }
    }

    pub fn add(&mut self, module: Submodule) {
        self.modules.insert(module.name.clone(), module);
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.modules.remove(name).is_some()
    }

    pub fn list(&self) -> Vec<&Submodule> {
        self.modules.values().collect()
    }

    pub fn get(&self, name: &str) -> Option<&Submodule> {
        self.modules.get(name)
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let mut lines = Vec::new();
        for m in self.modules.values() {
            let branch = m.branch.as_deref().unwrap_or("");
            lines.push(format!("{}\t{}\t{}\t{}", m.name, m.path, m.url, branch));
        }
        fs::write(self.ivaldi_dir.join("submodules"), lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gitmodules_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(".gitmodules"),
            r#"
[submodule "lib/crypto"]
    path = lib/crypto
    url = https://github.com/example/crypto.git
    branch = main

[submodule "vendor/tools"]
    path = vendor/tools
    url = https://github.com/example/tools.git
"#,
        )
        .unwrap();

        let modules = parse_gitmodules(dir.path());
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].name, "lib/crypto");
        assert_eq!(modules[0].path, "lib/crypto");
        assert_eq!(modules[0].branch, Some("main".to_string()));
        assert_eq!(modules[1].name, "vendor/tools");
    }

    #[test]
    fn parse_no_gitmodules() {
        let dir = tempfile::tempdir().unwrap();
        let modules = parse_gitmodules(dir.path());
        assert!(modules.is_empty());
    }

    #[test]
    fn submodule_manager() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        let mut mgr = SubmoduleManager::new(&ivaldi_dir);
        mgr.add(Submodule {
            name: "lib".into(),
            path: "lib".into(),
            url: "https://example.com/lib.git".into(),
            branch: Some("main".into()),
        });
        mgr.save().unwrap();

        // Reload
        let mgr2 = SubmoduleManager::new(&ivaldi_dir);
        assert_eq!(mgr2.list().len(), 1);
        assert_eq!(mgr2.get("lib").unwrap().url, "https://example.com/lib.git");
    }

    #[test]
    fn detect_submodules_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect_submodules(dir.path()).is_empty());
    }
}

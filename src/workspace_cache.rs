//! Disposable working-file hash cache. A miss always falls back to reading
//! content. Never use this cache as evidence that an object exists in the CAS.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::hash::B3Hash;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stamp {
    device: u64,
    inode: u64,
    size: u64,
    mode: u32,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Stamp {
    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    pub(crate) fn read(path: &Path) -> Option<Self> {
        // Platforms without change-time and stable identity fall back to
        // content reads. mtime + size alone misses restored-timestamp edits.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = fs::symlink_metadata(path).ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(Self {
                device: meta.dev(),
                inode: meta.ino(),
                size: meta.size(),
                mode: meta.mode(),
                modified: (meta.mtime(), meta.mtime_nsec()),
                changed: (meta.ctime(), meta.ctime_nsec()),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            None
        }
    }

    fn settled(&self, observed: u64) -> bool {
        // Exclude the timestamp race window, including coarse-resolution
        // filesystems. Future timestamps are deliberately never cached.
        [self.modified.0, self.changed.0]
            .iter()
            .all(|t| *t >= 0 && (*t as u64).saturating_add(2) < observed)
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    stamp: Stamp,
    hash: String,
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct FileCache {
    entries: BTreeMap<String, Entry>,
    #[serde(skip)]
    dirty: bool,
}

impl FileCache {
    pub(crate) fn load(dir: &Path) -> Self {
        let load = || -> Option<Self> {
            let bytes = fs::read(dir.join("workspace-cache-v1")).ok()?;
            let (checksum, body) = bytes.split_at_checked(32)?;
            if B3Hash::digest(body).as_bytes() != checksum {
                return None;
            }
            serde_json::from_slice(body).ok()
        };
        load().unwrap_or_default()
    }

    pub(crate) fn lookup(&self, path: &str, stamp: &Option<Stamp>) -> Option<B3Hash> {
        let entry = self.entries.get(path)?;
        if Some(&entry.stamp) != stamp.as_ref() {
            return None;
        }
        B3Hash::from_hex(&entry.hash)
    }

    pub(crate) fn record(
        &mut self,
        path: &str,
        before: Option<Stamp>,
        after: Option<Stamp>,
        hash: B3Hash,
        observed: u64,
    ) {
        if let Some(stamp) = before
            && after.as_ref() == Some(&stamp)
            && stamp.settled(observed)
        {
            self.entries.insert(
                path.to_owned(),
                Entry {
                    stamp,
                    hash: hash.to_hex(),
                },
            );
            self.dirty = true;
        } else if self.entries.remove(path).is_some() {
            self.dirty = true;
        }
    }

    pub(crate) fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub(crate) fn save(&mut self, dir: &Path) {
        if !self.dirty {
            return;
        }
        if let Ok(body) = serde_json::to_vec(self) {
            let mut bytes = B3Hash::digest(&body).as_bytes().to_vec();
            bytes.extend(body);
            if crate::atomic_io::atomic_write(&dir.join("workspace-cache-v1"), &bytes).is_ok() {
                self.dirty = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> Stamp {
        Stamp {
            device: 1,
            inode: 2,
            size: 3,
            mode: 0o100644,
            modified: (10, 0),
            changed: (10, 0),
        }
    }

    #[test]
    fn timestamp_races_and_mutations_are_not_cached() {
        let mut cache = FileCache::default();
        let hash = B3Hash::digest(b"abc");
        let before = stamp();
        cache.record("file", Some(before.clone()), Some(before.clone()), hash, 12);
        assert_eq!(cache.lookup("file", &Some(before.clone())), None);
        cache.record("file", Some(before.clone()), Some(before.clone()), hash, 13);
        assert_eq!(cache.lookup("file", &Some(before.clone())), Some(hash));
        let mut changed = before.clone();
        changed.changed.1 = 1;
        assert_eq!(cache.lookup("file", &Some(changed.clone())), None);
        cache.record("file", Some(before.clone()), Some(changed), hash, 13);
        assert_eq!(cache.lookup("file", &Some(before)), None);
    }

    #[test]
    fn corrupt_cache_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = FileCache::default();
        cache.record(
            "file",
            Some(stamp()),
            Some(stamp()),
            B3Hash::digest(b"abc"),
            20,
        );
        cache.save(dir.path());
        assert!(
            FileCache::load(dir.path())
                .lookup("file", &Some(stamp()))
                .is_some()
        );
        let path = dir.path().join("workspace-cache-v1");
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(path, bytes).unwrap();
        assert!(FileCache::load(dir.path()).entries.is_empty());
    }
}

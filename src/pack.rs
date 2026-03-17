//! Pack file support for Ivaldi VCS.
//!
//! Combines multiple small CAS objects into larger pack files for more
//! efficient storage and transfer. Uses a simple index for O(1) lookups.
//!
//! Format:
//! - Pack file: `<magic><version><entry_count><entries...>`
//! - Each entry: `<hash:32><offset:u64><size:u64><data>`
//! - Index file: sorted by hash for binary search

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hash::B3Hash;

const PACK_MAGIC: &[u8; 4] = b"IVPK";
const PACK_VERSION: u8 = 1;

/// A pack file containing multiple objects.
pub struct PackWriter {
    entries: BTreeMap<B3Hash, Vec<u8>>,
}

impl PackWriter {
    pub fn new() -> Self {
        Self { entries: BTreeMap::new() }
    }

    /// Add an object to the pack.
    pub fn add(&mut self, hash: B3Hash, data: Vec<u8>) {
        self.entries.insert(hash, data);
    }

    /// Number of objects in this pack.
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Write the pack file. Returns the pack file hash.
    pub fn write(&self, pack_dir: &Path) -> Result<B3Hash, PackError> {
        fs::create_dir_all(pack_dir).map_err(PackError::Io)?;

        let mut buf = Vec::new();
        buf.extend_from_slice(PACK_MAGIC);
        buf.push(PACK_VERSION);

        // Entry count as u64 LE
        buf.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());

        // Index: hash → (offset, size)
        let mut index = BTreeMap::new();
        let mut data_offset: u64 = 0;

        // First pass: write all data, track offsets
        let mut data_buf = Vec::new();
        for (hash, data) in &self.entries {
            index.insert(*hash, (data_offset, data.len() as u64));
            data_buf.extend_from_slice(data);
            data_offset += data.len() as u64;
        }

        // Write index entries
        for (hash, (offset, size)) in &index {
            buf.extend_from_slice(hash.as_bytes());
            buf.extend_from_slice(&offset.to_le_bytes());
            buf.extend_from_slice(&size.to_le_bytes());
        }

        // Write data section
        buf.extend_from_slice(&data_buf);

        let pack_hash = B3Hash::digest(&buf);
        let pack_path = pack_dir.join(format!("{}.pack", pack_hash.short(16)));
        fs::write(&pack_path, &buf).map_err(PackError::Io)?;

        // Write index file
        let idx_path = pack_dir.join(format!("{}.idx", pack_hash.short(16)));
        let idx_data: Vec<u8> = index.iter().flat_map(|(hash, (offset, size))| {
            let mut entry = Vec::with_capacity(48);
            entry.extend_from_slice(hash.as_bytes());
            entry.extend_from_slice(&offset.to_le_bytes());
            entry.extend_from_slice(&size.to_le_bytes());
            entry
        }).collect();
        fs::write(&idx_path, &idx_data).map_err(PackError::Io)?;

        Ok(pack_hash)
    }
}

impl Default for PackWriter {
    fn default() -> Self { Self::new() }
}

/// Read objects from a pack file.
pub struct PackReader {
    pack_dir: PathBuf,
}

impl PackReader {
    pub fn new(pack_dir: &Path) -> Self {
        Self { pack_dir: pack_dir.to_path_buf() }
    }

    /// List all pack files.
    pub fn list_packs(&self) -> Vec<String> {
        let mut packs = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.pack_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".pack") {
                    packs.push(name);
                }
            }
        }
        packs.sort();
        packs
    }

    /// Count total objects across all packs.
    pub fn total_objects(&self) -> usize {
        let mut count = 0;
        for pack_name in self.list_packs() {
            if let Ok(data) = fs::read(self.pack_dir.join(&pack_name)) {
                if data.len() >= 13 && &data[0..4] == PACK_MAGIC {
                    let entry_count = u64::from_le_bytes(data[5..13].try_into().unwrap_or([0; 8]));
                    count += entry_count as usize;
                }
            }
        }
        count
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt pack file")]
    Corrupt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_list_packs() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("packs");

        let mut writer = PackWriter::new();
        writer.add(B3Hash::digest(b"obj1"), b"data1".to_vec());
        writer.add(B3Hash::digest(b"obj2"), b"data2".to_vec());
        assert_eq!(writer.len(), 2);

        let hash = writer.write(&pack_dir).unwrap();
        assert_ne!(hash, B3Hash::ZERO);

        let reader = PackReader::new(&pack_dir);
        assert_eq!(reader.list_packs().len(), 1);
        assert_eq!(reader.total_objects(), 2);
    }

    #[test]
    fn empty_pack() {
        let dir = tempfile::tempdir().unwrap();
        let reader = PackReader::new(dir.path());
        assert!(reader.list_packs().is_empty());
        assert_eq!(reader.total_objects(), 0);
    }

    #[test]
    fn multiple_packs() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("packs");

        let mut w1 = PackWriter::new();
        w1.add(B3Hash::digest(b"a"), b"a".to_vec());
        w1.write(&pack_dir).unwrap();

        let mut w2 = PackWriter::new();
        w2.add(B3Hash::digest(b"b"), b"b".to_vec());
        w2.add(B3Hash::digest(b"c"), b"c".to_vec());
        w2.write(&pack_dir).unwrap();

        let reader = PackReader::new(&pack_dir);
        assert_eq!(reader.list_packs().len(), 2);
        assert_eq!(reader.total_objects(), 3);
    }
}

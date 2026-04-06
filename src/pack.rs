//! Pack file support for Ivaldi VCS.
//!
//! Combines multiple small CAS objects into larger pack files for more
//! efficient storage and transfer. Uses a simple index for O(1) lookups.
//!
//! Format v1 (full objects only):
//! - Pack file: `<magic><version><entry_count><index_entries><data>`
//! - Each index entry: `<hash:32><offset:u64><size:u64>`
//!
//! Format v2 (with delta compression):
//! - Pack file: `<magic><version><entry_count><index_entries><data>`
//! - Each index entry: `<hash:32><offset:u64><size:u64><entry_type:u8>`
//! - entry_type 0 = full object, 1 = delta (first 32 bytes of data = base hash)
//!
//! Delta encoding uses COPY(offset, len) / INSERT(len, data) instructions.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cas::{Cas, FileCas};
use crate::hash::B3Hash;

const PACK_MAGIC: &[u8; 4] = b"IVPK";
const PACK_VERSION: u8 = 1;
const PACK_VERSION_DELTA: u8 = 2;
const ENTRY_FULL: u8 = 0;
const ENTRY_DELTA: u8 = 1;

/// Delta instruction opcodes.
const OP_COPY: u8 = 0;
const OP_INSERT: u8 = 1;

/// Minimum savings ratio to use delta (25% savings required).
const DELTA_MIN_SAVINGS: f64 = 0.25;

/// A pack file containing multiple objects.
pub struct PackWriter {
    entries: BTreeMap<B3Hash, Vec<u8>>,
}

impl PackWriter {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Add an object to the pack.
    pub fn add(&mut self, hash: B3Hash, data: Vec<u8>) {
        self.entries.insert(hash, data);
    }

    /// Number of objects in this pack.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Write the pack file (v1, no deltas). Returns the pack file hash.
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
        let idx_data: Vec<u8> = index
            .iter()
            .flat_map(|(hash, (offset, size))| {
                let mut entry = Vec::with_capacity(48);
                entry.extend_from_slice(hash.as_bytes());
                entry.extend_from_slice(&offset.to_le_bytes());
                entry.extend_from_slice(&size.to_le_bytes());
                entry
            })
            .collect();
        fs::write(&idx_path, &idx_data).map_err(PackError::Io)?;

        Ok(pack_hash)
    }

    /// Write a v2 pack file with delta compression. Returns the pack file hash.
    ///
    /// Objects are sorted by hash. Each object is delta-compressed against the
    /// previous one if savings exceed 25%.
    pub fn write_delta(&self, pack_dir: &Path) -> Result<B3Hash, PackError> {
        fs::create_dir_all(pack_dir).map_err(PackError::Io)?;

        let mut buf = Vec::new();
        buf.extend_from_slice(PACK_MAGIC);
        buf.push(PACK_VERSION_DELTA);
        buf.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());

        // Index: hash → (offset, size, entry_type)
        let mut index: BTreeMap<B3Hash, (u64, u64, u8)> = BTreeMap::new();
        let mut data_buf = Vec::new();
        let mut data_offset: u64 = 0;

        let entries_vec: Vec<(&B3Hash, &Vec<u8>)> = self.entries.iter().collect();

        for (i, (hash, data)) in entries_vec.iter().enumerate() {
            // Try delta against previous entry
            let (stored_data, entry_type) = if i > 0 {
                let (_prev_hash, prev_data) = entries_vec[i - 1];
                let delta = compute_delta(prev_data, data);
                let savings = 1.0 - (delta.len() as f64 / data.len().max(1) as f64);
                if savings >= DELTA_MIN_SAVINGS && delta.len() + 32 < data.len() {
                    // Store delta: base_hash(32) + delta_bytes
                    let mut delta_data = Vec::with_capacity(32 + delta.len());
                    delta_data.extend_from_slice(entries_vec[i - 1].0.as_bytes());
                    delta_data.extend_from_slice(&delta);
                    (delta_data, ENTRY_DELTA)
                } else {
                    (data.to_vec(), ENTRY_FULL)
                }
            } else {
                (data.to_vec(), ENTRY_FULL)
            };

            index.insert(**hash, (data_offset, stored_data.len() as u64, entry_type));
            data_buf.extend_from_slice(&stored_data);
            data_offset += stored_data.len() as u64;
        }

        // Write index entries (v2: includes entry_type byte)
        for (hash, (offset, size, etype)) in &index {
            buf.extend_from_slice(hash.as_bytes());
            buf.extend_from_slice(&offset.to_le_bytes());
            buf.extend_from_slice(&size.to_le_bytes());
            buf.push(*etype);
        }

        buf.extend_from_slice(&data_buf);

        let pack_hash = B3Hash::digest(&buf);
        let pack_path = pack_dir.join(format!("{}.pack", pack_hash.short(16)));
        fs::write(&pack_path, &buf).map_err(PackError::Io)?;

        // Write v2 index file
        let idx_path = pack_dir.join(format!("{}.idx", pack_hash.short(16)));
        let idx_data: Vec<u8> = index
            .iter()
            .flat_map(|(hash, (offset, size, etype))| {
                let mut entry = Vec::with_capacity(49);
                entry.extend_from_slice(hash.as_bytes());
                entry.extend_from_slice(&offset.to_le_bytes());
                entry.extend_from_slice(&size.to_le_bytes());
                entry.push(*etype);
                entry
            })
            .collect();
        fs::write(&idx_path, &idx_data).map_err(PackError::Io)?;

        Ok(pack_hash)
    }
}

impl Default for PackWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Read objects from a pack file.
pub struct PackReader {
    pack_dir: PathBuf,
}

impl PackReader {
    pub fn new(pack_dir: &Path) -> Self {
        Self {
            pack_dir: pack_dir.to_path_buf(),
        }
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

    /// Get a single object by hash from all pack files.
    ///
    /// Searches all packs. For v2 packs, automatically resolves delta chains.
    pub fn get_object(&self, hash: B3Hash) -> Result<Vec<u8>, PackError> {
        for pack_name in self.list_packs() {
            let data = fs::read(self.pack_dir.join(&pack_name)).map_err(PackError::Io)?;
            if let Some(obj) = self.get_from_pack_data(&data, hash)? {
                return Ok(obj);
            }
        }
        Err(PackError::NotFound(hash))
    }

    /// Extract all objects from packs into a CAS.
    pub fn extract_to_cas(&self, cas: &FileCas) -> Result<usize, PackError> {
        let mut count = 0;
        for pack_name in self.list_packs() {
            let data = fs::read(self.pack_dir.join(&pack_name)).map_err(PackError::Io)?;
            if data.len() < 13 || &data[0..4] != PACK_MAGIC {
                continue;
            }
            let version = data[4];
            let entry_count = u64::from_le_bytes(data[5..13].try_into().unwrap_or([0; 8])) as usize;

            match version {
                PACK_VERSION => {
                    // v1: 48-byte index entries (hash:32 + offset:8 + size:8)
                    let index_size = entry_count * 48;
                    let index_start = 13;
                    let data_start = index_start + index_size;

                    for i in 0..entry_count {
                        let idx_off = index_start + i * 48;
                        if idx_off + 48 > data.len() {
                            break;
                        }
                        let hash = B3Hash::from_slice(&data[idx_off..idx_off + 32]).unwrap();
                        let offset = u64::from_le_bytes(
                            data[idx_off + 32..idx_off + 40].try_into().unwrap(),
                        ) as usize;
                        let size = u64::from_le_bytes(
                            data[idx_off + 40..idx_off + 48].try_into().unwrap(),
                        ) as usize;

                        let abs_offset = data_start + offset;
                        if abs_offset + size <= data.len() {
                            let obj_data = &data[abs_offset..abs_offset + size];
                            cas.put(hash, obj_data)
                                .map_err(|e| PackError::Other(e.to_string()))?;
                            count += 1;
                        }
                    }
                }
                PACK_VERSION_DELTA => {
                    // v2: 49-byte index entries (hash:32 + offset:8 + size:8 + type:1)
                    let index_size = entry_count * 49;
                    let index_start = 13;
                    let data_start = index_start + index_size;

                    // First pass: parse index
                    let mut entries: Vec<(B3Hash, usize, usize, u8)> =
                        Vec::with_capacity(entry_count);
                    for i in 0..entry_count {
                        let idx_off = index_start + i * 49;
                        if idx_off + 49 > data.len() {
                            break;
                        }
                        let hash = B3Hash::from_slice(&data[idx_off..idx_off + 32]).unwrap();
                        let offset = u64::from_le_bytes(
                            data[idx_off + 32..idx_off + 40].try_into().unwrap(),
                        ) as usize;
                        let size = u64::from_le_bytes(
                            data[idx_off + 40..idx_off + 48].try_into().unwrap(),
                        ) as usize;
                        let etype = data[idx_off + 48];
                        entries.push((hash, offset, size, etype));
                    }

                    // Build hash → index map for delta resolution
                    let hash_to_idx: BTreeMap<B3Hash, usize> = entries
                        .iter()
                        .enumerate()
                        .map(|(i, (h, _, _, _))| (*h, i))
                        .collect();

                    // Resolve and extract each entry
                    for (hash, offset, size, etype) in &entries {
                        let abs_offset = data_start + offset;
                        if abs_offset + size > data.len() {
                            continue;
                        }
                        let raw = &data[abs_offset..abs_offset + size];

                        let obj_data = if *etype == ENTRY_DELTA {
                            self.resolve_delta_chain(
                                raw,
                                &entries,
                                &hash_to_idx,
                                &data,
                                data_start,
                            )?
                        } else {
                            raw.to_vec()
                        };

                        cas.put(*hash, &obj_data)
                            .map_err(|e| PackError::Other(e.to_string()))?;
                        count += 1;
                    }
                }
                _ => continue,
            }
        }
        Ok(count)
    }

    fn get_from_pack_data(
        &self,
        data: &[u8],
        target_hash: B3Hash,
    ) -> Result<Option<Vec<u8>>, PackError> {
        if data.len() < 13 || &data[0..4] != PACK_MAGIC {
            return Ok(None);
        }
        let version = data[4];
        let entry_count = u64::from_le_bytes(data[5..13].try_into().unwrap_or([0; 8])) as usize;

        match version {
            PACK_VERSION => {
                let index_start = 13;
                let data_start = index_start + entry_count * 48;

                for i in 0..entry_count {
                    let idx_off = index_start + i * 48;
                    if idx_off + 48 > data.len() {
                        break;
                    }
                    let hash = B3Hash::from_slice(&data[idx_off..idx_off + 32]).unwrap();
                    if hash != target_hash {
                        continue;
                    }

                    let offset =
                        u64::from_le_bytes(data[idx_off + 32..idx_off + 40].try_into().unwrap())
                            as usize;
                    let size =
                        u64::from_le_bytes(data[idx_off + 40..idx_off + 48].try_into().unwrap())
                            as usize;
                    let abs_offset = data_start + offset;
                    if abs_offset + size > data.len() {
                        return Err(PackError::Corrupt);
                    }
                    return Ok(Some(data[abs_offset..abs_offset + size].to_vec()));
                }
                Ok(None)
            }
            PACK_VERSION_DELTA => {
                let index_start = 13;

                // Parse all index entries
                let mut entries: Vec<(B3Hash, usize, usize, u8)> = Vec::with_capacity(entry_count);
                for i in 0..entry_count {
                    let idx_off = index_start + i * 49;
                    if idx_off + 49 > data.len() {
                        break;
                    }
                    let hash = B3Hash::from_slice(&data[idx_off..idx_off + 32]).unwrap();
                    let offset =
                        u64::from_le_bytes(data[idx_off + 32..idx_off + 40].try_into().unwrap())
                            as usize;
                    let size =
                        u64::from_le_bytes(data[idx_off + 40..idx_off + 48].try_into().unwrap())
                            as usize;
                    let etype = data[idx_off + 48];
                    entries.push((hash, offset, size, etype));
                }

                let data_start = index_start + entry_count * 49;
                let hash_to_idx: BTreeMap<B3Hash, usize> = entries
                    .iter()
                    .enumerate()
                    .map(|(i, (h, _, _, _))| (*h, i))
                    .collect();

                // Find target entry
                let target_idx = match hash_to_idx.get(&target_hash) {
                    Some(&i) => i,
                    None => return Ok(None),
                };

                let (_, offset, size, etype) = &entries[target_idx];
                let abs_offset = data_start + offset;
                if abs_offset + size > data.len() {
                    return Err(PackError::Corrupt);
                }
                let raw = &data[abs_offset..abs_offset + size];

                if *etype == ENTRY_DELTA {
                    let resolved =
                        self.resolve_delta_chain(raw, &entries, &hash_to_idx, data, data_start)?;
                    Ok(Some(resolved))
                } else {
                    Ok(Some(raw.to_vec()))
                }
            }
            _ => Ok(None),
        }
    }

    fn resolve_delta_chain(
        &self,
        raw: &[u8],
        entries: &[(B3Hash, usize, usize, u8)],
        hash_to_idx: &BTreeMap<B3Hash, usize>,
        pack_data: &[u8],
        data_start: usize,
    ) -> Result<Vec<u8>, PackError> {
        if raw.len() < 32 {
            return Err(PackError::Corrupt);
        }
        let base_hash = B3Hash::from_slice(&raw[..32]).unwrap();
        let delta_bytes = &raw[32..];

        // Find base object
        let base_idx = hash_to_idx.get(&base_hash).ok_or(PackError::Corrupt)?;
        let (_, base_offset, base_size, base_etype) = &entries[*base_idx];
        let abs_base = data_start + base_offset;
        if abs_base + base_size > pack_data.len() {
            return Err(PackError::Corrupt);
        }
        let base_raw = &pack_data[abs_base..abs_base + base_size];

        // Recursively resolve if base is also a delta
        let base_data = if *base_etype == ENTRY_DELTA {
            self.resolve_delta_chain(base_raw, entries, hash_to_idx, pack_data, data_start)?
        } else {
            base_raw.to_vec()
        };

        Ok(apply_delta(&base_data, delta_bytes))
    }
}

// ---------------------------------------------------------------------------
// Delta compression
// ---------------------------------------------------------------------------

/// Compute a delta from `base` to `target`.
///
/// Uses COPY(offset, len) / INSERT(len, data) instructions.
/// COPY references bytes in the base object; INSERT provides new bytes.
pub fn compute_delta(base: &[u8], target: &[u8]) -> Vec<u8> {
    let mut delta = Vec::new();

    // Build a simple index of 4-byte windows from base
    let mut base_index: BTreeMap<&[u8], Vec<usize>> = BTreeMap::new();
    if base.len() >= 4 {
        for i in 0..=base.len() - 4 {
            base_index.entry(&base[i..i + 4]).or_default().push(i);
        }
    }

    let mut ti = 0; // position in target
    let mut insert_buf: Vec<u8> = Vec::new();

    while ti < target.len() {
        let mut best_offset = 0usize;
        let mut best_len = 0usize;

        // Try to find a match in base
        if ti + 4 <= target.len() {
            if let Some(positions) = base_index.get(&target[ti..ti + 4]) {
                for &pos in positions {
                    // Extend match as far as possible
                    let mut len = 0;
                    while pos + len < base.len()
                        && ti + len < target.len()
                        && base[pos + len] == target[ti + len]
                    {
                        len += 1;
                    }
                    if len > best_len {
                        best_len = len;
                        best_offset = pos;
                    }
                }
            }
        }

        if best_len >= 4 {
            // Flush any pending INSERT
            if !insert_buf.is_empty() {
                flush_insert(&mut delta, &insert_buf);
                insert_buf.clear();
            }
            // Emit COPY
            delta.push(OP_COPY);
            delta.extend_from_slice(&(best_offset as u32).to_le_bytes());
            delta.extend_from_slice(&(best_len as u32).to_le_bytes());
            ti += best_len;
        } else {
            insert_buf.push(target[ti]);
            ti += 1;
        }
    }

    // Flush remaining INSERT
    if !insert_buf.is_empty() {
        flush_insert(&mut delta, &insert_buf);
    }

    delta
}

fn flush_insert(delta: &mut Vec<u8>, buf: &[u8]) {
    delta.push(OP_INSERT);
    delta.extend_from_slice(&(buf.len() as u32).to_le_bytes());
    delta.extend_from_slice(buf);
}

/// Apply a delta to a base object, producing the target object.
pub fn apply_delta(base: &[u8], delta: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut di = 0;

    while di < delta.len() {
        let op = delta[di];
        di += 1;

        match op {
            OP_COPY => {
                if di + 8 > delta.len() {
                    break;
                }
                let offset = u32::from_le_bytes(delta[di..di + 4].try_into().unwrap()) as usize;
                di += 4;
                let len = u32::from_le_bytes(delta[di..di + 4].try_into().unwrap()) as usize;
                di += 4;
                if offset + len <= base.len() {
                    result.extend_from_slice(&base[offset..offset + len]);
                }
            }
            OP_INSERT => {
                if di + 4 > delta.len() {
                    break;
                }
                let len = u32::from_le_bytes(delta[di..di + 4].try_into().unwrap()) as usize;
                di += 4;
                if di + len <= delta.len() {
                    result.extend_from_slice(&delta[di..di + len]);
                    di += len;
                }
            }
            _ => break,
        }
    }

    result
}

// ---------------------------------------------------------------------------
// GC integration: pack loose objects
// ---------------------------------------------------------------------------

/// Result of packing loose objects.
#[derive(Debug)]
pub struct PackResult {
    pub objects_packed: usize,
    pub loose_removed: usize,
    pub pack_hash: B3Hash,
}

/// Pack all loose CAS objects into a delta-compressed pack file.
///
/// Scans the objects directory, creates a v2 pack, then removes the loose files.
pub fn pack_loose_objects(ivaldi_dir: &Path) -> Result<PackResult, PackError> {
    let objects_dir = ivaldi_dir.join("objects");
    let pack_dir = ivaldi_dir.join("packs");

    let all_objects =
        crate::gc::scan_all_objects(&objects_dir).map_err(|e| PackError::Other(e.to_string()))?;

    if all_objects.is_empty() {
        return Err(PackError::Other("no loose objects to pack".into()));
    }

    let mut writer = PackWriter::new();
    let mut paths_to_remove: Vec<PathBuf> = Vec::new();

    for (hash, path, _size) in &all_objects {
        let data = fs::read(path).map_err(PackError::Io)?;
        writer.add(*hash, data);
        paths_to_remove.push(path.clone());
    }

    let pack_hash = writer.write_delta(&pack_dir)?;

    // Remove loose objects
    let mut removed = 0;
    for path in &paths_to_remove {
        if fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }

    Ok(PackResult {
        objects_packed: writer.len(),
        loose_removed: removed,
        pack_hash,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt pack file")]
    Corrupt,
    #[error("object not found: {0}")]
    NotFound(B3Hash),
    #[error("{0}")]
    Other(String),
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

    #[test]
    fn delta_roundtrip() {
        let base = b"Hello, world! This is a test of the delta compression system.";
        let target = b"Hello, world! This is a test of the improved delta compression system.";

        let delta = compute_delta(base, target);
        let result = apply_delta(base, &delta);
        assert_eq!(result, target);
    }

    #[test]
    fn delta_identical() {
        let data = b"identical data here";
        let delta = compute_delta(data, data);
        let result = apply_delta(data, &delta);
        assert_eq!(result, data);
    }

    #[test]
    fn delta_completely_different() {
        let base = b"aaaa bbbb cccc";
        let target = b"xxxx yyyy zzzz";
        let delta = compute_delta(base, target);
        let result = apply_delta(base, &delta);
        assert_eq!(result, target);
    }

    #[test]
    fn delta_empty_base() {
        let base = b"";
        let target = b"new content";
        let delta = compute_delta(base, target);
        let result = apply_delta(base, &delta);
        assert_eq!(result, target);
    }

    #[test]
    fn delta_empty_target() {
        let base = b"old content";
        let target = b"";
        let delta = compute_delta(base, target);
        let result = apply_delta(base, &delta);
        assert_eq!(result, target);
    }

    #[test]
    fn delta_saves_space() {
        // Similar blobs should produce smaller delta than full object
        let base = b"line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\n";
        let target = b"line 1\nline 2\nline 3 modified\nline 4\nline 5\nline 6\nline 7\nline 8\n";

        let delta = compute_delta(base, target);
        assert!(
            delta.len() < target.len(),
            "delta ({}) should be smaller than target ({})",
            delta.len(),
            target.len()
        );
    }

    #[test]
    fn pack_v1_get_object() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("packs");

        let data1 = b"object data one";
        let data2 = b"object data two";
        let hash1 = B3Hash::digest(data1);
        let hash2 = B3Hash::digest(data2);

        let mut writer = PackWriter::new();
        writer.add(hash1, data1.to_vec());
        writer.add(hash2, data2.to_vec());
        writer.write(&pack_dir).unwrap();

        let reader = PackReader::new(&pack_dir);
        assert_eq!(reader.get_object(hash1).unwrap(), data1);
        assert_eq!(reader.get_object(hash2).unwrap(), data2);
    }

    #[test]
    fn pack_v2_write_read() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("packs");

        // Create similar objects that benefit from delta
        let base_text = b"This is a long piece of text that will serve as the base object for delta compression testing. It has enough content to make the delta worthwhile.";
        let modified_text = b"This is a long piece of text that will serve as the modified object for delta compression testing. It has enough content to make the delta worthwhile.";
        let different_text = b"Completely different content here";

        let hash1 = B3Hash::digest(base_text);
        let hash2 = B3Hash::digest(modified_text);
        let hash3 = B3Hash::digest(different_text);

        let mut writer = PackWriter::new();
        writer.add(hash1, base_text.to_vec());
        writer.add(hash2, modified_text.to_vec());
        writer.add(hash3, different_text.to_vec());
        writer.write_delta(&pack_dir).unwrap();

        let reader = PackReader::new(&pack_dir);
        assert_eq!(reader.get_object(hash1).unwrap(), base_text);
        assert_eq!(reader.get_object(hash2).unwrap(), modified_text);
        assert_eq!(reader.get_object(hash3).unwrap(), different_text);
    }

    #[test]
    fn extract_to_cas() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("packs");
        let cas_dir = dir.path().join("objects");

        let data1 = b"cas object 1";
        let data2 = b"cas object 2";
        let hash1 = B3Hash::digest(data1);
        let hash2 = B3Hash::digest(data2);

        let mut writer = PackWriter::new();
        writer.add(hash1, data1.to_vec());
        writer.add(hash2, data2.to_vec());
        writer.write(&pack_dir).unwrap();

        let cas = FileCas::new(&cas_dir).unwrap();
        let reader = PackReader::new(&pack_dir);
        let count = reader.extract_to_cas(&cas).unwrap();
        assert_eq!(count, 2);

        // Verify CAS contains the objects
        assert_eq!(cas.get(hash1).unwrap(), data1);
        assert_eq!(cas.get(hash2).unwrap(), data2);
    }

    #[test]
    fn pack_v2_extract_to_cas() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("packs");
        let cas_dir = dir.path().join("objects");

        let base = b"This is the base text that will be used for delta. It should be long enough for the algorithm to find matches.";
        let modified = b"This is the modified text that will be used for delta. It should be long enough for the algorithm to find matches.";

        let hash1 = B3Hash::digest(base);
        let hash2 = B3Hash::digest(modified);

        let mut writer = PackWriter::new();
        writer.add(hash1, base.to_vec());
        writer.add(hash2, modified.to_vec());
        writer.write_delta(&pack_dir).unwrap();

        let cas = FileCas::new(&cas_dir).unwrap();
        let reader = PackReader::new(&pack_dir);
        let count = reader.extract_to_cas(&cas).unwrap();
        assert_eq!(count, 2);

        assert_eq!(cas.get(hash1).unwrap(), base);
        assert_eq!(cas.get(hash2).unwrap(), modified);
    }

    #[test]
    fn pack_loose_objects_integration() {
        let dir = tempfile::tempdir().unwrap();
        let ivaldi_dir = dir.path().join(".ivaldi");
        fs::create_dir_all(&ivaldi_dir).unwrap();

        // Create some loose objects in the CAS
        let cas = FileCas::new(ivaldi_dir.join("objects")).unwrap();
        let data1 = b"loose object 1";
        let data2 = b"loose object 2";
        let data3 = b"loose object 3";
        let hash1 = B3Hash::digest(data1);
        let hash2 = B3Hash::digest(data2);
        let hash3 = B3Hash::digest(data3);
        cas.put(hash1, data1).unwrap();
        cas.put(hash2, data2).unwrap();
        cas.put(hash3, data3).unwrap();

        // Verify loose objects exist
        assert!(cas.has(hash1).unwrap());
        assert!(cas.has(hash2).unwrap());
        assert!(cas.has(hash3).unwrap());

        // Pack them
        let result = pack_loose_objects(&ivaldi_dir).unwrap();
        assert_eq!(result.objects_packed, 3);
        assert_eq!(result.loose_removed, 3);

        // Verify loose objects are removed
        assert!(!cas.has(hash1).unwrap());

        // Verify packed objects are readable
        let reader = PackReader::new(&ivaldi_dir.join("packs"));
        assert_eq!(reader.get_object(hash1).unwrap(), data1);
        assert_eq!(reader.get_object(hash2).unwrap(), data2);
        assert_eq!(reader.get_object(hash3).unwrap(), data3);
    }

    #[test]
    fn get_object_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("packs");
        fs::create_dir_all(&pack_dir).unwrap();

        let reader = PackReader::new(&pack_dir);
        let result = reader.get_object(B3Hash::digest(b"nonexistent"));
        assert!(result.is_err());
    }
}

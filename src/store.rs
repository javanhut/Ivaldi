//! Persistent key-value store for Ivaldi VCS, backed by `redb`.
//!
//! Provides ACID, crash-safe storage for:
//! - MMR leaves (commit history)
//! - Timeline heads (branch pointers)
//! - Seal name registry (bidirectional name ↔ hash)
//! - Butterfly metadata
//! - Generic metadata

use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::hash::B3Hash;

const LEAVES: TableDefinition<u64, &[u8]> = TableDefinition::new("leaves");
const TIMELINE_HEADS: TableDefinition<&str, u64> = TableDefinition::new("timeline_heads");
const SEAL_NAME_TO_HASH: TableDefinition<&str, &[u8]> = TableDefinition::new("seal_to_hash");
const HASH_TO_SEAL_NAME: TableDefinition<&[u8], &str> = TableDefinition::new("hash_to_seal");
const BUTTERFLY_DATA: TableDefinition<&str, &[u8]> = TableDefinition::new("butterflies");
const BUTTERFLY_CHILDREN: TableDefinition<&str, &str> = TableDefinition::new("bf_children");
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");

pub const MMR_SIZE_KEY: &str = "mmr.size";
pub const MMR_ROOT_KEY: &str = "mmr.root";

/// Persistent store backed by redb.
pub struct Store {
    db: Database,
}

/// One leaf in a multi-leaf atomic append (see [`Store::commit_leaves_atomic`]).
pub struct BatchLeaf<'a> {
    pub idx: u64,
    pub canonical: &'a [u8],
    pub seal_name: &'a str,
    pub seal_hash: B3Hash,
}

/// Unified error type — stringifies redb's varied error types.
#[derive(Debug, thiserror::Error)]
#[error("store: {0}")]
pub struct StoreError(String);

macro_rules! impl_from_redb {
    ($($t:ty),*) => {
        $(impl From<$t> for StoreError {
            fn from(e: $t) -> Self { StoreError(e.to_string()) }
        })*
    };
}
impl_from_redb!(
    redb::DatabaseError,
    redb::TransactionError,
    redb::TableError,
    redb::CommitError,
    redb::StorageError
);

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Database::create(path).map_err(|e| match e {
            redb::DatabaseError::DatabaseAlreadyOpen => StoreError(
                "repository store is in use by another ivaldi process; \
                 retry when it finishes"
                    .into(),
            ),
            other => StoreError::from(other),
        })?;
        let w = db.begin_write()?;
        {
            let _ = w.open_table(LEAVES)?;
        }
        {
            let _ = w.open_table(TIMELINE_HEADS)?;
        }
        {
            let _ = w.open_table(SEAL_NAME_TO_HASH)?;
        }
        {
            let _ = w.open_table(HASH_TO_SEAL_NAME)?;
        }
        {
            let _ = w.open_table(BUTTERFLY_DATA)?;
        }
        {
            let _ = w.open_table(BUTTERFLY_CHILDREN)?;
        }
        {
            let _ = w.open_table(META)?;
        }
        w.commit()?;
        Ok(Self { db })
    }

    // -- Leaves --

    pub fn put_leaf(&self, idx: u64, data: &[u8]) -> Result<(), StoreError> {
        let w = self.db.begin_write()?;
        {
            let mut leaves = w.open_table(LEAVES)?;
            if leaves.get(idx)?.is_some() {
                return Err(StoreError(format!(
                    "refusing to overwrite append-only MMR leaf {}",
                    idx
                )));
            }
            leaves.insert(idx, data)?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn get_leaf(&self, idx: u64) -> Result<Option<Vec<u8>>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(LEAVES)?;
        Ok(t.get(idx)?.map(|v| v.value().to_vec()))
    }

    pub fn leaf_count(&self) -> Result<u64, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(LEAVES)?;
        let mut n = 0u64;
        for _ in t.iter()? {
            n += 1;
        }
        Ok(n)
    }

    pub fn all_leaf_indices(&self) -> Result<Vec<u64>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(LEAVES)?;
        let mut v = Vec::new();
        for e in t.iter()? {
            let (k, _) = e?;
            v.push(k.value());
        }
        Ok(v)
    }

    // -- Timeline heads --

    pub fn set_timeline_head(&self, name: &str, idx: u64) -> Result<(), StoreError> {
        let w = self.db.begin_write()?;
        {
            w.open_table(TIMELINE_HEADS)?.insert(name, idx)?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn get_timeline_head(&self, name: &str) -> Result<Option<u64>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(TIMELINE_HEADS)?;
        Ok(t.get(name)?.map(|v| v.value()))
    }

    /// Move a timeline head to a new name in one transaction. A crash can
    /// never leave both names (or neither name) holding the head.
    pub fn rename_timeline_head(&self, old_name: &str, new_name: &str) -> Result<(), StoreError> {
        let w = self.db.begin_write()?;
        {
            let mut heads = w.open_table(TIMELINE_HEADS)?;
            if heads.get(new_name)?.is_some() {
                return Err(StoreError(format!("timeline {new_name:?} already exists")));
            }
            let head = heads
                .remove(old_name)?
                .map(|v| v.value())
                .ok_or_else(|| StoreError(format!("timeline {old_name:?} not found")))?;
            heads.insert(new_name, head)?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn remove_timeline_head(&self, name: &str) -> Result<bool, StoreError> {
        let w = self.db.begin_write()?;
        let removed;
        {
            removed = w.open_table(TIMELINE_HEADS)?.remove(name)?.is_some();
        }
        w.commit()?;
        Ok(removed)
    }

    pub fn list_timeline_heads(&self) -> Result<Vec<(String, u64)>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(TIMELINE_HEADS)?;
        let mut result = Vec::new();
        for e in t.iter()? {
            let (k, v) = e?;
            result.push((k.value().to_string(), v.value()));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(result)
    }

    // -- Seal names --

    pub fn put_seal_name(&self, name: &str, hash: B3Hash) -> Result<(), StoreError> {
        let w = self.db.begin_write()?;
        {
            w.open_table(SEAL_NAME_TO_HASH)?
                .insert(name, hash.as_bytes().as_slice())?;
            w.open_table(HASH_TO_SEAL_NAME)?
                .insert(hash.as_bytes().as_slice(), name)?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn get_hash_by_seal_name(&self, name: &str) -> Result<Option<B3Hash>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(SEAL_NAME_TO_HASH)?;
        Ok(t.get(name)?.and_then(|v| B3Hash::from_slice(v.value())))
    }

    pub fn get_seal_name_by_hash(&self, hash: B3Hash) -> Result<Option<String>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(HASH_TO_SEAL_NAME)?;
        Ok(t.get(hash.as_bytes().as_slice())?
            .map(|v| v.value().to_string()))
    }

    pub fn find_seal_names_by_prefix(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(SEAL_NAME_TO_HASH)?;
        let mut m = Vec::new();
        for e in t.iter()? {
            let (k, _) = e?;
            if k.value().starts_with(prefix) {
                m.push(k.value().to_string());
            }
        }
        Ok(m)
    }

    /// List the reverse seal registry (`leaf hash -> seal name`) for integrity
    /// verification. Invalid hash keys are reported rather than skipped.
    pub fn list_seal_hash_mappings(&self) -> Result<Vec<(B3Hash, String)>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(HASH_TO_SEAL_NAME)?;
        let mut mappings = Vec::new();
        for entry in t.iter()? {
            let (hash, name) = entry?;
            let hash = B3Hash::from_slice(hash.value()).ok_or_else(|| {
                StoreError("invalid hash key in reverse seal registry".to_string())
            })?;
            mappings.push((hash, name.value().to_string()));
        }
        mappings.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(mappings)
    }

    // -- Butterfly --

    pub fn put_butterfly(&self, name: &str, data: &[u8]) -> Result<(), StoreError> {
        let w = self.db.begin_write()?;
        {
            w.open_table(BUTTERFLY_DATA)?.insert(name, data)?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn get_butterfly(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(BUTTERFLY_DATA)?;
        Ok(t.get(name)?.map(|v| v.value().to_vec()))
    }

    pub fn delete_butterfly(&self, name: &str) -> Result<bool, StoreError> {
        let w = self.db.begin_write()?;
        let removed;
        {
            removed = w.open_table(BUTTERFLY_DATA)?.remove(name)?.is_some();
        }
        w.commit()?;
        Ok(removed)
    }

    pub fn list_butterflies(&self) -> Result<Vec<String>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(BUTTERFLY_DATA)?;
        let mut v = Vec::new();
        for e in t.iter()? {
            let (k, _) = e?;
            v.push(k.value().to_string());
        }
        Ok(v)
    }

    pub fn set_butterfly_children(
        &self,
        parent: &str,
        children: &[String],
    ) -> Result<(), StoreError> {
        let w = self.db.begin_write()?;
        {
            let val = children.join(",");
            w.open_table(BUTTERFLY_CHILDREN)?
                .insert(parent, val.as_str())?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn get_butterfly_children(&self, parent: &str) -> Result<Vec<String>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(BUTTERFLY_CHILDREN)?;
        Ok(t.get(parent)?
            .map(|v| {
                let s = v.value();
                if s.is_empty() {
                    Vec::new()
                } else {
                    s.split(',').map(|x| x.to_string()).collect()
                }
            })
            .unwrap_or_default())
    }

    // -- Meta --

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let w = self.db.begin_write()?;
        {
            w.open_table(META)?.insert(key, value)?;
        }
        w.commit()?;
        Ok(())
    }

    /// Persist a single commit's leaf, timeline head, seal mapping, and MMR
    /// size/root checkpoint in one redb write transaction (one fsync).
    /// Existing leaf indices cannot be overwritten.
    #[allow(clippy::too_many_arguments)]
    pub fn commit_leaf_atomic(
        &self,
        idx: u64,
        canonical: &[u8],
        timeline: &str,
        timeline_head: u64,
        seal_name: &str,
        seal_hash: B3Hash,
        mmr_size: u64,
        mmr_root: B3Hash,
    ) -> Result<(), StoreError> {
        if mmr_size != idx + 1 {
            return Err(StoreError(format!(
                "invalid MMR append: leaf index {} does not produce size {}",
                idx, mmr_size
            )));
        }

        let w = self.db.begin_write()?;
        {
            let mut leaves = w.open_table(LEAVES)?;
            if leaves.get(idx)?.is_some() {
                return Err(StoreError(format!(
                    "refusing to overwrite append-only MMR leaf {}",
                    idx
                )));
            }
            leaves.insert(idx, canonical)?;
            w.open_table(TIMELINE_HEADS)?
                .insert(timeline, timeline_head)?;
            w.open_table(SEAL_NAME_TO_HASH)?
                .insert(seal_name, seal_hash.as_bytes().as_slice())?;
            w.open_table(HASH_TO_SEAL_NAME)?
                .insert(seal_hash.as_bytes().as_slice(), seal_name)?;
            let mut meta = w.open_table(META)?;
            if let Some(previous_size) = meta.get(MMR_SIZE_KEY)? {
                let previous_size = previous_size.value().parse::<u64>().map_err(|_| {
                    StoreError("stored MMR size checkpoint is not a valid integer".into())
                })?;
                if previous_size != idx {
                    return Err(StoreError(format!(
                        "non-contiguous MMR append: stored size is {}, new leaf index is {}",
                        previous_size, idx
                    )));
                }
            } else if idx != 0 {
                return Err(StoreError(format!(
                    "missing MMR size checkpoint before appending leaf {}",
                    idx
                )));
            }
            meta.insert(MMR_SIZE_KEY, mmr_size.to_string().as_str())?;
            meta.insert(MMR_ROOT_KEY, mmr_root.to_hex().as_str())?;
        }
        crate::failpoint::fail_point("store.commit_leaf.before_commit");
        w.commit()?;
        crate::failpoint::fail_point("store.commit_leaf.after_commit");
        Ok(())
    }

    /// Persist a batch of contiguous new leaves, their seal mappings, the
    /// final timeline head, and the MMR size/root checkpoint in ONE redb
    /// write transaction. Used by multi-seal rewrites (`weld`) so a crash
    /// can never expose a partially replayed chain: either every leaf in
    /// the batch is visible with the head on the last one, or none are.
    pub fn commit_leaves_atomic(
        &self,
        entries: &[BatchLeaf<'_>],
        timeline: &str,
        timeline_head: u64,
        mmr_size: u64,
        mmr_root: B3Hash,
    ) -> Result<(), StoreError> {
        let first = entries
            .first()
            .ok_or_else(|| StoreError("empty leaf batch".into()))?;
        let last_idx = entries.last().unwrap().idx;
        if mmr_size != last_idx + 1 {
            return Err(StoreError(format!(
                "invalid MMR batch append: last leaf index {} does not produce size {}",
                last_idx, mmr_size
            )));
        }
        if timeline_head != last_idx {
            return Err(StoreError(format!(
                "batch head {} is not the batch's last leaf {}",
                timeline_head, last_idx
            )));
        }

        let w = self.db.begin_write()?;
        {
            let mut leaves = w.open_table(LEAVES)?;
            for (expected, entry) in (first.idx..).zip(entries.iter()) {
                if entry.idx != expected {
                    return Err(StoreError(format!(
                        "non-contiguous leaf batch: expected index {}, got {}",
                        expected, entry.idx
                    )));
                }
                if leaves.get(entry.idx)?.is_some() {
                    return Err(StoreError(format!(
                        "refusing to overwrite append-only MMR leaf {}",
                        entry.idx
                    )));
                }
                leaves.insert(entry.idx, entry.canonical)?;
            }
            w.open_table(TIMELINE_HEADS)?
                .insert(timeline, timeline_head)?;
            {
                let mut n2h = w.open_table(SEAL_NAME_TO_HASH)?;
                let mut h2n = w.open_table(HASH_TO_SEAL_NAME)?;
                for entry in entries {
                    n2h.insert(entry.seal_name, entry.seal_hash.as_bytes().as_slice())?;
                    h2n.insert(entry.seal_hash.as_bytes().as_slice(), entry.seal_name)?;
                }
            }
            let mut meta = w.open_table(META)?;
            if let Some(previous_size) = meta.get(MMR_SIZE_KEY)? {
                let previous_size = previous_size.value().parse::<u64>().map_err(|_| {
                    StoreError("stored MMR size checkpoint is not a valid integer".into())
                })?;
                if previous_size != first.idx {
                    return Err(StoreError(format!(
                        "non-contiguous MMR append: stored size is {}, batch starts at index {}",
                        previous_size, first.idx
                    )));
                }
            } else if first.idx != 0 {
                return Err(StoreError(format!(
                    "missing MMR size checkpoint before appending leaf {}",
                    first.idx
                )));
            }
            meta.insert(MMR_SIZE_KEY, mmr_size.to_string().as_str())?;
            meta.insert(MMR_ROOT_KEY, mmr_root.to_hex().as_str())?;
        }
        crate::failpoint::fail_point("store.commit_leaves.before_commit");
        w.commit()?;
        crate::failpoint::fail_point("store.commit_leaves.after_commit");
        Ok(())
    }

    /// Establish or repair both MMR checkpoint fields in one transaction.
    /// Used when opening legacy repositories that predate root checkpoints.
    pub fn set_mmr_checkpoint(&self, size: u64, root: B3Hash) -> Result<(), StoreError> {
        let w = self.db.begin_write()?;
        {
            let mut meta = w.open_table(META)?;
            meta.insert(MMR_SIZE_KEY, size.to_string().as_str())?;
            meta.insert(MMR_ROOT_KEY, root.to_hex().as_str())?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn remove_meta(&self, key: &str) -> Result<bool, StoreError> {
        let w = self.db.begin_write()?;
        let removed;
        {
            removed = w.open_table(META)?.remove(key)?.is_some();
        }
        w.commit()?;
        Ok(removed)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>, StoreError> {
        let r = self.db.begin_read()?;
        let t = r.open_table(META)?;
        Ok(t.get(key)?.map(|v| v.value().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn leaf_put_get() {
        let (_d, s) = setup();
        s.put_leaf(0, b"data").unwrap();
        assert_eq!(s.get_leaf(0).unwrap().unwrap(), b"data");
        assert!(s.get_leaf(99).unwrap().is_none());
    }

    #[test]
    fn leaf_indices_are_append_only() {
        let (_d, s) = setup();
        s.put_leaf(0, b"original").unwrap();
        let error = s.put_leaf(0, b"replacement").unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"));
        assert_eq!(s.get_leaf(0).unwrap().unwrap(), b"original");
    }

    #[test]
    fn leaf_count_and_indices() {
        let (_d, s) = setup();
        s.put_leaf(0, b"a").unwrap();
        s.put_leaf(1, b"b").unwrap();
        s.put_leaf(5, b"c").unwrap();
        assert_eq!(s.leaf_count().unwrap(), 3);
        assert_eq!(s.all_leaf_indices().unwrap(), vec![0, 1, 5]);
    }

    #[test]
    fn timeline_head_crud() {
        let (_d, s) = setup();
        s.set_timeline_head("main", 42).unwrap();
        assert_eq!(s.get_timeline_head("main").unwrap(), Some(42));
        s.set_timeline_head("main", 99).unwrap();
        assert_eq!(s.get_timeline_head("main").unwrap(), Some(99));
        assert!(s.remove_timeline_head("main").unwrap());
        assert!(s.get_timeline_head("main").unwrap().is_none());
    }

    #[test]
    fn timeline_list_sorted() {
        let (_d, s) = setup();
        s.set_timeline_head("zeta", 0).unwrap();
        s.set_timeline_head("alpha", 1).unwrap();
        let list = s.list_timeline_heads().unwrap();
        assert_eq!(list[0].0, "alpha");
        assert_eq!(list[1].0, "zeta");
    }

    #[test]
    fn seal_name_bidirectional() {
        let (_d, s) = setup();
        let h = B3Hash::digest(b"x");
        s.put_seal_name("swift-eagle", h).unwrap();
        assert_eq!(s.get_hash_by_seal_name("swift-eagle").unwrap(), Some(h));
        assert_eq!(
            s.get_seal_name_by_hash(h).unwrap(),
            Some("swift-eagle".into())
        );
    }

    #[test]
    fn seal_name_prefix() {
        let (_d, s) = setup();
        s.put_seal_name("swift-eagle", B3Hash::digest(b"1"))
            .unwrap();
        s.put_seal_name("swift-wolf", B3Hash::digest(b"2")).unwrap();
        s.put_seal_name("bold-hawk", B3Hash::digest(b"3")).unwrap();
        assert_eq!(s.find_seal_names_by_prefix("swift").unwrap().len(), 2);
    }

    #[test]
    fn reverse_seal_mappings_are_listed() {
        let (_d, s) = setup();
        let first = B3Hash::digest(b"first");
        let second = B3Hash::digest(b"second");
        s.put_seal_name("zeta", second).unwrap();
        s.put_seal_name("alpha", first).unwrap();

        assert_eq!(
            s.list_seal_hash_mappings().unwrap(),
            vec![(first, "alpha".into()), (second, "zeta".into())]
        );
    }

    #[test]
    fn butterfly_crud() {
        let (_d, s) = setup();
        s.put_butterfly("exp", b"json").unwrap();
        assert_eq!(s.get_butterfly("exp").unwrap().unwrap(), b"json");
        assert!(s.delete_butterfly("exp").unwrap());
        assert!(s.get_butterfly("exp").unwrap().is_none());
    }

    #[test]
    fn butterfly_children() {
        let (_d, s) = setup();
        s.set_butterfly_children("main", &["a".into(), "b".into()])
            .unwrap();
        assert_eq!(s.get_butterfly_children("main").unwrap(), vec!["a", "b"]);
        assert!(s.get_butterfly_children("empty").unwrap().is_empty());
    }

    #[test]
    fn meta() {
        let (_d, s) = setup();
        s.set_meta("k", "v").unwrap();
        assert_eq!(s.get_meta("k").unwrap(), Some("v".into()));
        assert!(s.remove_meta("k").unwrap());
        assert_eq!(s.get_meta("k").unwrap(), None);
    }

    #[test]
    fn batch_commit_is_atomic_and_validated() {
        let (_d, s) = setup();
        let h = |b: &[u8]| B3Hash::digest(b);

        // Establish the checkpoint with a first ordinary commit.
        s.commit_leaf_atomic(0, b"leaf0", "main", 0, "alpha", h(b"0"), 1, h(b"r0"))
            .unwrap();

        // A valid two-leaf batch: both leaves land, head on the last one.
        let batch = [
            BatchLeaf {
                idx: 1,
                canonical: b"leaf1",
                seal_name: "beta",
                seal_hash: h(b"1"),
            },
            BatchLeaf {
                idx: 2,
                canonical: b"leaf2",
                seal_name: "gamma",
                seal_hash: h(b"2"),
            },
        ];
        s.commit_leaves_atomic(&batch, "main", 2, 3, h(b"r2"))
            .unwrap();
        assert_eq!(s.get_leaf(1).unwrap().unwrap(), b"leaf1");
        assert_eq!(s.get_leaf(2).unwrap().unwrap(), b"leaf2");
        assert_eq!(s.get_timeline_head("main").unwrap(), Some(2));
        assert_eq!(s.get_hash_by_seal_name("gamma").unwrap(), Some(h(b"2")));

        // Refusals: empty batch, gap against the checkpoint, head not on the
        // last leaf, overwrite of an existing leaf. None may leave residue.
        assert!(s.commit_leaves_atomic(&[], "main", 0, 0, h(b"r")).is_err());
        let gap = [BatchLeaf {
            idx: 5,
            canonical: b"leaf5",
            seal_name: "delta",
            seal_hash: h(b"5"),
        }];
        assert!(s.commit_leaves_atomic(&gap, "main", 5, 6, h(b"r")).is_err());
        let overwrite = [BatchLeaf {
            idx: 2,
            canonical: b"evil",
            seal_name: "evil",
            seal_hash: h(b"e"),
        }];
        assert!(
            s.commit_leaves_atomic(&overwrite, "main", 2, 3, h(b"r"))
                .is_err()
        );
        let wrong_head = [BatchLeaf {
            idx: 3,
            canonical: b"leaf3",
            seal_name: "eps",
            seal_hash: h(b"3"),
        }];
        assert!(
            s.commit_leaves_atomic(&wrong_head, "main", 1, 4, h(b"r"))
                .is_err()
        );
        assert_eq!(s.get_leaf(2).unwrap().unwrap(), b"leaf2");
        assert_eq!(s.get_timeline_head("main").unwrap(), Some(2));
        assert_eq!(s.get_meta(MMR_SIZE_KEY).unwrap(), Some("3".into()));
    }

    #[test]
    fn persistence() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.db");
        {
            let s = Store::open(&p).unwrap();
            s.put_leaf(0, b"persist").unwrap();
            s.set_timeline_head("main", 0).unwrap();
        }
        {
            let s = Store::open(&p).unwrap();
            assert_eq!(s.get_leaf(0).unwrap().unwrap(), b"persist");
            assert_eq!(s.get_timeline_head("main").unwrap(), Some(0));
        }
    }

    #[test]
    fn many_leaves() {
        let (_d, s) = setup();
        for i in 0..500u64 {
            s.put_leaf(i, format!("l{}", i).as_bytes()).unwrap();
        }
        assert_eq!(s.leaf_count().unwrap(), 500);
    }
}

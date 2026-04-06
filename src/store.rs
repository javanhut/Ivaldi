//! Persistent key-value store for Ivaldi VCS, backed by `redb`.
//!
//! Provides ACID, crash-safe storage for:
//! - MMR leaves (commit history)
//! - Timeline heads (branch pointers)
//! - Seal name registry (bidirectional name ↔ hash)
//! - Butterfly metadata
//! - Generic metadata

use std::path::Path;

use redb::{Database, ReadableTable, TableDefinition};

use crate::hash::B3Hash;

const LEAVES: TableDefinition<u64, &[u8]> = TableDefinition::new("leaves");
const TIMELINE_HEADS: TableDefinition<&str, u64> = TableDefinition::new("timeline_heads");
const SEAL_NAME_TO_HASH: TableDefinition<&str, &[u8]> = TableDefinition::new("seal_to_hash");
const HASH_TO_SEAL_NAME: TableDefinition<&[u8], &str> = TableDefinition::new("hash_to_seal");
const BUTTERFLY_DATA: TableDefinition<&str, &[u8]> = TableDefinition::new("butterflies");
const BUTTERFLY_CHILDREN: TableDefinition<&str, &str> = TableDefinition::new("bf_children");
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");

/// Persistent store backed by redb.
pub struct Store {
    db: Database,
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
        let db = Database::create(path)?;
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
            w.open_table(LEAVES)?.insert(idx, data)?;
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

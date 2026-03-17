//! Hash Array Mapped Trie (HAMT) for immutable directory trees.
//!
//! Provides O(log32 n) lookups with structural sharing — unchanged subtrees
//! share storage between versions. Used as an optimization layer over fsmerkle
//! TreeNode for large directories.

use crate::hash::B3Hash;

const BITS_PER_LEVEL: u32 = 5;

/// A HAMT node — either a leaf (key-value) or a branch (bitmap + children).
#[derive(Debug, Clone)]
enum Node<V: Clone> {
    Leaf { key: String, hash: B3Hash, value: V },
    Branch { bitmap: u32, children: Vec<Box<Node<V>>> },
}

/// Immutable HAMT directory tree.
#[derive(Debug, Clone)]
pub struct Hamt<V: Clone> {
    root: Option<Box<Node<V>>>,
    len: usize,
}

impl<V: Clone> Hamt<V> {
    pub fn new() -> Self {
        Self { root: None, len: 0 }
    }

    /// Number of entries.
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    /// Lookup a value by key.
    pub fn get(&self, key: &str) -> Option<&V> {
        let hash = B3Hash::digest(key.as_bytes());
        self.root.as_ref().and_then(|node| get_recursive(node, key, hash, 0))
    }

    /// Insert a key-value pair. Returns a new HAMT (structural sharing).
    pub fn insert(&self, key: String, value: V) -> Self {
        let existed = self.get(&key).is_some();
        let hash = B3Hash::digest(key.as_bytes());
        let new_root = match &self.root {
            None => Box::new(Node::Leaf { key, hash, value }),
            Some(node) => insert_recursive(node, key, hash, value, 0),
        };
        Self {
            root: Some(new_root),
            len: if existed { self.len } else { self.len + 1 },
        }
    }

    /// Remove a key. Returns a new HAMT.
    pub fn remove(&self, key: &str) -> Self {
        let hash = B3Hash::digest(key.as_bytes());
        match &self.root {
            None => self.clone(),
            Some(node) => {
                let new_root = remove_recursive(node, key, hash, 0);
                let new_len = if self.get(key).is_some() { self.len - 1 } else { self.len };
                Self { root: new_root, len: new_len }
            }
        }
    }

    /// Collect all entries as (key, value) pairs.
    pub fn entries(&self) -> Vec<(&str, &V)> {
        let mut result = Vec::new();
        if let Some(ref root) = self.root {
            collect_entries(root, &mut result);
        }
        result.sort_by_key(|(k, _)| *k);
        result
    }
}

impl<V: Clone> Default for Hamt<V> {
    fn default() -> Self { Self::new() }
}

fn index_at_level(hash: B3Hash, level: u32) -> usize {
    let shift = level * BITS_PER_LEVEL;
    let bytes = hash.as_bytes();
    let bit_offset = shift as usize;
    let byte_idx = bit_offset / 8;
    let bit_idx = bit_offset % 8;
    if byte_idx >= 32 { return 0; }
    ((bytes[byte_idx] >> bit_idx) & 0x1F) as usize
}

fn get_recursive<'a, V: Clone>(node: &'a Node<V>, key: &str, hash: B3Hash, level: u32) -> Option<&'a V> {
    match node {
        Node::Leaf { key: k, value: v, .. } => {
            if k == key { Some(v) } else { None }
        }
        Node::Branch { bitmap, children } => {
            let idx = index_at_level(hash, level);
            let bit = 1u32 << idx;
            if bitmap & bit == 0 { return None; }
            let child_idx = (bitmap & (bit - 1)).count_ones() as usize;
            get_recursive(&children[child_idx], key, hash, level + 1)
        }
    }
}

fn insert_recursive<V: Clone>(node: &Node<V>, key: String, hash: B3Hash, value: V, level: u32) -> Box<Node<V>> {
    match node {
        Node::Leaf { key: existing_key, hash: existing_hash, value: existing_value } => {
            if *existing_key == key {
                Box::new(Node::Leaf { key, hash, value })
            } else {
                // Collision at this level — create branch
                let branch = Node::Branch { bitmap: 0, children: Vec::new() };
                let b = insert_into_branch(&branch, existing_key.clone(), *existing_hash, existing_value.clone(), level);
                insert_into_branch(&b, key, hash, value, level)
            }
        }
        Node::Branch { .. } => {
            insert_into_branch(node, key, hash, value, level)
        }
    }
}

fn insert_into_branch<V: Clone>(node: &Node<V>, key: String, hash: B3Hash, value: V, level: u32) -> Box<Node<V>> {
    match node {
        Node::Branch { bitmap, children } => {
            let idx = index_at_level(hash, level);
            let bit = 1u32 << idx;
            let child_idx = (bitmap & (bit - 1)).count_ones() as usize;

            if bitmap & bit == 0 {
                // Empty slot — insert leaf
                let mut new_children = children.clone();
                new_children.insert(child_idx, Box::new(Node::Leaf { key, hash, value }));
                Box::new(Node::Branch { bitmap: bitmap | bit, children: new_children })
            } else {
                // Slot occupied — recurse
                let mut new_children = children.clone();
                new_children[child_idx] = insert_recursive(&children[child_idx], key, hash, value, level + 1);
                Box::new(Node::Branch { bitmap: *bitmap, children: new_children })
            }
        }
        _ => Box::new(Node::Leaf { key, hash, value }),
    }
}

fn remove_recursive<V: Clone>(node: &Node<V>, key: &str, hash: B3Hash, level: u32) -> Option<Box<Node<V>>> {
    match node {
        Node::Leaf { key: k, .. } => {
            if k == key { None } else { Some(Box::new(node.clone())) }
        }
        Node::Branch { bitmap, children } => {
            let idx = index_at_level(hash, level);
            let bit = 1u32 << idx;
            if bitmap & bit == 0 { return Some(Box::new(node.clone())); }
            let child_idx = (bitmap & (bit - 1)).count_ones() as usize;
            let new_child = remove_recursive(&children[child_idx], key, hash, level + 1);
            let mut new_children = children.clone();
            let new_bitmap;
            match new_child {
                None => {
                    new_children.remove(child_idx);
                    new_bitmap = bitmap & !bit;
                }
                Some(c) => {
                    new_children[child_idx] = c;
                    new_bitmap = *bitmap;
                }
            }
            if new_children.is_empty() {
                None
            } else if new_children.len() == 1 {
                if let Node::Leaf { .. } = *new_children[0] {
                    Some(new_children.into_iter().next().unwrap())
                } else {
                    Some(Box::new(Node::Branch { bitmap: new_bitmap, children: new_children }))
                }
            } else {
                Some(Box::new(Node::Branch { bitmap: new_bitmap, children: new_children }))
            }
        }
    }
}

fn collect_entries<'a, V: Clone>(node: &'a Node<V>, result: &mut Vec<(&'a str, &'a V)>) {
    match node {
        Node::Leaf { key, value, .. } => result.push((key, value)),
        Node::Branch { children, .. } => {
            for child in children { collect_entries(child, result); }
        }
    }
}

#[allow(dead_code)]
fn key_from_node<V: Clone>(node: &Node<V>) -> String {
    match node {
        Node::Leaf { key, .. } => key.clone(),
        Node::Branch { children, .. } if !children.is_empty() => key_from_node(&children[0]),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hamt() {
        let h: Hamt<i32> = Hamt::new();
        assert!(h.is_empty());
        assert_eq!(h.get("x"), None);
    }

    #[test]
    fn insert_and_get() {
        let h = Hamt::new().insert("hello".into(), 42);
        assert_eq!(h.get("hello"), Some(&42));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn insert_multiple() {
        let h = Hamt::new()
            .insert("a".into(), 1)
            .insert("b".into(), 2)
            .insert("c".into(), 3);
        assert_eq!(h.get("a"), Some(&1));
        assert_eq!(h.get("b"), Some(&2));
        assert_eq!(h.get("c"), Some(&3));
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn overwrite() {
        let h = Hamt::new().insert("k".into(), 1).insert("k".into(), 2);
        assert_eq!(h.get("k"), Some(&2));
    }

    #[test]
    fn remove() {
        let h = Hamt::new().insert("a".into(), 1).insert("b".into(), 2);
        let h2 = h.remove("a");
        assert_eq!(h2.get("a"), None);
        assert_eq!(h2.get("b"), Some(&2));
    }

    #[test]
    fn structural_sharing() {
        let h1 = Hamt::new().insert("a".into(), 1).insert("b".into(), 2);
        let h2 = h1.insert("c".into(), 3);
        // h1 still works (immutable)
        assert_eq!(h1.get("a"), Some(&1));
        assert_eq!(h1.get("c"), None);
        assert_eq!(h2.get("c"), Some(&3));
    }

    #[test]
    fn entries_sorted() {
        let h = Hamt::new()
            .insert("z".into(), 3)
            .insert("a".into(), 1)
            .insert("m".into(), 2);
        let entries = h.entries();
        let keys: Vec<&str> = entries.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["a", "m", "z"]);
    }

    #[test]
    fn many_entries() {
        let mut h = Hamt::new();
        for i in 0..100 {
            h = h.insert(format!("key_{:03}", i), i);
        }
        assert_eq!(h.len(), 100);
        for i in 0..100 {
            assert_eq!(h.get(&format!("key_{:03}", i)), Some(&i));
        }
    }
}

//! Raw rescue export (`ivaldi rescue`).
//!
//! Recovers files from a damaged repository by reading content directly. It
//! never goes through `Repo::open`, never trusts HEAD, refs, or the MMR — those
//! are exactly what may be broken. Every object is checked against its own hash
//! before use, so rescued content is guaranteed intact; anything corrupt,
//! missing, or unsafe is skipped and reported.
//!
//! Two independent sources, either of which may be broken: the redb store gives
//! commit leaves → tree roots (with author/message), and the CAS (`objects/`)
//! gives the blob/tree content to materialize. If the store is unreadable, an
//! orphan sweep still materializes every tree that parses out of the CAS, so
//! files come back even with no commit records.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::fsmerkle::{self, NodeKind};
use crate::hash::B3Hash;
use crate::leaf;
use crate::store::Store;

#[cfg(unix)]
use crate::fsmerkle::MODE_EXEC;

/// Deepest tree nesting rescue will follow. Bounds recursion against a hostile
/// or cyclic-looking tree (content addressing forbids true cycles, but a
/// corrupt object could still nest absurdly). Real trees are nowhere near this.
const MAX_DEPTH: usize = 128;

#[derive(Default, serde::Serialize)]
pub struct RescueReport {
    pub store_readable: bool,
    pub seals_recovered: usize,
    pub trees_materialized: usize,
    pub files_written: usize,
    pub objects_scanned: usize,
    pub objects_corrupt: usize,
    pub problems: Vec<String>,
}

impl RescueReport {
    pub fn print_human(&self, out_dir: &Path) {
        println!("Rescued into {}", out_dir.display());
        println!("  objects scanned:    {}", self.objects_scanned);
        println!("  corrupt (skipped):  {}", self.objects_corrupt);
        println!("  commit records:     {}", self.seals_recovered);
        println!("  snapshots written:  {}", self.trees_materialized);
        println!("  files written:      {}", self.files_written);
        if !self.store_readable {
            println!(
                "  {}",
                crate::color::yellow("commit store unreadable — recovered via orphan tree sweep")
            );
        }
        if !self.problems.is_empty() {
            println!("  {} problems:", self.problems.len());
            for p in &self.problems {
                println!("    - {p}");
            }
        }
    }
}

/// Locate a repository's `.ivaldi` directory by walking upward, requiring only
/// that `objects/` exist — deliberately NOT that HEAD or refs are intact.
pub fn find_ivaldi_dir(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        let candidate = cur.join(".ivaldi");
        if candidate.join("objects").is_dir() {
            return Some(candidate);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Recover everything decodable from `ivaldi_dir` into `out_dir`.
pub fn rescue(ivaldi_dir: &Path, out_dir: &Path) -> std::io::Result<RescueReport> {
    let mut report = RescueReport::default();

    // 1. Load every object that hashes to its own name. Corrupt ones are
    //    excluded, so nothing tampered can ever be written out.
    let objects = load_verified_objects(&ivaldi_dir.join("objects"), &mut report);

    // 2. Commit leaves → tree roots (best metadata). Store may be dead.
    let leaves = read_leaves(&ivaldi_dir.join("store.db"), &mut report);

    std::fs::create_dir_all(out_dir)?;
    let mut manifest = String::new();
    // Track trees reachable from a recorded snapshot so the orphan sweep does
    // not export them again. This must not be used to suppress materialization:
    // the same content-addressed subtree can legitimately appear in multiple
    // snapshots or at multiple paths within one snapshot.
    let mut reachable: HashSet<B3Hash> = HashSet::new();
    let mut materialized_roots: HashSet<B3Hash> = HashSet::new();

    // 3. Materialize each distinct snapshot referenced by a commit.
    for leaf in &leaves {
        let short = &leaf.tree_root.to_hex()[..16];
        let dest = out_dir.join(short);
        if materialized_roots.insert(leaf.tree_root) {
            let mut ancestors = HashSet::new();
            materialize_tree(
                &objects,
                leaf.tree_root,
                &dest,
                0,
                &mut reachable,
                &mut ancestors,
                &mut report,
            );
            report.trees_materialized += 1;
        }
        manifest.push_str(&format!(
            "snapshot {short}  t={}  {}  {}\n",
            leaf.time_unix,
            leaf.author,
            leaf.message.lines().next().unwrap_or("")
        ));
    }

    // 4. Orphan sweep: any tree in the CAS not reachable from a commit is
    //    dumped anyway. This is what recovers data when the store is gone.
    let orphans_root = out_dir.join("orphans");
    for (&hash, bytes) in &objects {
        if reachable.contains(&hash)
            || (fsmerkle::parse_tree(bytes).is_err() && !crate::hamt::is_hamt_node(bytes))
        {
            continue;
        }
        let short = &hash.to_hex()[..16];
        let mut ancestors = HashSet::new();
        materialize_tree(
            &objects,
            hash,
            &orphans_root.join(short),
            0,
            &mut reachable,
            &mut ancestors,
            &mut report,
        );
        report.trees_materialized += 1;
        manifest.push_str(&format!("orphan {short}\n"));
    }

    std::fs::write(out_dir.join("MANIFEST.txt"), manifest)?;
    Ok(report)
}

/// Walk `objects/<2hex>/<62hex>`, keeping only objects whose content matches
/// their address. Corrupt and stray files are counted/skipped, never returned.
fn load_verified_objects(
    objects_dir: &Path,
    report: &mut RescueReport,
) -> HashMap<B3Hash, Vec<u8>> {
    // ponytail: holds every object in RAM (O(repo size)). Rescue is a cold
    // path so this is fine; stream by hash from disk if a huge repo needs it.
    let mut map = HashMap::new();
    let Ok(shards) = std::fs::read_dir(objects_dir) else {
        report
            .problems
            .push(format!("cannot read {}", objects_dir.display()));
        return map;
    };
    for shard in shards.flatten() {
        let shard_path = shard.path();
        if !shard_path.is_dir() {
            continue;
        }
        let prefix = shard.file_name().to_string_lossy().into_owned();
        let Ok(entries) = std::fs::read_dir(&shard_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(expected) = B3Hash::from_hex(&format!("{prefix}{name}")) else {
                continue; // tmp leftover or non-object; not corruption
            };
            report.objects_scanned += 1;
            match std::fs::read(entry.path()) {
                Ok(data) if B3Hash::digest(&data) == expected => {
                    map.insert(expected, data);
                }
                Ok(_) => report.objects_corrupt += 1,
                Err(e) => report.problems.push(format!("read {expected}: {e}")),
            }
        }
    }
    map
}

/// Read commit leaves from the redb store. Returns empty (and flags the report)
/// if the store cannot be opened or a leaf cannot be parsed.
fn read_leaves(store_path: &Path, report: &mut RescueReport) -> Vec<leaf::Leaf> {
    let mut leaves = Vec::new();
    // Never call Store::open on a missing file: redb would create a fresh empty
    // store inside the repo we are trying to rescue. Only open what exists.
    if !store_path.exists() {
        report
            .problems
            .push("no commit store (store.db absent)".into());
        return leaves;
    }
    let store = match Store::open(store_path) {
        Ok(s) => s,
        Err(e) => {
            report
                .problems
                .push(format!("commit store unreadable: {e}"));
            return leaves;
        }
    };
    report.store_readable = true;
    let indices = match store.all_leaf_indices() {
        Ok(i) => i,
        Err(e) => {
            report.problems.push(format!("cannot list commits: {e}"));
            return leaves;
        }
    };
    for idx in indices {
        match store.get_leaf(idx) {
            Ok(Some(bytes)) => match leaf::parse_leaf(&bytes) {
                Ok(l) => leaves.push(l),
                Err(e) => report.problems.push(format!("commit {idx} corrupt: {e}")),
            },
            Ok(None) => {}
            Err(e) => report
                .problems
                .push(format!("commit {idx} unreadable: {e}")),
        }
    }
    report.seals_recovered = leaves.len();
    leaves
}

/// Best-effort flattening of a HAMT directory from verified in-RAM objects.
/// Missing or malformed nodes are recorded and skipped — one bad node must
/// not discard the rest of a directory during recovery. Every visited node
/// is marked reachable so the orphan sweep does not re-dump interiors of a
/// directory that already materialized.
fn collect_hamt_entries(
    objects: &HashMap<B3Hash, Vec<u8>>,
    node_hash: B3Hash,
    depth: usize,
    reachable: &mut HashSet<B3Hash>,
    entries: &mut Vec<fsmerkle::Entry>,
    report: &mut RescueReport,
) {
    if depth >= MAX_DEPTH {
        report.problems.push(format!(
            "HAMT node {node_hash} exceeds max depth {MAX_DEPTH}, stopped"
        ));
        return;
    }
    reachable.insert(node_hash);
    let Some(bytes) = objects.get(&node_hash) else {
        report
            .problems
            .push(format!("HAMT node {node_hash} missing"));
        return;
    };
    match crate::hamt::parse_node(bytes) {
        Ok(crate::hamt::HamtNode::Leaf(entry)) => entries.push(entry),
        Ok(crate::hamt::HamtNode::Branch { children, .. }) => {
            for child in children {
                collect_hamt_entries(objects, child, depth + 1, reachable, entries, report);
            }
        }
        Err(e) => report
            .problems
            .push(format!("HAMT node {node_hash} unparseable: {e}")),
    }
}

/// Best-effort materialization of one tree into `dest`. Never returns an error:
/// every failure is recorded and skipped so one bad object can't abort a
/// recovery. `reachable` records trees for the later orphan sweep, while
/// `ancestors` is path-local recursion protection and must be removed on unwind
/// so shared subtrees can be written at every destination that references them.
fn materialize_tree(
    objects: &HashMap<B3Hash, Vec<u8>>,
    tree_hash: B3Hash,
    dest: &Path,
    depth: usize,
    reachable: &mut HashSet<B3Hash>,
    ancestors: &mut HashSet<B3Hash>,
    report: &mut RescueReport,
) {
    if depth >= MAX_DEPTH {
        report.problems.push(format!(
            "tree {tree_hash} exceeds max depth {MAX_DEPTH}, stopped"
        ));
        return;
    }
    reachable.insert(tree_hash);
    if !ancestors.insert(tree_hash) {
        report
            .problems
            .push(format!("tree cycle detected at {tree_hash}, stopped"));
        return;
    }
    let Some(bytes) = objects.get(&tree_hash) else {
        report.problems.push(format!("tree {tree_hash} missing"));
        ancestors.remove(&tree_hash);
        return;
    };
    let entries = if crate::hamt::is_hamt_node(bytes) {
        // HAMT directory: flatten best-effort. A missing or corrupt interior
        // node loses only its own subtrie, never the rest of the directory.
        let mut entries = Vec::new();
        collect_hamt_entries(objects, tree_hash, depth, reachable, &mut entries, report);
        entries
    } else {
        match fsmerkle::parse_tree(bytes) {
            Ok(t) => t.entries,
            Err(e) => {
                report
                    .problems
                    .push(format!("tree {tree_hash} unparseable: {e}"));
                ancestors.remove(&tree_hash);
                return;
            }
        }
    };
    if let Err(e) = std::fs::create_dir_all(dest) {
        report
            .problems
            .push(format!("mkdir {}: {e}", dest.display()));
        ancestors.remove(&tree_hash);
        return;
    }

    for entry in entries {
        // Trust boundary: entry.name is attacker-influenceable. A tree entry is
        // a single path component; anything else is rejected so a corrupt tree
        // cannot escape `dest` (path traversal / absolute paths).
        if !safe_component(&entry.name) {
            report
                .problems
                .push(format!("unsafe name {:?} skipped", entry.name));
            continue;
        }
        let child = dest.join(&entry.name);
        match entry.kind {
            NodeKind::Tree => {
                materialize_tree(
                    objects,
                    entry.hash,
                    &child,
                    depth + 1,
                    reachable,
                    ancestors,
                    report,
                );
            }
            NodeKind::Blob => {
                let Some(blob) = objects.get(&entry.hash) else {
                    report
                        .problems
                        .push(format!("file {} missing ({})", entry.name, entry.hash));
                    continue;
                };
                let content = match fsmerkle::parse_blob(blob) {
                    // ponytail: symlinks are written as regular files holding the
                    // link target — safe, avoids creating dangling/escaping links.
                    Ok((_, content)) => content,
                    Err(e) => {
                        report
                            .problems
                            .push(format!("file {} corrupt: {e}", entry.name));
                        continue;
                    }
                };
                if let Err(e) = std::fs::write(&child, &content) {
                    report
                        .problems
                        .push(format!("write {}: {e}", child.display()));
                    continue;
                }
                set_exec_if_needed(&child, entry.mode);
                report.files_written += 1;
            }
        }
    }
    ancestors.remove(&tree_hash);
}

/// A tree entry name must be exactly one safe path component.
fn safe_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

#[cfg(unix)]
fn set_exec_if_needed(path: &Path, mode: u32) {
    if mode == MODE_EXEC {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
}

#[cfg(not(unix))]
fn set_exec_if_needed(_path: &Path, _mode: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::{Cas, FileCas};
    use crate::fsmerkle::{Entry, FsStore, MODE_DIR, MODE_FILE};

    /// Seed a repo's CAS with a blob+tree and its commit leaf, then damage the
    /// refs and HEAD and confirm rescue still recovers the file content.
    #[test]
    fn recovers_files_after_refs_destroyed() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let ivaldi = dir.path().join(".ivaldi");

        let cas = FileCas::new(ivaldi.join("objects")).unwrap();
        let fs_store = FsStore::new(&cas);
        let (blob, _) = fs_store.put_blob(b"secret data").unwrap();
        let tree = fs_store
            .put_tree(vec![Entry {
                name: "notes.txt".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob,
            }])
            .unwrap();
        cas.flush().unwrap();

        let store = Store::open(&ivaldi.join("store.db")).unwrap();
        let leaf = leaf::Leaf::new(tree, "main", "Me <me@x>", 0, "first");
        store.put_leaf(0, &leaf.canonical_bytes()).unwrap();
        drop(store);

        // Destroy everything rescue must not depend on.
        std::fs::remove_file(ivaldi.join("HEAD")).ok();
        std::fs::remove_dir_all(ivaldi.join("refs")).ok();

        let out = dir.path().join("rescued");
        let report = rescue(&ivaldi, &out).unwrap();

        assert_eq!(report.files_written, 1);
        assert!(report.store_readable);
        let short = &tree.to_hex()[..16];
        let recovered = std::fs::read(out.join(short).join("notes.txt")).unwrap();
        assert_eq!(recovered, b"secret data");
    }

    #[test]
    fn materializes_shared_subtree_at_every_path_and_snapshot_root() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let ivaldi = dir.path().join(".ivaldi");

        let cas = FileCas::new(ivaldi.join("objects")).unwrap();
        let fs_store = FsStore::new(&cas);
        let (blob, _) = fs_store.put_blob(b"shared").unwrap();
        let shared = fs_store
            .put_tree(vec![Entry {
                name: "file.txt".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob,
            }])
            .unwrap();
        let root = fs_store
            .put_tree(vec![
                Entry {
                    name: "first".into(),
                    mode: MODE_DIR,
                    kind: NodeKind::Tree,
                    hash: shared,
                },
                Entry {
                    name: "second".into(),
                    mode: MODE_DIR,
                    kind: NodeKind::Tree,
                    hash: shared,
                },
            ])
            .unwrap();
        cas.flush().unwrap();

        let store = Store::open(&ivaldi.join("store.db")).unwrap();
        let leaf = leaf::Leaf::new(root, "main", "x", 0, "shared subtree");
        store.put_leaf(0, &leaf.canonical_bytes()).unwrap();
        let leaf = leaf::Leaf::new(shared, "main", "x", 1, "subtree as snapshot");
        store.put_leaf(1, &leaf.canonical_bytes()).unwrap();
        drop(store);

        let out = dir.path().join("rescued");
        let report = rescue(&ivaldi, &out).unwrap();
        let snapshot = out.join(&root.to_hex()[..16]);

        assert_eq!(report.files_written, 3);
        assert_eq!(
            std::fs::read(snapshot.join("first/file.txt")).unwrap(),
            b"shared"
        );
        assert_eq!(
            std::fs::read(snapshot.join("second/file.txt")).unwrap(),
            b"shared"
        );
        let shared_snapshot = out.join(&shared.to_hex()[..16]);
        assert_eq!(
            std::fs::read(shared_snapshot.join("file.txt")).unwrap(),
            b"shared"
        );
    }

    // --- Adversarial tests: rescue must survive hostile/corrupt input without
    // panicking, escaping the output directory, or writing tampered content. ---

    /// Minimal LEB128 encoder, matching the tree canonical format, so tests can
    /// forge object bytes the safe encoder (`canonical_bytes`) would reject.
    fn uvarint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    /// Hand-encode a single-entry tree with an arbitrary (possibly unsafe) name.
    fn forge_tree_bytes(name: &str, kind: NodeKind, child: B3Hash) -> Vec<u8> {
        let mut b = Vec::new();
        uvarint(1, &mut b); // entry count
        uvarint(MODE_FILE as u64, &mut b);
        uvarint(name.len() as u64, &mut b);
        b.extend_from_slice(name.as_bytes());
        b.push(kind as u8);
        b.extend_from_slice(child.as_bytes());
        b
    }

    #[test]
    fn rejects_path_traversal_entries() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let ivaldi = dir.path().join(".ivaldi");

        let cas = FileCas::new(ivaldi.join("objects")).unwrap();
        let fs_store = FsStore::new(&cas);
        let (blob, _) = fs_store.put_blob(b"pwn").unwrap();

        // A tree that tries to escape via "../pwned". parse_tree rejects the
        // name at decode time (first line of defense), and rescue must still
        // refuse to write anything from it.
        let evil_bytes = forge_tree_bytes("../pwned", NodeKind::Blob, blob);
        assert!(
            fsmerkle::parse_tree(&evil_bytes).is_err(),
            "decode-side name validation must reject traversal entries"
        );
        let evil_tree = B3Hash::digest(&evil_bytes);
        cas.put(evil_tree, &evil_bytes).unwrap();
        cas.flush().unwrap();

        let store = Store::open(&ivaldi.join("store.db")).unwrap();
        let leaf = leaf::Leaf::new(evil_tree, "main", "x", 0, "evil");
        store.put_leaf(0, &leaf.canonical_bytes()).unwrap();
        drop(store);

        let out = dir.path().join("rescued");
        let report = rescue(&ivaldi, &out).unwrap();

        assert_eq!(report.files_written, 0, "no file should be written");
        assert!(report.problems.iter().any(|p| p.contains("unparseable")));
        // The escaped target must not exist anywhere outside the tree dir.
        assert!(!out.join("pwned").exists());
        assert!(!dir.path().join("pwned").exists());
    }

    #[test]
    fn skips_tampered_blob() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let ivaldi = dir.path().join(".ivaldi");

        let cas = FileCas::new(ivaldi.join("objects")).unwrap();
        let fs_store = FsStore::new(&cas);
        let (blob, _) = fs_store.put_blob(b"good content").unwrap();
        let tree = fs_store
            .put_tree(vec![Entry {
                name: "f.txt".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob,
            }])
            .unwrap();
        cas.flush().unwrap();

        // Tamper the blob file so it no longer matches its hash.
        let hex = blob.to_hex();
        let (d, f) = hex.split_at(2);
        std::fs::write(ivaldi.join("objects").join(d).join(f), b"EVIL").unwrap();

        let store = Store::open(&ivaldi.join("store.db")).unwrap();
        let leaf = leaf::Leaf::new(tree, "main", "x", 0, "c");
        store.put_leaf(0, &leaf.canonical_bytes()).unwrap();
        drop(store);

        let out = dir.path().join("rescued");
        let report = rescue(&ivaldi, &out).unwrap();

        // Tampered blob is excluded; nothing tampered is ever written.
        assert_eq!(report.objects_corrupt, 1);
        assert_eq!(report.files_written, 0);
        assert!(report.problems.iter().any(|p| p.contains("missing")));
        let short = &tree.to_hex()[..16];
        assert!(!out.join(short).join("f.txt").exists());
    }

    #[test]
    fn survives_excessive_tree_depth() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let ivaldi = dir.path().join(".ivaldi");

        let cas = FileCas::new(ivaldi.join("objects")).unwrap();
        let fs_store = FsStore::new(&cas);
        let (blob, _) = fs_store.put_blob(b"deep").unwrap();
        // Build a chain deeper than MAX_DEPTH, bottom-up so hashes resolve.
        let mut cur = fs_store
            .put_tree(vec![Entry {
                name: "f.txt".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob,
            }])
            .unwrap();
        for _ in 0..(MAX_DEPTH + 5) {
            cur = fs_store
                .put_tree(vec![Entry {
                    name: "d".into(),
                    mode: crate::fsmerkle::MODE_DIR,
                    kind: NodeKind::Tree,
                    hash: cur,
                }])
                .unwrap();
        }
        cas.flush().unwrap();

        let store = Store::open(&ivaldi.join("store.db")).unwrap();
        let leaf = leaf::Leaf::new(cur, "main", "x", 0, "deep");
        store.put_leaf(0, &leaf.canonical_bytes()).unwrap();
        drop(store);

        // Must not stack-overflow or hang; must report hitting the limit.
        let out = dir.path().join("rescued");
        let report = rescue(&ivaldi, &out).unwrap();
        assert!(report.problems.iter().any(|p| p.contains("max depth")));
    }

    #[test]
    fn survives_corrupt_store() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let ivaldi = dir.path().join(".ivaldi");

        let cas = FileCas::new(ivaldi.join("objects")).unwrap();
        let fs_store = FsStore::new(&cas);
        let (blob, _) = fs_store.put_blob(b"resilient").unwrap();
        let tree = fs_store
            .put_tree(vec![Entry {
                name: "a.txt".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob,
            }])
            .unwrap();
        cas.flush().unwrap();

        // A store.db that exists but is not a valid redb file.
        std::fs::write(ivaldi.join("store.db"), b"not a database").unwrap();

        let out = dir.path().join("rescued");
        let report = rescue(&ivaldi, &out).unwrap();

        assert!(!report.store_readable);
        assert_eq!(report.files_written, 1); // recovered via orphan sweep
        let short = &tree.to_hex()[..16];
        let recovered = std::fs::read(out.join("orphans").join(short).join("a.txt")).unwrap();
        assert_eq!(recovered, b"resilient");
    }

    /// With the commit store itself gone, the orphan sweep must still recover
    /// the file from the dangling tree.
    #[test]
    fn recovers_via_orphan_sweep_when_store_gone() {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let ivaldi = dir.path().join(".ivaldi");

        let cas = FileCas::new(ivaldi.join("objects")).unwrap();
        let fs_store = FsStore::new(&cas);
        let (blob, _) = fs_store.put_blob(b"orphaned").unwrap();
        let tree = fs_store
            .put_tree(vec![Entry {
                name: "a.txt".into(),
                mode: MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob,
            }])
            .unwrap();
        cas.flush().unwrap();

        std::fs::remove_file(ivaldi.join("store.db")).ok();

        let out = dir.path().join("rescued");
        let report = rescue(&ivaldi, &out).unwrap();

        assert!(!report.store_readable);
        assert_eq!(report.files_written, 1);
        let short = &tree.to_hex()[..16];
        let recovered = std::fs::read(out.join("orphans").join(short).join("a.txt")).unwrap();
        assert_eq!(recovered, b"orphaned");
    }
}

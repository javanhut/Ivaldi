//! Translate an Ivaldi history slice into git-format objects + packfile.
//!
//! Used by SSH push (`SshClient::push_repo`) to ship native Ivaldi seals to
//! a real git server via `git-receive-pack`. The remote server only speaks
//! git's wire format — packfile of zlib-compressed objects with SHA-1
//! identifiers + 20-byte trailer — so we have to translate.
//!
//! Three translations happen here:
//!
//! 1. **Blob**. Ivaldi blob CAS bytes are *literally* `"blob <size>\0<content>"`
//!    — the same canonical envelope git uses. We strip the header to get the
//!    raw body for packing; the git SHA-1 is over the full envelope.
//!
//! 2. **Tree**. Ivaldi trees use a custom uvarint encoding. We load each
//!    Ivaldi tree, recursively translate every entry, and emit the git tree
//!    body (`<mode> <name>\0<20-byte-sha1>` per entry, sorted by name).
//!
//! 3. **Commit**. Ivaldi `Leaf` doesn't map 1:1 to a git commit. We mint a
//!    canonical git commit body from the leaf's fields:
//!
//!    ```text
//!    tree <sha1-hex>\n
//!    parent <sha1-hex>\n  (zero or more, in prev_idx + merge_idxs order)
//!    author <Name> <<email>> <unix-secs> <±HHMM>\n
//!    committer <Name> <<email>> <unix-secs> <±HHMM>\n
//!    \n
//!    <message>
//!    ```
//!
//!    For leaves originally imported from git (the `download` path stashes
//!    `git.committer`, `git.committer_time`, `git.committer_tz`,
//!    `git.author_tz` in `leaf.meta`), we faithfully reconstruct the
//!    committer line, which means the round-tripped commit hashes match
//!    the originals byte-for-byte.

use std::collections::{BTreeMap, BTreeSet};

use crate::cas::FileCas;
use crate::fsmerkle::{self, FsStore, NodeKind};
use crate::git_remote::{GitObjectKind, git_object_id};
use crate::hash::B3Hash;
use crate::leaf::{Leaf, NO_PARENT};
use crate::repo::Repo;

/// One git object ready to be packed.
#[derive(Debug, Clone)]
pub struct GitObject {
    pub sha1: [u8; 20],
    pub kind: GitObjectKind,
    /// Raw object body (no `<type> <size>\0` envelope — the pack writer
    /// adds its own framing).
    pub body: Vec<u8>,
}

/// Result of translating an Ivaldi commit chain to git.
#[derive(Debug)]
pub struct ExportResult {
    /// Every distinct git object we need to send (commits + trees + blobs).
    /// Keyed + deduped by git SHA-1.
    pub objects: BTreeMap<[u8; 20], GitObject>,
    /// Git SHA-1 of the new tip commit (the value `git-receive-pack`
    /// expects on the right-hand side of its update command).
    pub tip_sha1: [u8; 20],
}

/// Translate every commit reachable from `head_idx` along `prev_idx +
/// merge_idxs` into git objects.
///
/// `server_has_sha1` is the set of git SHA-1s the *target* server already
/// advertised — only commits whose mapped SHA-1 is in this set are
/// skipped (i.e. their tree+blob translation is also skipped, since the
/// server already has them). Pass an empty set for an empty remote.
///
/// `known_mapping` is consulted only for the SHA-1 lookup; nothing is
/// skipped purely because it was seen on *some* prior remote.
pub fn export_chain(
    repo: &Repo,
    head_idx: u64,
    known_mapping: &crate::remote::HashMapping,
    server_has_sha1: &BTreeSet<[u8; 20]>,
) -> Result<ExportResult, ExportError> {
    let cas = FileCas::new(repo.ivaldi_dir.join("objects"))
        .map_err(|e| ExportError::Other(format!("cas: {}", e)))?;
    let store = FsStore::new(&cas);

    // Topological order of leaves to translate: deepest-ancestor-first so
    // each leaf's parents already have their git SHA-1s by the time we
    // mint the commit object.
    let order = collect_topological(repo, head_idx, known_mapping, server_has_sha1)?;

    // Per-translation state.
    let mut objects: BTreeMap<[u8; 20], GitObject> = BTreeMap::new();
    let mut leaf_to_git: BTreeMap<u64, [u8; 20]> = BTreeMap::new();
    let mut tree_translation_cache: BTreeMap<B3Hash, [u8; 20]> = BTreeMap::new();

    // Seed leaf_to_git with already-mapped ancestors so parent lookups
    // resolve without re-translating.
    for idx in 0..repo.commit_count() {
        if let Ok(Some(leaf)) = repo.get_leaf(idx)
            && let Some(sha_str) = known_mapping.get_sha1(leaf.hash())
            && let Some(b) = sha1_hex_to_bytes(sha_str)
        {
            leaf_to_git.insert(idx, b);
        }
    }

    let mut tip_sha1 = [0u8; 20];

    for idx in &order {
        let leaf = repo
            .get_leaf(*idx)
            .map_err(|e| ExportError::Other(e.to_string()))?
            .ok_or_else(|| ExportError::Other(format!("leaf {} vanished", idx)))?;

        // Translate the tree first — commit body needs the tree SHA-1.
        let tree_sha1 = translate_tree(
            &store,
            leaf.tree_root,
            &mut tree_translation_cache,
            &mut objects,
        )?;

        // Resolve parent SHA-1s from already-translated map.
        let mut parents: Vec<[u8; 20]> = Vec::new();
        if leaf.has_parent()
            && let Some(p) = leaf_to_git.get(&leaf.prev_idx).copied()
        {
            parents.push(p);
        }
        for &midx in &leaf.merge_idxs {
            if let Some(p) = leaf_to_git.get(&midx).copied()
                && !parents.contains(&p)
            {
                parents.push(p);
            }
        }

        let body = mint_git_commit_body(&leaf, &tree_sha1, &parents);
        let sha1: [u8; 20] = git_object_id(GitObjectKind::Commit, &body)
            .as_str()
            .pipe(|hex| sha1_hex_to_bytes(hex).expect("git_object_id returns 40-hex"));
        objects.insert(
            sha1,
            GitObject {
                sha1,
                kind: GitObjectKind::Commit,
                body,
            },
        );
        leaf_to_git.insert(*idx, sha1);
        tip_sha1 = sha1;
    }

    Ok(ExportResult { objects, tip_sha1 })
}

// =====================================================================
// Topological traversal
// =====================================================================

fn collect_topological(
    repo: &Repo,
    head_idx: u64,
    known_mapping: &crate::remote::HashMapping,
    server_has_sha1: &BTreeSet<[u8; 20]>,
) -> Result<Vec<u64>, ExportError> {
    // BFS to discover all unmapped ancestors, then reverse-prev_idx
    // ordering produces parent-before-child chronology.
    let mut chain: Vec<u64> = Vec::new();
    let mut visited: BTreeSet<u64> = BTreeSet::new();
    let mut stack = vec![head_idx];
    while let Some(idx) = stack.pop() {
        if !visited.insert(idx) {
            continue;
        }
        let leaf = match repo
            .get_leaf(idx)
            .map_err(|e| ExportError::Other(e.to_string()))?
        {
            Some(l) => l,
            None => continue,
        };
        // Stop descending only when this leaf's mapped git SHA-1 is in
        // the *server's* advertised set — i.e. the server already has
        // this exact commit + everything it transitively references.
        if let Some(sha_str) = known_mapping.get_sha1(leaf.hash())
            && let Some(b) = sha1_hex_to_bytes(sha_str)
            && server_has_sha1.contains(&b)
        {
            continue;
        }
        chain.push(idx);
        for p in leaf.all_parents() {
            stack.push(p);
        }
    }
    // Sort by index ascending — for an Ivaldi MMR, indices are monotonic
    // in commit order so ascending == parents-before-children.
    chain.sort_unstable();
    Ok(chain)
}

// =====================================================================
// Tree translation (Ivaldi → git)
// =====================================================================

fn translate_tree(
    store: &FsStore<'_>,
    tree_hash: B3Hash,
    cache: &mut BTreeMap<B3Hash, [u8; 20]>,
    objects: &mut BTreeMap<[u8; 20], GitObject>,
) -> Result<[u8; 20], ExportError> {
    if let Some(s) = cache.get(&tree_hash).copied() {
        return Ok(s);
    }
    let tree = store
        .load_tree(tree_hash)
        .map_err(|e| ExportError::Other(e.to_string()))?;

    // Git tree entries must be sorted lexically by name. Ivaldi already
    // canonicalizes that way, but be defensive.
    let mut entries = tree.entries.clone();
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut body: Vec<u8> = Vec::new();
    for entry in &entries {
        let (mode_str, child_sha1) = match entry.kind {
            NodeKind::Blob => {
                // Emit the git mode that matches the stored Ivaldi mode so the
                // round-trip preserves executable bits and symlinks (which are
                // part of the git tree object and therefore its SHA-1).
                let mode = match entry.mode {
                    fsmerkle::MODE_EXEC => "100755",
                    fsmerkle::MODE_SYMLINK => "120000",
                    _ => "100644",
                };
                let sha = translate_blob(store, entry.hash, objects)?;
                (mode, sha)
            }
            NodeKind::Tree => {
                let sha = translate_tree(store, entry.hash, cache, objects)?;
                ("40000", sha)
            }
        };
        body.extend_from_slice(mode_str.as_bytes());
        body.push(b' ');
        body.extend_from_slice(entry.name.as_bytes());
        body.push(0);
        body.extend_from_slice(&child_sha1);
    }

    let sha1 = sha1_hex_to_bytes(git_object_id(GitObjectKind::Tree, &body).as_str())
        .expect("git_object_id always returns 40 hex chars");
    objects.insert(
        sha1,
        GitObject {
            sha1,
            kind: GitObjectKind::Tree,
            body,
        },
    );
    cache.insert(tree_hash, sha1);
    Ok(sha1)
}

fn translate_blob(
    store: &FsStore<'_>,
    blob_hash: B3Hash,
    objects: &mut BTreeMap<[u8; 20], GitObject>,
) -> Result<[u8; 20], ExportError> {
    // `load_blob` returns (BlobNode, raw_content). We want the raw content
    // for the git pack body; the git SHA-1 is over `blob <len>\0<content>`,
    // which is exactly the Ivaldi blob CAS canonical form.
    let (_, content) = store
        .load_blob(blob_hash)
        .map_err(|e| ExportError::Other(e.to_string()))?;
    let sha1 = sha1_hex_to_bytes(git_object_id(GitObjectKind::Blob, &content).as_str())
        .expect("git_object_id always returns 40 hex chars");
    objects.entry(sha1).or_insert(GitObject {
        sha1,
        kind: GitObjectKind::Blob,
        body: content,
    });
    Ok(sha1)
}

// =====================================================================
// Commit body minting
// =====================================================================

fn mint_git_commit_body(leaf: &Leaf, tree_sha1: &[u8; 20], parents: &[[u8; 20]]) -> Vec<u8> {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "tree {}", hex::encode(tree_sha1));
    for p in parents {
        let _ = writeln!(s, "parent {}", hex::encode(p));
    }

    let author_tz = leaf
        .meta
        .get("git.author_tz")
        .map(String::as_str)
        .unwrap_or("+0000");
    let _ = writeln!(s, "author {} {} {}", leaf.author, leaf.time_unix, author_tz);

    let (committer_line, committer_time, committer_tz) = match (
        leaf.meta.get("git.committer"),
        leaf.meta.get("git.committer_time"),
    ) {
        (Some(c), Some(t)) => {
            let time = t.parse::<i64>().unwrap_or(leaf.time_unix);
            let tz = leaf
                .meta
                .get("git.committer_tz")
                .map(String::as_str)
                .unwrap_or("+0000");
            (c.clone(), time, tz.to_string())
        }
        _ => (leaf.author.clone(), leaf.time_unix, author_tz.to_string()),
    };
    let _ = writeln!(
        s,
        "committer {} {} {}",
        committer_line, committer_time, committer_tz
    );
    s.push('\n');
    s.push_str(&leaf.message);
    // Do NOT auto-append a trailing newline. git's canonical commit bytes
    // preserve the message verbatim — appending here would change the
    // SHA-1 for any commit whose original message didn't end with `\n`.
    s.into_bytes()
}

// =====================================================================
// Helpers
// =====================================================================

fn sha1_hex_to_bytes(hex: &str) -> Option<[u8; 20]> {
    if hex.len() != 40 {
        return None;
    }
    let raw = hex::decode(hex).ok()?;
    let mut out = [0u8; 20];
    out.copy_from_slice(&raw);
    Some(out)
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("git export: {0}")]
    Other(String),
}

// Suppress dead-code warnings for the unused legacy `NO_PARENT` constant
// referenced in some traversal-helper variants we may add later.
#[allow(dead_code)]
const _: u64 = NO_PARENT;

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_commit_body_includes_tree_parents_author_committer() {
        let mut leaf = Leaf::new(
            B3Hash::digest(b"t"),
            "main",
            "Jane Doe <jane@example.com>",
            1_700_000_000,
            "first commit",
        );
        leaf.meta.insert("git.author_tz".into(), "+0530".into());
        leaf.meta
            .insert("git.committer".into(), "Bob <bob@example.com>".into());
        leaf.meta
            .insert("git.committer_time".into(), "1700000060".into());
        leaf.meta.insert("git.committer_tz".into(), "+0100".into());

        let tree = [0xAB; 20];
        let parent = [0xCD; 20];
        let body = mint_git_commit_body(&leaf, &tree, &[parent]);
        let s = std::str::from_utf8(&body).unwrap();
        assert!(s.starts_with(&format!("tree {}\n", hex::encode([0xAB; 20]))));
        assert!(s.contains(&format!("\nparent {}\n", hex::encode([0xCD; 20]))));
        assert!(s.contains("\nauthor Jane Doe <jane@example.com> 1700000000 +0530\n"));
        assert!(s.contains("\ncommitter Bob <bob@example.com> 1700000060 +0100\n"));
        // git canonical commit bytes preserve the message verbatim — no
        // auto-appended trailing newline.
        assert!(s.ends_with("first commit"));
    }

    #[test]
    fn mint_commit_body_falls_back_to_author_when_meta_missing() {
        let leaf = Leaf::new(
            B3Hash::digest(b"t"),
            "main",
            "Solo <solo@x>",
            1_700_000_000,
            "no committer meta",
        );
        let tree = [0u8; 20];
        let body = mint_git_commit_body(&leaf, &tree, &[]);
        let s = std::str::from_utf8(&body).unwrap();
        assert!(s.contains("\nauthor Solo <solo@x> 1700000000 +0000\n"));
        assert!(s.contains("\ncommitter Solo <solo@x> 1700000000 +0000\n"));
    }

    #[test]
    fn mint_commit_body_no_parents_for_root() {
        let leaf = Leaf::new(
            B3Hash::digest(b"t"),
            "main",
            "Author <a@x>",
            1_700_000_000,
            "root",
        );
        let body = mint_git_commit_body(&leaf, &[0u8; 20], &[]);
        let s = std::str::from_utf8(&body).unwrap();
        assert!(!s.contains("\nparent "));
    }

    #[test]
    fn translate_tree_round_trips_a_single_blob() {
        // Build an Ivaldi tree with one file, translate to git, and check
        // both the body shape and that the tree's git SHA-1 matches what
        // git itself would produce for the same content.
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path().join("objects")).unwrap();
        let store = FsStore::new(&cas);
        let (blob_hash, _) = store.put_blob(b"hello").unwrap();
        let tree_hash = store
            .put_tree(vec![fsmerkle::Entry {
                name: "greet.txt".into(),
                mode: fsmerkle::MODE_FILE,
                kind: NodeKind::Blob,
                hash: blob_hash,
            }])
            .unwrap();

        let mut cache = BTreeMap::new();
        let mut objects: BTreeMap<[u8; 20], GitObject> = BTreeMap::new();
        let tree_sha = translate_tree(&store, tree_hash, &mut cache, &mut objects).unwrap();

        // Two objects: the blob and the tree.
        assert_eq!(objects.len(), 2);

        // Git SHA-1 of `blob 5\0hello` is well-known.
        let blob_obj = objects
            .values()
            .find(|o| matches!(o.kind, GitObjectKind::Blob))
            .unwrap();
        assert_eq!(
            hex::encode(blob_obj.sha1),
            "b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0"
        );

        // Tree body shape: `100644 greet.txt\0<sha1>`.
        let tree_obj = objects.get(&tree_sha).unwrap();
        let tree_body = &tree_obj.body;
        assert!(tree_body.starts_with(b"100644 greet.txt\0"));
        // 20-byte SHA-1 immediately after the NUL.
        let nul = tree_body.iter().position(|&b| b == 0).unwrap();
        assert_eq!(tree_body[nul + 1..nul + 21], blob_obj.sha1);
    }

    #[test]
    fn translate_tree_preserves_exec_and_symlink_modes() {
        // Executable bit and symlinks are part of the git tree object (and thus
        // its SHA-1). Export must emit 100755 / 120000, not collapse to 100644.
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path().join("objects")).unwrap();
        let store = FsStore::new(&cas);

        let (regular, _) = store.put_blob(b"plain").unwrap();
        let (script, _) = store.put_blob(b"#!/bin/sh\n").unwrap();
        // A symlink blob's content is the link target path.
        let (link, _) = store.put_blob(b"regular.txt").unwrap();

        let tree_hash = store
            .put_tree(vec![
                fsmerkle::Entry {
                    name: "link".into(),
                    mode: fsmerkle::MODE_SYMLINK,
                    kind: NodeKind::Blob,
                    hash: link,
                },
                fsmerkle::Entry {
                    name: "regular.txt".into(),
                    mode: fsmerkle::MODE_FILE,
                    kind: NodeKind::Blob,
                    hash: regular,
                },
                fsmerkle::Entry {
                    name: "run.sh".into(),
                    mode: fsmerkle::MODE_EXEC,
                    kind: NodeKind::Blob,
                    hash: script,
                },
            ])
            .unwrap();

        let mut cache = BTreeMap::new();
        let mut objects: BTreeMap<[u8; 20], GitObject> = BTreeMap::new();
        let tree_sha = translate_tree(&store, tree_hash, &mut cache, &mut objects).unwrap();
        let body = &objects.get(&tree_sha).unwrap().body;

        let contains = |needle: &[u8]| body.windows(needle.len()).any(|w| w == needle);
        assert!(contains(b"100755 run.sh\0"), "executable mode preserved");
        assert!(contains(b"120000 link\0"), "symlink mode preserved");
        assert!(contains(b"100644 regular.txt\0"), "regular mode preserved");
    }

    #[test]
    fn deleted_file_is_absent_from_exported_tree() {
        // A seal that removes a file produces a `tree_root` without that file
        // (see `Workspace::build_seal_tree`). Export must faithfully emit a git
        // tree that also omits it — that's how a deletion reaches the remote on
        // push. Here we model the before/after trees directly and check the
        // exported git tree drops the deleted entry (and changes SHA-1).
        let dir = tempfile::tempdir().unwrap();
        let cas = FileCas::new(dir.path().join("objects")).unwrap();
        let store = FsStore::new(&cas);

        let (a, _) = store.put_blob(b"a").unwrap();
        let (b, _) = store.put_blob(b"b").unwrap();
        let entry = |name: &str, hash| fsmerkle::Entry {
            name: name.into(),
            mode: fsmerkle::MODE_FILE,
            kind: NodeKind::Blob,
            hash,
        };

        let before = store
            .put_tree(vec![entry("a.txt", a), entry("b.txt", b)])
            .unwrap();
        let after = store.put_tree(vec![entry("a.txt", a)]).unwrap(); // b.txt deleted

        let mut cache = BTreeMap::new();
        let mut objects: BTreeMap<[u8; 20], GitObject> = BTreeMap::new();
        let before_sha = translate_tree(&store, before, &mut cache, &mut objects).unwrap();
        let after_sha = translate_tree(&store, after, &mut cache, &mut objects).unwrap();

        assert_ne!(before_sha, after_sha, "deletion must change the tree SHA-1");
        let after_body = &objects.get(&after_sha).unwrap().body;
        let contains = |needle: &[u8]| after_body.windows(needle.len()).any(|w| w == needle);
        assert!(contains(b"100644 a.txt\0"), "kept file present");
        assert!(!contains(b"b.txt\0"), "deleted file absent from exported tree");
    }
}

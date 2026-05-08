# Git Export (`git_export.rs`)

Translates an Ivaldi commit chain into the git objects + git SHA-1s
needed for SSH push (`git-receive-pack`). Pairs with
[`git_pack_writer.rs`](git_pack_writer.md), which packages the result
into a git-format packfile.

## What gets translated

Three object kinds, three different rules:

### Blobs — zero-cost

Ivaldi blob CAS bytes are *literally* git's blob canonical form:

```text
blob <size>\0<content>
```

So we just read the CAS bytes and use them directly. The SHA-1 is over
the full envelope (including the `blob <size>\0` prefix), and the body
shipped in the pack is the raw content.

### Trees — recursive remap

Ivaldi trees use a custom uvarint encoding; git trees use a sorted
sequence of `<mode> <name>\0<20-byte-sha1>` entries. We load the Ivaldi
tree, recursively translate every child (blob or sub-tree) to its git
SHA-1, then emit the git tree body sorted by name.

Modes:

| Ivaldi entry | Git mode |
|---|---|
| `MODE_FILE` blob | `100644` |
| `MODE_DIR` tree | `40000` (no leading zero — git is strict) |

Submodules (`160000` gitlinks) are dropped at *import* time, so they
don't appear in the export either.

### Commits — minted from leaf fields

Ivaldi `Leaf` doesn't map 1:1 to a git commit. We mint canonical git
commit bytes from `Leaf` fields:

```text
tree <sha1-hex>\n
parent <sha1-hex>\n  (zero or more, prev_idx + merge_idxs in order)
author <Name> <<email>> <unix-secs> <±HHMM>\n
committer <Name> <<email>> <unix-secs> <±HHMM>\n
\n
<message>
```

Two important rules so SHA-1 matches upstream byte-for-byte when
round-tripping a git-imported repo:

1. **Verbatim message preservation.** No auto-appended trailing `\n`.
   Git's canonical form preserves whatever the original commit had.
2. **Committer/timezone fidelity.** For leaves originally imported from
   git, `import_fetch_result` stashes `git.committer` /
   `git.committer_time` / `git.committer_tz` / `git.author_tz` in
   `leaf.meta`. These are read back here verbatim. For native Ivaldi
   commits with no `git.*` meta, committer mirrors author and timezone
   defaults to `+0000`.

## Server-aware skip

`export_chain` takes a `server_has_sha1: BTreeSet<[u8; 20]>` of SHA-1s
the *target* server already advertised in its receive-pack ref
advertisement. While walking the local commit DAG, a leaf is skipped
**only** when its mapped SHA-1 is in that set — i.e. that exact remote
already has this commit + everything it transitively references.

This is the key correctness fix vs. an earlier draft that skipped any
leaf already in the global `HashMapping`. That earlier behavior was
wrong: it meant "I cloned this from GitHub, so when I push it to a
*different* remote, send nothing." The current behavior treats each
remote's advertisement as the source of truth.

## Topological order

Leaves are emitted oldest-first (by MMR index, which is monotonic in
commit creation order). This guarantees each leaf's parents have already
been minted (and thus their SHA-1s known) by the time we mint the leaf
itself.

## Object dedup

Output is a `BTreeMap<[u8; 20], GitObject>` keyed by SHA-1, so the same
git object is never emitted twice — important for trees, where a
sub-tree shared across many commits is encoded once.

## Verified round-trip

`octocat/Hello-World`'s merge commit:

```text
upstream:    7fd1a60b01f91b314f59955a4e4d4e80d8edf11d
ivaldi:      7fd1a60b01f91b314f59955a4e4d4e80d8edf11d
```

After `ivaldi download octocat/Hello-World` followed by
`ivaldi upload <ssh-portal>`, the bare git remote ends up with the exact
same commit SHA. See [`docs/ssh.md`](ssh.md#verifying-a-real-round-trip)
for the test recipe.

## Tests

4 unit tests in `src/git_export.rs`:

- `mint_commit_body_includes_tree_parents_author_committer` — full
  field coverage with both `git.*` meta keys present.
- `mint_commit_body_falls_back_to_author_when_meta_missing` — native
  Ivaldi commit path.
- `mint_commit_body_no_parents_for_root` — root commit emits no
  `parent` lines.
- `translate_tree_round_trips_a_single_blob` — actually computes git
  SHA-1s and asserts the well-known `blob 5\0hello` SHA
  (`b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0`) is produced.

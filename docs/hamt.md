# HAMT Prototype (`hamt.rs`)

## Status

The HAMT is an experimental, in-memory prototype for a possible future
large-directory index. It is not used by the repository, workspace, CAS,
synchronization, or Git interoperability paths.

Ivaldi currently stores directories as canonical content-addressed Merkle tree
nodes in `fsmerkle`. Unchanged directory subtrees retain the same hashes across
seals, providing storage-level reuse without a HAMT.

The prototype's immutable API keeps older values unchanged, but its current
`Box<Node>` representation deep-clones branches. It therefore does not yet
provide memory or storage structural sharing.

## Possible production implementation

A production HAMT would be an optimization for directories large enough that
rebuilding and sorting their complete entry lists is measurably expensive. It
would not replace the filesystem Merkle tree as a whole.

The likely implementation would:

1. Define a canonical, versioned encoding for HAMT leaf and branch nodes.
2. Store every encoded node in the existing CAS under its BLAKE3 hash.
3. Represent a directory by its root HAMT hash.
4. Rewrite only the nodes on the path to an inserted, changed, or removed
   entry; unchanged nodes would retain their hashes.
5. Traverse entries in Git's canonical ordering when exporting a Git tree.
6. Validate node depth, bitmap/child counts, duplicate names, and hash
   integrity when loading untrusted nodes.
7. Provide a repository-format migration or retain support for both directory
   encodings through an explicit format version.

Using CAS hashes is preferable to merely replacing `Box<Node>` with
`Arc<Node>`: `Arc` would share memory within one process, while CAS-backed
nodes would share data across processes, seals, and synchronized repositories.

## Integration criteria

The HAMT should be integrated only if benchmarks demonstrate a meaningful
improvement over `fsmerkle`. Benchmarks should cover directories containing
1,000, 10,000, 100,000, and 1,000,000 entries and measure:

- initial tree construction;
- lookup;
- adding, modifying, and removing one entry;
- status and tree comparison;
- repository reopen and cold-cache loading;
- Git tree export;
- stored bytes and CAS object count.

The production implementation should also have property tests comparing its
contents with a `BTreeMap`, corruption tests for every encoded node type, and
round-trip tests proving Git export remains deterministic.

Until those criteria are met, `fsmerkle` remains Ivaldi's sole directory
storage implementation.

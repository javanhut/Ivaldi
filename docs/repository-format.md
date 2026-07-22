# Repository Format and Compatibility

How Ivaldi versions its on-disk format so repositories stay openable across
upgrades.

## The `FORMAT` file

`forge` writes `.ivaldi/FORMAT` as plain `key = value` lines:

```
format = 2
min_ivaldi = 0.1.2
features =
```

- `format` — the on-disk format version (an integer).
- `min_ivaldi` — the oldest Ivaldi release that understands this format, shown
  in the "too new" error so a user knows what to install.
- `features` — reserved for optional features; empty today.

The plain line format is deliberate: an older or newer binary can read the keys
it knows and ignore ones it does not. Unknown keys are forward-compatible.

## Compatibility gate

`Repo::open` calls `forge::check_format` before touching anything. The rule is a
simple version comparison:

- The binary knows the maximum format it supports (`CURRENT_FORMAT`).
- If a repository's `format` is **newer**, the open is refused with
  `FormatTooNew`, which names the version to install — the repository is never
  misread by a binary too old to understand it.
- A missing `FORMAT` file reads as **format 0**: repositories created before
  `FORMAT` existed still open.

## Format history

- **Format 1** — the original encoding set.
- **Format 2** — directories with more than `HAMT_DIR_THRESHOLD` (256)
  entries are stored as HAMT roots (see [hamt.md](hamt.md)). New repositories
  are stamped format 2; format-1 repositories remain fully supported
  read/write and never receive HAMT objects, so **no migration is needed** —
  the older format is a permanent citizen, not a deprecated one. A binary too
  old to know format 2 refuses such repositories via `FormatTooNew`.

## Bumping the format

Increment `CURRENT_FORMAT` on any breaking change to a persisted encoding
(leaves, trees, packs, journals, configuration, …). Older binaries will then
correctly refuse the new repositories. A bump that stops supporting an older
format must ship a forward migration and pre-migration backup — see the
roadmap in [`../plan.md`](../plan.md), Gate 3. Format 2 did not need one
because format 1 stays fully supported.

## Explicit migration and rollback

Older repositories remain usable in place. `ivaldi migrate` is an explicit,
opt-in promotion to the current write format; it is never performed merely by
opening a repository.

Before modifying the source repository, migration:

1. copies every repository file and object to `.ivaldi/migrations/backup`;
2. writes a versioned manifest containing the length and BLAKE3 of every file;
3. verifies the completed snapshot against that manifest;
4. publishes a pending marker that makes ordinary repository opens fail closed;
5. opens and fully verifies the repository before and after changing `FORMAT`;
6. retains the verified snapshot and a migration receipt.

If migration returns an error, it restores the verified source snapshot. If the
process or machine stops at a migration boundary, normal opens remain blocked;
rerunning `ivaldi migrate` first restores the exact old snapshot and then
restarts the upgrade. It never guesses whether an interrupted write landed.

`ivaldi migrate --rollback` restores the pre-migration snapshot. Automatic
rollback is deliberately refused after any repository mutation is attempted,
because overwriting post-migration work would violate Ivaldi's durability
contract. Read-only commands such as `status` and `verify` do not invalidate
rollback. Rollback itself uses the same pending marker and is idempotently
retryable after interruption.

The snapshot is complete rather than metadata-only, so enough free space for a
second copy of `.ivaldi` is required. Symlinks and unsupported filesystem entry
types inside `.ivaldi` are rejected rather than followed.

The non-negotiable executable contract lives in
`tests/repository_migration.rs`. It covers immutable format inputs, contentful
and divergent history, staging and permission preservation, exact rollback,
corrupt source and backup rejection, too-new non-mutation, post-migration work
protection, and deterministic process aborts at migration and rollback
publication boundaries.

Related: [forge.md](forge.md) (writes `FORMAT`), [repo.md](repo.md) (checks it),
[verify.md](verify.md) (reports it), [hamt.md](hamt.md) (format 2).

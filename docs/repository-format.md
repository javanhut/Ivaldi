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

Related: [forge.md](forge.md) (writes `FORMAT`), [repo.md](repo.md) (checks it),
[verify.md](verify.md) (reports it), [hamt.md](hamt.md) (format 2).

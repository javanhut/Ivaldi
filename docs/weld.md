# Weld — collapse a seal range into one

`ivaldi weld` combines a contiguous range of seals on the current timeline
into a single new seal, producing a linear history. The replaced seals
remain in the MMR for content-addressed integrity but are no longer
reachable from the timeline head.

This replaces the older `shift` command. The verb fits Ivaldi's metalwork
theme — welding combines pieces of metal into one — and the CLI is more
explicit about the resulting shape of history.

## Usage

```text
ivaldi weld --last N [-m MSG]                 # combine the last N seals
ivaldi weld START [-m MSG]                    # combine START..HEAD
ivaldi weld START to END [-m MSG]             # combine START..END (with `to` connector)
ivaldi weld START END [-m MSG]                # same, no connector
ivaldi weld                                   # interactive TUI picker
```

The alias `w` is accepted for all forms.

`START` and `END` may be seal names (`bold-tower-finds-loud-…`) or hash
prefixes. `END` defaults to the current timeline head.

## What weld does

Given a range `[START, …, END]` on the current timeline (oldest →
newest):

1. The welded seal is created with:
   - `tree_root` = the tree of the **newest** seal in the range (END)
   - `prev_idx` = the parent of the **oldest** seal in the range (START)
   - `message` = `-m MSG` if provided, otherwise an auto-generated summary
     of the welded seals' first-line messages, oldest → newest
2. Any seals that come **after** END on the timeline (trailing seals) are
   **replayed** on top of the welded seal — same tree, author, message,
   and timestamp; only `prev_idx` changes (so their hashes change). Without
   this replay, `weld bold to clear` in the middle of a chain would silently
   drop everything that came after `clear`.
3. The timeline head is updated to point at either the welded seal (no
   trailing seals) or the last replayed seal (with trailing seals).

## Examples

### Tail collapse

```
Before:  A → B → C → D → E   (timeline head = E)

ivaldi weld --last 3 -m "consolidate"

After:   A → B → W           (W's tree = E's tree;  message = "consolidate")
```

### Middle collapse

```
Before:  A → B → C → D → E   (timeline head = E)

ivaldi weld B to D -m "merged BCD"

After:   A → W → E'          (W = B-to-D welded; E' = E replayed on top of W)
```

`E'` has the same tree/author/message/timestamp as `E`, but a new hash
because its parent changed.

## Combined message format (when `-m` is omitted)

```
Welded N seals:

- <oldest seal's first line>
- <next seal's first line>
- …
- <newest seal's first line>
```

## Errors

| Condition | Error |
|---|---|
| Range smaller than 2 seals | `need at least 2 seals to weld` |
| `--last N` with N > history length | `only K seals on '<timeline>', need N` |
| Range not contiguous on the timeline | `range is not contiguous on '<timeline>' …` |
| START not an ancestor of END | `start seal X is not an ancestor of Y on '<timeline>'` |
| END not reachable from current head | `end seal X is not reachable from current timeline head` |
| Bad connector (e.g. `weld A from B`) | `expected 'weld START to END' (got 'from' between names)` |

## Design

- **MMR is append-only**: original seals remain at their indices; only the
  timeline-head pointer chain changes.
- **Atomic per seal**: each seal write goes through `Repo::commit_raw`,
  which uses a single redb write transaction (one fsync per seal).
- **Trailing-seal hash churn**: replaying trailing seals changes their
  `prev_idx` and therefore their hashes. There is no way to preserve the
  original hashes of trailing seals when their parent moves; this is the
  same trade-off `git rebase` makes.
- **Interactive picker**: when run with no arguments, `weld` reuses the
  existing `tui::shift` picker for selecting the start/end and entering a
  message.

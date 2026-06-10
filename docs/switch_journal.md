# Switch Journal Module (`switch_journal.rs`)

Crash journal for timeline switches.

## Overview

A timeline switch is multi-step: shelve the current timeline's dirty
state, rewrite HEAD, materialize the target tree, restore the target's
shelf. A crash mid-sequence used to leave the working tree half
transitioned with nothing recording that fact.

The journal file `.ivaldi/SWITCH_IN_PROGRESS` (JSON, written atomically)
is created after the shelve phase — the only non-idempotent part — and
removed when the switch completes:

```json
{ "from": "main", "to": "feature", "shelf_saved": true, "started_at": 1781000000 }
```

## Recovery

While the journal exists:

- **Mutating commands refuse to run** with guidance
  ("an interrupted timeline switch from 'main' to 'feature' needs recovery…").
  Read-only commands stay usable for orientation.
- `ivaldi timeline switch feature` (the original target) **completes** the
  switch: HEAD rewrite, materialize, and shelf restore are all idempotent
  and simply replay.
- `ivaldi timeline switch main` (the original source) **rolls back** the
  same way.
- Switching to any third timeline is refused until the transition is
  resolved.
- The worktree is never re-captured during recovery — it is mid-transition;
  the source timeline's dirty state already lives in its shelf.

A corrupt journal is a hard error naming the file (inspect, and delete if
invalid) rather than being silently ignored.

## Ordering guarantee

`do_timeline_switch` (cli/commands.rs) sequences: capture changes →
save shelf (atomic) → flush CAS (the shelf holds the only copies of
uncommitted content) → clear staging → **write journal** → HEAD rewrite →
materialize → restore target shelf → **clear journal**. A crash before the
journal write leaves HEAD and the worktree untouched (at worst a redundant
shelf, which is harmless); a crash after it is recoverable as above.

# Resolve Module (`resolve.rs`): settling what a fuse cannot

## Overview

`ivaldi fuse` is meant to finish. It merges everything that has one right
answer by itself:

1. **Per file, by content hash** — a file only one side changed, or both
   changed identically, never needs a look.
2. **Per line** — a file both sides changed in *different places* is merged
   line by line inside the engine. Two timelines editing the same file is not
   a conflict; editing the same *lines* is.

What is left is a **collision**: the same lines rewritten two ways, a binary
changed twice, an edit on one side of a deletion on the other. No algorithm
knows which was meant, so somebody is asked — but one collision at a time,
*before anything is written*, and in the same command:

```
$ ivaldi fuse main

src/render.rs — collision 1 of 1, around line 169
    ) -> (Vec<HuginnElement>, usize) {
  mine (gaming_changes):
  |     let include_cursor = pass != Pass::Screenshot && !state.pointer_locked();
  theirs (main):
  |     let include_cursor = pass != Pass::Screenshot && state.pointer_visible();
    let hidden = match pass {
  [m]ine  [t]heirs  [b]oth (mine, then theirs)  [e]dit  [q]uit — cancels the fuse; nothing has been changed > e
Settled 1 conflicted file(s).
[OK] Merge completed successfully!
  Merge seal: shallow-flower-hunts-dull (3c90b55d)
  Not what you wanted? 'ivaldi oops' undoes the whole fuse.
```

There are no conflict markers in the working files, no half-open merge, and
no `--continue`. The fuse either completes or changes nothing at all.

The same applies to `ivaldi sync` when local and remote have diverged — it is
a fuse of the two — and to the TUI's Fuse tab.

## Who answers

| Situation | Resolver |
|-----------|----------|
| `--prefer mine\|theirs\|both` | `PreferResolver`: every collision settled that way. Only collisions — each side's other edits to the same file still both land. `both` is refused for a file that cannot hold both (binary, modify/delete). |
| On a terminal | `PromptResolver`: asked per collision |
| Neither | **Refused**, listing each file and its collision count, with nothing changed |

Picking a side by rule is never the silent default: a rule-picked region can
compile and still be wrong. A script has to say `--prefer`.

`IVALDI_INTERACTIVE=1` (or `0`) overrides terminal detection, for a driver
that feeds answers on stdin.

## The answers

| Key | Effect |
|-----|--------|
| `m` | Keep my version of the region |
| `t` | Take theirs |
| `b` | Both: mine, then theirs |
| `e` | Open just this region in `$VISUAL` / `$EDITOR` (with three lines of context either side) and take what is written. For when the right answer is neither — typically both edits combined on one line. Leaving the markers in, or touching the context lines, is not taken as an answer and the question is asked again |
| `q` | Back out. End of input counts as `q`: a closed stdin is never read as consent |

Whole-file conflicts (binary; changed vs. deleted) offer `m` / `t` / `q`, with
each spelled out ("keep my changed file" / "delete it, as they did").

**Every question is asked before anything is written** — including the ones
about [carried](carry.md) uncommitted work. Whether that work collides with the
fuse depends on what the fused files turn out to be, so it is worked out in
memory (`fuse_op::carried_collisions`) once the first round of answers is in,
asked as a second round, and the answers replayed (`Questions::into_resolver`)
when the work is merged back on top. `q` therefore means the same thing at
every question: the fuse is cancelled and nothing has changed.

A replayed answer is only given back to the *same* collision it was given to.
If a file changes between asking and replaying, that file gets conflict
markers rather than an answer to a question nobody was asked. (The one place
a question can still come *after* a seal is recovering a fuse that was
interrupted mid-carry; its prompt says so.)

## Undo instead of abort

Every fuse is snapshotted first, so a wrong answer costs one command:
`ivaldi oops` takes the merge seal back off the timeline and restores the
files, and the fuse can be run again and answered differently. That is what
makes it reasonable to settle collisions in place rather than parking the
merge for careful hand-editing. See [snapshot.md](snapshot.md).

## Resolving by hand

`ivaldi fuse <source> --markers` is the explicit opt-in to the traditional
flow: conflict markers are written into the files, the fuse is left open, and
`fuse --continue` / `fuse --abort` (or a bare `ivaldi oops`) settle it. A
diverged `sync` that collides uses the same open-merge state.

## API

```rust
use ivaldi::resolve::{self, Labels, Prefer, PreferResolver};

let labels = Labels { mine: "main", theirs: "feature", on_quit: "changes nothing" };
let mut resolver = PreferResolver(Prefer::Theirs);
for conflict in &result.conflicts {
    match resolve::resolve_conflict(&store, conflict, labels, &mut resolver)? {
        Some(hash) => { merged.insert(conflict.path.clone(), hash); }
        None => { merged.remove(&conflict.path); }   // resolved as deleted
    }
}
```

`Resolver` is a trait (`region`, `whole_file`): a callback, for a front end
that can stop and wait. An event loop cannot, so `Questions` lays the same
questions out as a list instead — `get(i)`, `answer(i, …)`,
`next_unanswered(from)`, and `apply(&store, &mut merged_files)`, which refuses
while anything is unanswered. `ask(resolver, labels)` fills one in from a
blocking resolver; `from_merges` builds one for carried collisions. The TUI's
Fuse tab is built on it.

Editing a region is three separable steps — `edit_buffer`, `edit_text`,
`parse_edited` — because front ends differ in the middle one: a prompt just
runs the editor, a TUI has to give up the screen first.

The steps around the merge — snapshot, set-aside, merge seal with the source
as a parent, materialize, re-apply — live in `fuse_op` (`plan`, `set_aside`,
`seal`, `reapply`, `complete`, `abort`, `finish_interrupted`) and are shared by
every front end; only the asking differs. `fuse::Merge3` is the
structured merge underneath: `chunks` of `Clean` lines and `Collision`s, and
`render(&resolutions, …)`, which falls back to markers for any collision left
unanswered.

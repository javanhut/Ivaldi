# Undo and Pluck

Two commands apply an existing seal's delta to the current timeline head as
a **new** seal — history is never rewritten:

- `ivaldi undo <seal>` removes what `<seal>` changed.
- `ivaldi pluck <seal>` applies what `<seal>` changed (git users: this is
  `cherry-pick`, and that name works as an alias).

```bash
# Take back a bad seal without rewriting history
ivaldi undo swift-eagle-flies-high

# Bring one fix over from another timeline
ivaldi pluck gentle-otter-swims-fast
ivaldi cherry-pick gentle-otter-swims-fast   # same thing

# Custom message
ivaldi undo swift-eagle-flies-high -m "Back out the cache change"
```

## How it works

Both run the same three-way fuse used by `ivaldi fuse`, with the current
head as "ours":

| Command | base              | theirs            |
|---------|-------------------|-------------------|
| undo    | the seal's tree   | its parent's tree |
| pluck   | its parent's tree | the seal's tree   |

The merged tree is sealed as a plain (non-merge) seal and materialized to
the working directory.

- Undoing a timeline's **first** seal deletes the files it introduced.
- Plucking a seal whose changes are already in the head reports
  "no changes" and creates nothing.
- The default undo message is `Undo "<original first line>"` with a
  trailer naming the undone seal; pluck keeps the original message plus a
  `(plucked from …)` trailer.

## Conflicts

The fuse engine works at whole-file granularity. If a file touched by the
seal was also changed by later history (undo) or by your timeline
(pluck), the command **refuses and changes nothing** — it lists the
conflicting paths instead:

```
error: undo conflicts with other changes in:
  src/cache.rs
Resolve by editing the files manually and sealing, nothing was changed.
```

## Restrictions

- The staging area must be clean (seal or `ivaldi reset` first) so the
  materialized result can't clobber staged work.
- No merge may be in progress (`ivaldi fuse --continue` / `--abort` first).
- Merge seals cannot be undone or plucked yet (there is no way to choose
  which parent's side to keep).

## Related

- `ivaldi reseal` — redo the most recent seal (new message and/or fold in
  staged changes).
- `ivaldi rewind <seal>` — move the timeline head back to an earlier seal;
  add `--discard` to also rewrite your files.

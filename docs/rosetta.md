# Ivaldi ↔ Git Rosetta Stone

Ivaldi is a version control system, not a git port. The vocabulary is
different on purpose — names match what you're actually trying to do.
But if you already know git, you don't need to learn a new mental model
from scratch. Almost every command has a one-line translation.

This page is the translation table. Read it once and you'll be productive
in Ivaldi the same afternoon.

## The big picture

| Concept | Git | Ivaldi |
|---|---|---|
| Commit | commit | **seal** |
| Commit hash | `a1b2c3d` | `swift-eagle-flies-high-a1b2c3d4` (memorable name + short hash) |
| Branch | branch | **timeline** |
| Staging area | index | gathered set (no separate index file) |
| Stash | manual `git stash` | **automatic** on timeline switch ("auto-shelving") |
| Working tree scratch branch | worktree | **butterfly** (sandbox timeline w/ bidirectional parent sync) |
| Reflog | `git reflog` | append-only MMR — `travel --all` walks orphaned seals |
| Remote | remote | **portal** |
| Hash function | SHA-1 | BLAKE3 |

## Daily workflow

| You want to… | Git | Ivaldi |
|---|---|---|
| Start a new repo | `git init` | `ivaldi forge` |
| Stage a file | `git add f.txt` | `ivaldi gather f.txt` |
| Stage everything | `git add .` | `ivaldi gather .` |
| Commit | `git commit -m "msg"` | `ivaldi seal -m "msg"` |
| See what's changed | `git status` | `ivaldi status` |
| Where am I? | `git branch --show-current` + `git log -1` | `ivaldi whereami` (alias `wai`) |
| Show history | `git log` | `ivaldi log` |
| Compact history | `git log --oneline` | `ivaldi log --oneline` |
| See file diff | `git diff` | `ivaldi diff` |
| See staged diff | `git diff --staged` | `ivaldi diff --staged` |
| Diff between two refs | `git diff A B` | `ivaldi diff A B` |
| Unstage a file | `git restore --staged f.txt` | `ivaldi discard f.txt` |
| Discard local changes | `git reset --hard` | `ivaldi reverse --all` |
| Stage parts of a file | `git add -p` | `ivaldi gather -p` |
| Fix the last commit | `git commit --amend` | `ivaldi reseal` |
| Undo a commit safely | `git revert <sha>` | `ivaldi undo <seal>` |
| Copy one commit over | `git cherry-pick <sha>` | `ivaldi pluck <seal>` (alias `cherry-pick`) |
| Who wrote each line? | `git blame f.txt` | `ivaldi whodidit f.txt` (alias `wdi`) |
| Ignore a path | edit `.gitignore` | `ivaldi exclude pattern` (writes `.ivaldiignore`) |
| Skip a file temporarily | `git update-index --skip-worktree f` | `ivaldi skip f` (`ivaldi unskip f` to undo) |

## Branches (timelines)

| You want to… | Git | Ivaldi |
|---|---|---|
| List branches | `git branch` | `ivaldi timeline list` (alias `tl ls`) |
| New branch from current | `git switch -c feature` | `ivaldi timeline create feature` (alias `tl cr`) |
| Switch branches | `git switch main` | `ivaldi timeline switch main` (alias `tl sw`) |
| Rename current branch | `git branch -m new-name` | `ivaldi timeline rename new-name` |
| Delete a branch | `git branch -D feature` | `ivaldi timeline remove feature` (alias `tl rm`) |
| Merge into current | `git merge feature` | `ivaldi fuse feature` |
| Merge A into B | `git checkout B && git merge A` | `ivaldi fuse A to B` |
| Abort merge | `git merge --abort` | `ivaldi fuse --abort` |
| Continue merge | `git merge --continue` | `ivaldi fuse --continue` |
| Throwaway scratch branch | `git worktree add` or `git switch -c` | `ivaldi tl bf create scratch` (butterfly) |
| Push butterfly into parent | (n/a — manual rebase) | `ivaldi tl bf up` |
| Pull parent into butterfly | (n/a — manual merge) | `ivaldi tl bf down` |

## Rewriting history

| You want to… | Git | Ivaldi |
|---|---|---|
| Squash last 3 commits | `git rebase -i HEAD~3` | `ivaldi weld --last 3 -m "msg"` |
| Squash a range | `git rebase -i <base>` | `ivaldi weld <start> to <end> -m "msg"` |
| Amend the head commit | `git commit --amend` | `ivaldi reseal [msg]` |
| Move head, keep your files | `git reset --soft/--mixed <sha>` | `ivaldi rewind <seal>` |
| Hard-reset to a commit | `git reset --hard <sha>` | `ivaldi rewind <seal> --discard` (or `ivaldi travel` → **Overwrite**) |
| Branch off an old commit | `git switch -c new <sha>` | `ivaldi travel` → pick seal → **Diverge** |
| Browse old commits | `git log` then `git checkout <sha>` | `ivaldi travel` (interactive TUI) |
| Recover a "lost" commit | `git reflog` | `ivaldi travel --all` (walks every leaf in the MMR) |
| Search commits | `git log --grep=foo` | `ivaldi travel --search foo` |

> Note on safety: `weld` does not delete the original seals — they remain
> in the append-only MMR, just unreachable from the timeline head.
> `travel --all` is how you find them again. There is no GC running by
> default, so welded/overwritten seals are always recoverable until you
> explicitly run garbage collection.

## Remotes

Ivaldi auto-routes uploads/downloads based on the URL — same commands
whether you're talking to GitHub HTTPS, an SSH host, or another Ivaldi
peer.

| You want to… | Git | Ivaldi |
|---|---|---|
| Add a remote | `git remote add origin <url>` | `ivaldi portal add owner/repo` (or full URL) |
| List remotes | `git remote -v` | `ivaldi portal list` |
| Clone | `git clone <url>` | `ivaldi download <url>` |
| Push | `git push` | `ivaldi upload` |
| Force push | `git push --force` | `ivaldi upload --force` |
| Fetch all branches | `git fetch` | `ivaldi scout` |
| Fetch a branch | `git fetch origin feature` | `ivaldi harvest feature` |
| Pull | `git pull` | `ivaldi sync` |
| Auth with GitHub | (token in URL / SSH key) | `ivaldi auth login` (OAuth) |
| Auth with GitLab | (token in URL / SSH key) | `ivaldi auth login --gitlab` |

Portal URLs Ivaldi understands:

- `owner/repo` — GitHub HTTPS shorthand
- `git@host:team/repo.git` — any git host over SSH (GitHub, GitLab, Gitea, …)
- `ivaldi://host:9418` — Ivaldi peer-to-peer (no third party)

## Things that are different on purpose

A few places where the translation isn't 1:1 because Ivaldi rejects a
git design choice rather than reproducing it.

**No conflict markers in your files.** `git merge` writes `<<<<<<<` /
`=======` / `>>>>>>>` directly into your source. Ivaldi's `fuse`
auto-resolves with one of several strategies (`auto`, `ours`, `theirs`,
`union`, `base`); your working tree never contains a half-merged file.
If you want git's behavior, the strategies map to it — `--strategy
ours` and `--strategy theirs` are the two-sided choices.

**No detached HEAD.** Browsing history with `travel` doesn't put you
into a state where commits you make get garbage-collected. You either
**Diverge** (creates a new timeline rooted at that seal) or
**Overwrite** (moves the current timeline's head). Both are explicit.

**No separate stash.** `git stash` exists because switching branches
destroys uncommitted work. Ivaldi auto-shelves on switch — your
uncommitted changes follow you back when you return to the timeline.
There's nothing to stash because nothing is at risk.

**One verb per intent.** `git reset` is `--soft`, `--mixed`, `--hard`,
each doing wildly different things. Ivaldi splits these:
- Ungather a file: `ivaldi discard f.txt`
- Throw away local changes: `ivaldi reverse --all`
- Move the timeline head back: `ivaldi rewind <seal>` (add `--discard` to also rewrite your files)
- Redo the last seal: `ivaldi reseal`
- Combine commits: `ivaldi weld`

**Memorable names.** Every seal gets a deterministic four-word name like
`swift-eagle-flies-high-a1b2c3d4` derived from its hash. You can use
the seal name anywhere a hash works (`fuse`, `travel`, `weld`, `diff`).
You can also still use the short hash if you prefer.

## Things git has that Ivaldi (deliberately) doesn't

If you reach for one of these, here's the intended Ivaldi answer.

| Git feature | Ivaldi equivalent / why it's missing |
|---|---|
| `git worktree` | Use butterflies, or just clone twice. The "two checkouts at once" need is rare enough that a second clone is fine. |
| `git stash` | Auto-shelving on timeline switch. |
| `git rebase -i` (reorder/edit) | `weld` covers squash; `travel` covers reset. Reordering individual commits is intentionally not in v0.1 — the workflows it enables are usually a smell. |
| `git cherry-pick` | `ivaldi pluck <seal>` (the `cherry-pick` alias also works). See docs/undo.md. |
| `git submodule` | Supported (`src/submodule.rs`); same semantics as git. |
| `.git/hooks/*` | `.ivaldi/hooks/*` — same shape. |
| `git lfs` | `filechunk` handles large files via content-defined chunking; no separate tool. |

## Five-minute starter

```bash
# Init + identity
ivaldi forge
ivaldi config --set user.name "Your Name"
ivaldi config --set user.email "you@example.com"

# First seal
ivaldi gather .
ivaldi seal -m "Initial commit"

# Branch, edit, merge
ivaldi timeline create feature
echo "change" >> file.txt
ivaldi gather file.txt
ivaldi seal -m "Add feature"
ivaldi timeline switch main
ivaldi fuse feature

# Push to GitHub (already a git repo on the other end? fine — bridge handles it)
ivaldi portal add owner/repo
ivaldi auth login
ivaldi upload
```

That's the whole workflow. The git muscle memory carries over; the
names just got better.

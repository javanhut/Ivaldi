# TUI Module (`tui/`)

Interactive terminal UI components built with `ratatui` + `crossterm`.

## Components

### Travel (`tui/travel.rs`)

Interactive history browser with arrow key navigation. Walks the
timeline's full commit DAG (`prev_idx` + `merge_idxs`) by default, so
merge commits expose all reachable ancestors — not just the
first-parent chain.

```bash
ivaldi travel                    # browse current timeline (DAG walk)
ivaldi travel --search "auth"    # filter by message / author / seal name
ivaldi travel --all              # browse every leaf in the MMR — useful
                                 # for finding seals orphaned by `weld`
```

**Controls:**
| Key | Action |
|-----|--------|
| ↑/↓ | Navigate one seal at a time |
| **PgUp / PgDn** | Page up/down by one viewport |
| Home / End | Jump to first / last |
| 1-9 | Jump to specific seal |
| Enter | Select seal → choose action |
| q / Esc | Quit |

**Actions after selecting a seal:**
1. **Diverge** — Create a new timeline rooted at this seal (non-destructive).
   This is also how you recover seals that were orphaned by `weld` /
   destructive history rewrites: `travel --all`, find the seal, Diverge.
2. **Overwrite** — Reset current timeline to this seal (destructive,
   requires explicit "yes" confirmation).
3. **Cancel**

#### Layout & rendering

Each seal renders as a deterministic 3-line slot (header / message /
author·time) followed by a 1-row gutter. The viewport capacity is
recomputed every frame from the actual terminal height — no hard-coded
"~10 entries" assumption — so `↓` always advances by one *visible* seal.
Resizing the terminal is safe: the offset re-clamps each frame.

The header shows `showing N-M of TOTAL` so it's obvious how much of
your history is in view at any moment.

### Shift (`tui/shift.rs`)

Interactive commit range selection used as the no-args backend for
[`weld`](weld.md). Kept for backward-compatibility callers; new code
should invoke `weld` directly.

```bash
ivaldi weld                # interactive picker (this TUI)
ivaldi weld --last 3       # non-interactive, weld last 3
ivaldi weld START to END   # non-interactive range
```

**Interactive two-phase selection:**
1. Select START seal (oldest in range)
2. Select END seal (newest in range)
3. Review seals, enter message, confirm with "yes"

### Config form (`tui/config_form.rs`)

Full-screen editor for Ivaldi configuration, launched by `ivaldi config` with
no flags. Works both inside a repo (writes `.ivaldi/config`) and outside
(writes `~/.ivaldi/config`).

**Sections:** User, Appearance, Core, Remote (repo-local only).

**Controls:**
| Key | Action |
|-----|--------|
| ↑/↓ or j/k | Navigate fields |
| Enter | Edit focused text field (or toggle bool) |
| ←/→ or h/l | Toggle bool fields |
| Esc | Cancel edit, or exit without saving |
| `s` | Save and exit |
| `q` | Quit (prompts if modified) |

Validates `user.email` (`x@y.z`) and `portal.default` (must parse via
`parse_repo_spec`). See [config.md](config.md) for the full field list.

### Resolver (`tui/resolver.rs`)

Per-file conflict resolution during fuse operations.

**Controls:**
| Key | Action |
|-----|--------|
| ↑/↓ or 1-4 | Choose resolution |
| Enter | Confirm choice |
| a | Abort merge |
| q/Esc | Quit |

**Resolution choices:**
1. Keep OURS (target timeline)
2. Keep THEIRS (source timeline)
3. Keep BOTH (concatenate)
4. Skip this file

## Dashboard Tabs (`ivaldi tui`)

The TUI dashboard has 7 tabs, switchable with number keys or Tab/Shift+Tab:

| Key | Tab | View |
|-----|-----|------|
| 1 | Status | Repository status overview |
| 2 | Log | Commit history |
| 3 | Diff | Change comparison |
| 4 | Timelines | Timeline management |
| 5 | Remote | Remote operations (scout/harvest work without auth on public repos; upload/sync require auth) |
| 6 | Fuse | Merge timelines |
| 7 | Review | Local code review |

### Review Tab (`tui/views/review.rs`)

Local code review management with three sub-modes:

**List Mode** — Browse all reviews with status icons.

| Key | Action |
|-----|--------|
| j/k | Navigate up/down |
| Enter | Open review detail |
| r | Refresh list |

**Detail Mode** — View a single review with comments and verdicts.

| Key | Action |
|-----|--------|
| j/k | Scroll up/down |
| d | View diff between source/target |
| C | Add comment (opens dialog) |
| a | Approve review |
| x | Request changes |
| m | Merge (requires approval, asks for confirmation) |
| q | Close review (asks for confirmation) |
| Esc | Back to list |

**Diff Mode** — File-level changes between source and target timelines.

| Key | Action |
|-----|--------|
| j/k | Scroll up/down |
| g/G | Jump to top/bottom |
| Esc | Back to detail |

## Architecture

```
tui/mod.rs       — Terminal init/restore helpers
tui/app.rs       — Dashboard app loop, tab dispatch
tui/types.rs     — TabId, Action, AppContext
tui/theme.rs     — Color themes
tui/views/       — Tab view implementations (status, log, diff, timeline, remote, fuse, review)
tui/components/  — Reusable widgets (tab_bar, status_bar, file_list, diff_view, dialog)
tui/travel.rs      — History browser
tui/shift.rs       — Squash range selector
tui/resolver.rs    — Conflict resolver
tui/config_form.rs — Interactive config form
```

All TUI components use the same pattern:
1. `init_terminal()` — enter raw mode + alternate screen
2. Event loop: draw → read key → update state
3. `restore_terminal()` — restore normal terminal
4. Return action to CLI layer for execution

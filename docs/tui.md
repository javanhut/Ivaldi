# TUI Module (`tui/`)

Interactive terminal UI components built with `ratatui` + `crossterm`.

## Components

### Travel (`tui/travel.rs`)

Interactive history browser with arrow key navigation.

```bash
ivaldi travel                    # browse current timeline
ivaldi travel --search "auth"    # filter by keyword
```

**Controls:**
| Key | Action |
|-----|--------|
| ↑/↓ | Navigate through seals |
| Enter | Select seal → choose action |
| Home/End | Jump to first/last |
| 1-9 | Jump to specific seal |
| q/Esc | Quit |

**Actions after selecting a seal:**
1. **Diverge** — Create new timeline from this point (non-destructive)
2. **Overwrite** — Reset current timeline to this seal (destructive, requires "yes")
3. **Cancel**

### Shift (`tui/shift.rs`)

Interactive commit range selection for squashing.

```bash
ivaldi shift                # interactive mode
ivaldi shift --last 3       # non-interactive, squash last 3
```

**Interactive two-phase selection:**
1. Select START commit (oldest in range)
2. Select END commit (newest in range)
3. Review commits, enter message, confirm with "yes"

**Non-interactive `--last N`:**
- Shows commits to squash
- Creates squashed seal with combined message
- No TUI needed

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
| 5 | Remote | Remote operations |
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
tui/travel.rs    — History browser
tui/shift.rs     — Squash range selector
tui/resolver.rs  — Conflict resolver
```

All TUI components use the same pattern:
1. `init_terminal()` — enter raw mode + alternate screen
2. Event loop: draw → read key → update state
3. `restore_terminal()` — restore normal terminal
4. Return action to CLI layer for execution

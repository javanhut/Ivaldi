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

## Architecture

```
tui/mod.rs       — Terminal init/restore helpers
tui/travel.rs    — History browser
tui/shift.rs     — Squash range selector
tui/resolver.rs  — Conflict resolver
```

All TUI components use the same pattern:
1. `init_terminal()` — enter raw mode + alternate screen
2. Event loop: draw → read key → update state
3. `restore_terminal()` — restore normal terminal
4. Return action to CLI layer for execution

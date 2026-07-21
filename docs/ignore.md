# Ignore Module (`ignore.rs`)

Pattern matching for `.ivaldiignore` files.

## Overview

Controls which files are excluded from version control. Supports glob patterns, directory matching, recursive globs, and automatic security exclusions.

## Pattern Syntax

| Pattern | Meaning |
|---------|---------|
| `*.log` | Glob: matches any `.log` file (basename) |
| `build/` | Directory: matches `build/` and everything under it |
| `**/*.tmp` | Recursive glob: matches `.tmp` files at any depth |
| `node_modules` | Literal: exact name match |
| `file[0-9].txt` | Character class: `file0.txt` through `file9.txt` |
| `test?.txt` | Single char wildcard: `test1.txt`, `testA.txt` |
| `# comment` | Comment line (ignored) |

## Auto-Excluded (Security)

Always excluded regardless of `.ivaldiignore`:
- `.env`, `.env.*` — environment variable files
- `.venv`, `.venv/` — Python virtual environments

## Dotfiles and Security Blocks

Dotfile handling is separate from `.ivaldiignore` patterns and often
surprises users coming from git:

- **Dotfiles are skipped by `gather .`** even when no ignore pattern
  matches them. The skipped paths are reported on stderr so they are not
  silently lost.
- **Explicitly gathering a dotfile** (`ivaldi gather .editorconfig`)
  prompts `y/N` per file. Confirmed paths are remembered in
  `.ivaldi/dotfile-allowlist`, so each dotfile is asked about only once.
- **`ivaldi gather --allow-all`** confirms all pending dotfiles in one go.
- **Security-blocked files cannot be staged at all.** `.env`, `.env.*`,
  and `.venv` are hard-blocked; neither the allowlist nor `--allow-all`
  overrides this. Staging one is an error, not a prompt.
- Adding `.*` to `.ivaldiignore` is unnecessary — the dotfile gate already
  covers hidden files; the ignore file is for non-hidden paths like
  `build/` or `*.log` (managed with `ivaldi exclude <pattern>`).

## Temporary Exclusion: `ivaldi skip`

`.ivaldiignore` is the committed, pattern-based mechanism — right for things
that should never be tracked by anyone. For the opposite case — a tracked
file you changed locally but don't want to seal or push right now (a
regenerated lockfile, debug or test output) — use `ivaldi skip` instead of
editing the ignore file back and forth:

- `ivaldi skip <path>` excludes the path from `gather` (bulk and explicit)
  and `gather -p`, and hides it from `status`. A skipped tracked file is
  never staged as a deletion either. Skipping a directory excludes
  everything beneath it.
- `ivaldi skip --list` shows the current set; `ivaldi unskip <path>` removes
  entries.
- The set lives in `.ivaldi/skipped` — repo-local, never committed, so it
  never reaches remotes or clones.
- Skipping a path that is **already staged** does not unstage it — run
  `ivaldi discard <path>` first if it is already gathered.
- Autoshelve still preserves a skipped file's uncommitted edits across
  timeline switches; skip only affects staging.

## Built-In Defaults

Always excluded (VCS/tool directories):
- `.git/`, `.svn/`, `.hg/`, `.fossil/`, `.claude/`

## Special Rules

- `.ivaldiignore` itself is **never** ignored
- Directory patterns enable early pruning during `scan` (skip traversal entirely)

## Usage

```rust
use ivaldi::ignore::{PatternCache, load_pattern_cache};

// From patterns
let cache = PatternCache::new(&["*.log", "build/", "**/*.tmp"]);
assert!(cache.is_ignored("error.log"));
assert!(cache.is_ignored("build/output.js"));
assert!(!cache.is_ignored("README.md"));

// Directory pruning
assert!(cache.is_dir_ignored("build"));

// From .ivaldiignore file
let cache = load_pattern_cache(work_dir);
```

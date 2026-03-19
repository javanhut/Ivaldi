# Review Module (`review.rs`)

Local code review system for Ivaldi VCS.

## Overview

Provides a fully offline review workflow: create reviews (similar to pull requests), comment on code, approve or reject, and merge — all without requiring a remote. Reviews link a source timeline to a target timeline and track comments, verdicts, and merge status.

**Key difference from Git PRs**: Reviews are local-first. No server needed. The review state lives in `.ivaldi/reviews/` as human-readable JSON files.

## Data Model

### ReviewStatus

| Status | Symbol | Description |
|--------|--------|-------------|
| `Open` | `O` | Review is active and awaiting feedback |
| `Approved` | `+` | Review has been approved, eligible for merge |
| `ChangesRequested` | `!` | Reviewer requested changes |
| `Merged` | `M` | Review was merged into the target timeline |
| `Closed` | `X` | Review was closed without merging |

### Review

```rust
pub struct Review {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub source_timeline: String,     // Timeline being reviewed
    pub target_timeline: String,     // Timeline to merge into
    pub source_head_seal: String,    // Head seal at creation time
    pub target_head_seal: String,    // Head seal at creation time
    pub author: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: ReviewStatus,
    pub comments: Vec<ReviewComment>,
    pub verdicts: Vec<ReviewVerdict>,
    pub fuse_strategy: String,       // Merge strategy (auto, ours, theirs, union, base)
    pub merge_seal: Option<String>,  // Seal name of merge commit (set after merge)
}
```

### ReviewComment

Comments can be attached to a specific file and line, or be file-level. Flat threading is supported via `reply_to`.

```rust
pub struct ReviewComment {
    pub id: u64,
    pub path: String,
    pub line: Option<u64>,       // None = file-level comment
    pub author: String,
    pub time_unix: i64,
    pub body: String,
    pub reply_to: Option<u64>,   // Flat threading
}
```

### ReviewVerdict

A verdict records an approval or change request.

```rust
pub struct ReviewVerdict {
    pub author: String,
    pub time_unix: i64,
    pub status: ReviewStatus,    // Approved or ChangesRequested
    pub body: String,
}
```

## Storage

Reviews are stored as individual JSON files in `.ivaldi/reviews/`:

```
.ivaldi/reviews/
  1.json
  2.json
  3.json
```

The next ID counter is persisted in the META table (`"review.next_id"`).

This approach is consistent with the `MergeState` JSON persistence pattern in `repo.rs` and keeps reviews human-inspectable, variable-size, and future sync-friendly.

## Operations

| Operation | Function | Description |
|-----------|----------|-------------|
| Create | `create_review()` | Validates timelines exist, assigns ID, records head seals |
| List | `list_reviews()` | Reads all reviews, optional status filter, sorted by updated_at |
| Comment | `add_comment()` | Appends comment, updates `updated_at` |
| Approve | `submit_verdict(Approved)` | Sets status to Approved |
| Request Changes | `submit_verdict(ChangesRequested)` | Sets status to ChangesRequested |
| Merge | `merge_review()` | **Requires Approved status.** Uses `FuseEngine::fuse()`, commits to target timeline, sets Merged |
| Close | `close_review()` | Sets Closed without merge |
| Reopen | `reopen_review()` | Sets back to Open (only if Closed) |
| Diff | `review_diff()` | Calls `diff_trees()` between source/target head trees |

## Merge Policy

**Approval required before merge.** Calling `merge_review()` on a non-approved review returns an error. This enforces that at least one verdict of `Approved` exists before code is merged.

The merge uses `FuseEngine::fuse()` from `fuse.rs` with the strategy stored in the review (default: `auto`). The strategy can be overridden at merge time via the CLI `--strategy` flag.

## CLI Commands

```bash
# Create a review
ivaldi review create --source feature --target main --title "Add login"

# List reviews (active by default, --all for everything)
ivaldi review list
ivaldi review list --status open
ivaldi review list --all

# Show review details
ivaldi review show 1

# View diff between source and target
ivaldi review diff 1
ivaldi review diff 1 --stat

# Comment on code
ivaldi review comment 1 --file src/main.rs --line 42 --body "Fix this"

# Approve or request changes
ivaldi review approve 1 --body "LGTM"
ivaldi review request-changes 1 --body "Needs error handling"

# Merge (requires approval)
ivaldi review merge 1
ivaldi review merge 1 --strategy theirs

# Close/reopen
ivaldi review close 1
ivaldi review reopen 1
```

**Alias**: `ivaldi rv` is equivalent to `ivaldi review`.

## TUI Tab (Tab 7)

The review tab in the TUI dashboard (press `7`) has three sub-modes:

### List Mode

Displays all reviews with status icons and cursor navigation.

| Key | Action |
|-----|--------|
| j/k | Navigate up/down |
| Enter | Open review detail |
| r | Refresh |

### Detail Mode

Shows a single review: title, status, comments, verdict history.

| Key | Action |
|-----|--------|
| j/k | Scroll up/down |
| d | View diff |
| C | Add comment (opens dialog) |
| a | Approve review |
| x | Request changes |
| m | Merge (requires approval, confirms) |
| q | Close review (confirms) |
| Esc | Back to list |

### Diff Mode

Shows file-level changes between source and target timelines.

| Key | Action |
|-----|--------|
| j/k | Scroll up/down |
| g/G | Jump to top/bottom |
| Esc | Back to detail |

## Usage Example

```rust
use ivaldi::review::{self, ReviewFilter, ReviewStatus};
use ivaldi::repo::Repo;

let repo = Repo::open(work_dir)?;

// Create
let review = review::create_review(&repo, "Add login", "Details...", "feature", "main", "auto")?;

// Comment
review::add_comment(&repo, review.id, "src/auth.rs", Some(42), "Add error handling", None)?;

// Approve
review::submit_verdict(&repo, review.id, ReviewStatus::Approved, "LGTM")?;

// Merge
let merged = review::merge_review(&mut repo, review.id)?;
println!("Merged: {}", merged.merge_seal.unwrap());
```

## Design Decisions

- **JSON file storage**: One file per review for human readability and easy inspection. Consistent with `MergeState` pattern.
- **ID counter in META table**: Uses redb for atomic ID generation, avoiding collisions.
- **Approval gate**: Merge requires explicit approval to enforce review discipline even when working solo.
- **Flat threading**: Comments use `reply_to` for simple threading without nested structures.
- **Status machine**: Open -> Approved/ChangesRequested -> Merged (terminal) or Closed (reopenable).
- **Reuses FuseEngine**: Merge uses the same three-way merge engine as `ivaldi fuse`, ensuring consistent behavior.
- **Diff uses diff_trees**: Same tree comparison as `ivaldi diff`, showing file-level changes.

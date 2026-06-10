//! Line-level diff rendering for the CLI.
//!
//! `fsmerkle::diff_trees` returns file-level changes only (added/deleted/modified).
//! This module loads blob content and produces unified-diff-style output for the
//! CLI. The TUI has its own LCS implementation tailored to its `DiffLine` widget;
//! we keep them separate so neither has to depend on the other.

use crate::color;
use crate::fsmerkle::FsStore;
use crate::hash::B3Hash;

/// Detect binary content by looking for NUL bytes in the first 8 KiB.
///
/// Matches what most diff tools do (git, diff, hg). Cheap and reasonably
/// accurate for source code; a UTF-16 file would still be flagged as binary,
/// which is the desired outcome for textual diff display.
pub fn is_binary(content: &[u8]) -> bool {
    let limit = content.len().min(8192);
    content[..limit].contains(&0)
}

/// Print line-level diff for a Modified file.
///
/// Loads both blobs from CAS, falls back to "Binary files differ" if either
/// side is binary, otherwise prints a unified-diff style block with `-`/`+`
/// markers and limited context.
pub fn print_blob_diff(store: &FsStore<'_>, old_hash: B3Hash, new_hash: B3Hash) {
    let old = match store.load_blob(old_hash) {
        Ok((_, c)) => c,
        Err(_) => return,
    };
    let new = match store.load_blob(new_hash) {
        Ok((_, c)) => c,
        Err(_) => return,
    };

    if is_binary(&old) || is_binary(&new) {
        println!("    {}", color::dim("(binary file — content not shown)"));
        return;
    }

    let old_text = String::from_utf8_lossy(&old);
    let new_text = String::from_utf8_lossy(&new);
    print_unified_lines(&old_text, &new_text);
}

/// Print all lines of a blob with a leading `+` marker (Added file).
pub fn print_blob_as_added(store: &FsStore<'_>, hash: B3Hash) {
    let content = match store.load_blob(hash) {
        Ok((_, c)) => c,
        Err(_) => return,
    };
    if is_binary(&content) {
        println!("    {}", color::dim("(binary file — content not shown)"));
        return;
    }
    for line in String::from_utf8_lossy(&content).lines() {
        println!("    {}", color::bold_green(&format!("+{}", line)));
    }
}

/// Print all lines of a blob with a leading `-` marker (Deleted file).
pub fn print_blob_as_deleted(store: &FsStore<'_>, hash: B3Hash) {
    let content = match store.load_blob(hash) {
        Ok((_, c)) => c,
        Err(_) => return,
    };
    if is_binary(&content) {
        println!("    {}", color::dim("(binary file — content not shown)"));
        return;
    }
    for line in String::from_utf8_lossy(&content).lines() {
        println!("    {}", color::bold_red(&format!("-{}", line)));
    }
}

/// LCS-based diff op: each entry is one source-side or new-side line.
#[derive(Debug, Clone)]
pub(crate) enum LineOp {
    Context(String),
    Add(String),
    Remove(String),
}

/// One displayable/selectable hunk: a run of changes plus surrounding
/// context, expressed as a range of indices into the global ops vec.
/// Hunks produced by [`compute_hunks`] never overlap.
#[derive(Debug, Clone)]
pub(crate) struct Hunk {
    pub ops_range: std::ops::Range<usize>,
}

/// Group changed ops into hunks: each run of changes whose gaps are within
/// `2 * context` is merged, then padded with up to `context` lines of
/// surrounding context.
pub(crate) fn compute_hunks(ops: &[LineOp], context: usize) -> Vec<Hunk> {
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| match op {
            LineOp::Add(_) | LineOp::Remove(_) => Some(i),
            LineOp::Context(_) => None,
        })
        .collect();

    let mut hunks = Vec::new();
    let mut i = 0;
    while i < changed.len() {
        let start = changed[i].saturating_sub(context);
        let mut j = i;
        while j + 1 < changed.len() && changed[j + 1] <= changed[j] + 2 * context {
            j += 1;
        }
        let end = (changed[j] + context + 1).min(ops.len());
        hunks.push(Hunk {
            ops_range: start..end,
        });
        i = j + 1;
    }
    hunks
}

/// Reconstruct file content with only the `selected` hunks applied.
///
/// Walks the global ops vec: context lines always pass through; removals
/// pass through (as the old line) unless their hunk is selected; additions
/// appear only when their hunk is selected. Ops outside any hunk are
/// context by construction.
///
/// Trailing newline: `str::lines()` drops the final terminator, so it is
/// re-attached from whichever side "owns" the end of the file — `new` when
/// a selected hunk reaches EOF, `old` otherwise. (A diff consisting solely
/// of a trailing-newline change produces no line ops and is not selectable;
/// known limitation.)
pub(crate) fn apply_selected_hunks(
    old: &str,
    new: &str,
    ops: &[LineOp],
    hunks: &[Hunk],
    selected: &[bool],
) -> String {
    debug_assert_eq!(hunks.len(), selected.len());

    let mut lines: Vec<&str> = Vec::new();
    let mut hunk_idx = 0;
    for (i, op) in ops.iter().enumerate() {
        while hunk_idx < hunks.len() && hunks[hunk_idx].ops_range.end <= i {
            hunk_idx += 1;
        }
        let in_selected =
            hunk_idx < hunks.len() && hunks[hunk_idx].ops_range.contains(&i) && selected[hunk_idx];
        match op {
            LineOp::Context(s) => lines.push(s),
            LineOp::Add(s) => {
                if in_selected {
                    lines.push(s);
                }
            }
            LineOp::Remove(s) => {
                if !in_selected {
                    lines.push(s);
                }
            }
        }
    }

    let eof_owned_by_new = match (hunks.last(), selected.last()) {
        (Some(h), Some(&sel)) => sel && h.ops_range.end >= ops.len(),
        _ => false,
    };
    let ends_with_newline = if eof_owned_by_new {
        new.ends_with('\n')
    } else {
        old.ends_with('\n')
    };

    let mut out = lines.join("\n");
    if ends_with_newline && !out.is_empty() {
        out.push('\n');
    }
    out
}

pub(crate) fn compute_ops(old: &str, new: &str) -> Vec<LineOp> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let m = old_lines.len();
    let n = new_lines.len();

    // LCS DP table
    let mut dp = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if old_lines[i - 1] == new_lines[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    // Backtrack
    let mut i = m;
    let mut j = n;
    let mut ops: Vec<LineOp> = Vec::new();
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old_lines[i - 1] == new_lines[j - 1] {
            ops.push(LineOp::Context(old_lines[i - 1].to_string()));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            ops.push(LineOp::Add(new_lines[j - 1].to_string()));
            j -= 1;
        } else {
            ops.push(LineOp::Remove(old_lines[i - 1].to_string()));
            i -= 1;
        }
    }
    ops.reverse();
    ops
}

/// Print a unified-diff-style block.
///
/// Shows up to 3 lines of leading/trailing context around each run of
/// changes. We don't emit `@@` hunk headers since the CLI's two-target
/// output already prints the file path and these aren't being applied as
/// patches.
fn print_unified_lines(old: &str, new: &str) {
    const CONTEXT: usize = 3;
    let ops = compute_ops(old, new);
    let hunks = compute_hunks(&ops, CONTEXT);

    let mut printed_to: Option<usize> = None;
    for hunk in &hunks {
        let start = hunk.ops_range.start;
        let end = hunk.ops_range.end;

        let block_start = match printed_to {
            Some(p) if p >= start => p,
            _ => start,
        };

        if let Some(p) = printed_to
            && block_start > p
        {
            println!("    {}", color::dim("..."));
        }

        for op in &ops[block_start..end] {
            match op {
                LineOp::Context(s) => println!("    {}", color::dim(&format!(" {}", s))),
                LineOp::Add(s) => println!("    {}", color::bold_green(&format!("+{}", s))),
                LineOp::Remove(s) => println!("    {}", color::bold_red(&format!("-{}", s))),
            }
        }
        printed_to = Some(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_detection_finds_nul() {
        assert!(is_binary(b"hello\x00world"));
        assert!(!is_binary(b"plain text\nlines"));
    }

    #[test]
    fn ops_for_pure_addition() {
        let ops = compute_ops("a\nb", "a\nb\nc");
        let adds = ops.iter().filter(|o| matches!(o, LineOp::Add(_))).count();
        let removes = ops
            .iter()
            .filter(|o| matches!(o, LineOp::Remove(_)))
            .count();
        assert_eq!(adds, 1);
        assert_eq!(removes, 0);
    }

    #[test]
    fn ops_for_pure_removal() {
        let ops = compute_ops("a\nb\nc", "a\nc");
        let adds = ops.iter().filter(|o| matches!(o, LineOp::Add(_))).count();
        let removes = ops
            .iter()
            .filter(|o| matches!(o, LineOp::Remove(_)))
            .count();
        assert_eq!(adds, 0);
        assert_eq!(removes, 1);
    }

    #[test]
    fn ops_for_modify() {
        let ops = compute_ops("hello\nworld", "hello\nrust");
        // 1 context (hello), 1 remove (world), 1 add (rust)
        let adds = ops.iter().filter(|o| matches!(o, LineOp::Add(_))).count();
        let removes = ops
            .iter()
            .filter(|o| matches!(o, LineOp::Remove(_)))
            .count();
        assert_eq!(adds, 1);
        assert_eq!(removes, 1);
    }

    // ---- Hunk machinery ----

    /// Two changes far apart in a sea of context: lines 2 and 12 of 14.
    fn two_hunk_input() -> (String, String) {
        let old: Vec<String> = (1..=14).map(|i| format!("line {}", i)).collect();
        let mut new = old.clone();
        new[1] = "line 2 CHANGED".into();
        new[11] = "line 12 CHANGED".into();
        (old.join("\n") + "\n", new.join("\n") + "\n")
    }

    fn apply(old: &str, new: &str, pick: &dyn Fn(usize) -> bool) -> String {
        let ops = compute_ops(old, new);
        let hunks = compute_hunks(&ops, 3);
        let selected: Vec<bool> = (0..hunks.len()).map(pick).collect();
        apply_selected_hunks(old, new, &ops, &hunks, &selected)
    }

    #[test]
    fn hunks_split_distant_changes_and_merge_close_ones() {
        let (old, new) = two_hunk_input();
        let ops = compute_ops(&old, &new);
        assert_eq!(compute_hunks(&ops, 3).len(), 2);

        // Changes on adjacent lines merge into one hunk.
        let ops2 = compute_ops("a\nb\nc\nd", "a\nB\nC\nd");
        assert_eq!(compute_hunks(&ops2, 3).len(), 1);
    }

    #[test]
    fn select_none_returns_old() {
        let (old, new) = two_hunk_input();
        assert_eq!(apply(&old, &new, &|_| false), old);
    }

    #[test]
    fn select_all_returns_new() {
        let (old, new) = two_hunk_input();
        assert_eq!(apply(&old, &new, &|_| true), new);
    }

    #[test]
    fn select_first_hunk_only() {
        let (old, new) = two_hunk_input();
        let result = apply(&old, &new, &|i| i == 0);
        assert!(result.contains("line 2 CHANGED"));
        assert!(result.contains("line 12\n"));
        assert!(!result.contains("line 12 CHANGED"));
    }

    #[test]
    fn select_second_hunk_only() {
        let (old, new) = two_hunk_input();
        let result = apply(&old, &new, &|i| i == 1);
        assert!(result.contains("line 2\n"));
        assert!(!result.contains("line 2 CHANGED"));
        assert!(result.contains("line 12 CHANGED"));
    }

    #[test]
    fn trailing_newline_matrix() {
        // Change at EOF, new side drops the trailing newline.
        let old = "a\nb\nlast\n";
        let new = "a\nb\nlast changed";
        assert_eq!(apply(old, new, &|_| true), new);
        assert_eq!(apply(old, new, &|_| false), old);

        // Change at EOF, new side gains a trailing newline.
        let old2 = "a\nb\nlast";
        let new2 = "a\nb\nlast changed\n";
        assert_eq!(apply(old2, new2, &|_| true), new2);
        assert_eq!(apply(old2, new2, &|_| false), old2);

        // Change far from EOF: terminator stays old's either way.
        let old3 = "first\nx\nx\nx\nx\nx\nx\nend\n";
        let new3 = "FIRST\nx\nx\nx\nx\nx\nx\nend\n";
        assert_eq!(apply(old3, new3, &|_| true), new3);
        assert_eq!(apply(old3, new3, &|_| false), old3);
    }

    #[test]
    fn apply_to_empty_old_builds_new_file() {
        let old = "";
        let new = "fresh\ncontent\n";
        assert_eq!(apply(old, new, &|_| true), new);
        assert_eq!(apply(old, new, &|_| false), old);
    }
}

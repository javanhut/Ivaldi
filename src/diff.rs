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
enum LineOp {
    Context(String),
    Add(String),
    Remove(String),
}

fn compute_ops(old: &str, new: &str) -> Vec<LineOp> {
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

    // Find indices of changed ops, then emit each run of changes plus
    // up to CONTEXT context lines on each side, skipping unrelated context
    // in between.
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter_map(|(i, op)| match op {
            LineOp::Add(_) | LineOp::Remove(_) => Some(i),
            LineOp::Context(_) => None,
        })
        .collect();

    if changed.is_empty() {
        return;
    }

    let mut printed_to: Option<usize> = None;
    let mut i = 0;
    while i < changed.len() {
        let start = changed[i].saturating_sub(CONTEXT);
        let mut j = i;
        while j + 1 < changed.len() && changed[j + 1] <= changed[j] + 2 * CONTEXT {
            j += 1;
        }
        let end = (changed[j] + CONTEXT + 1).min(ops.len());

        let block_start = match printed_to {
            Some(p) if p >= start => p,
            _ => start,
        };

        if let Some(p) = printed_to {
            if block_start > p {
                println!("    {}", color::dim("..."));
            }
        }

        for op in &ops[block_start..end] {
            match op {
                LineOp::Context(s) => println!("    {}", color::dim(&format!(" {}", s))),
                LineOp::Add(s) => println!("    {}", color::bold_green(&format!("+{}", s))),
                LineOp::Remove(s) => println!("    {}", color::bold_red(&format!("-{}", s))),
            }
        }
        printed_to = Some(end);
        i = j + 1;
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
        let adds = ops
            .iter()
            .filter(|o| matches!(o, LineOp::Add(_)))
            .count();
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
        let adds = ops
            .iter()
            .filter(|o| matches!(o, LineOp::Add(_)))
            .count();
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
        let adds = ops
            .iter()
            .filter(|o| matches!(o, LineOp::Add(_)))
            .count();
        let removes = ops
            .iter()
            .filter(|o| matches!(o, LineOp::Remove(_)))
            .count();
        assert_eq!(adds, 1);
        assert_eq!(removes, 1);
    }
}

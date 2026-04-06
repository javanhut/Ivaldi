//! Scrollable diff renderer with syntax coloring.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::theme::Theme;

/// Type of a diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Header,
    Context,
    Add,
    Remove,
}

/// A single line in the diff view.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// Scrollable diff view component.
pub struct DiffViewWidget {
    pub lines: Vec<DiffLine>,
    pub offset: usize,
    pub file_boundaries: Vec<usize>,
}

impl DiffViewWidget {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            offset: 0,
            file_boundaries: Vec::new(),
        }
    }

    pub fn set_lines(&mut self, lines: Vec<DiffLine>) {
        // Record where file headers are for next/prev file navigation
        self.file_boundaries = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind == DiffLineKind::Header)
            .map(|(i, _)| i)
            .collect();
        self.lines = lines;
        self.offset = 0;
    }

    pub fn scroll_down(&mut self, n: usize) {
        let max = self.lines.len().saturating_sub(1);
        self.offset = (self.offset + n).min(max);
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.offset = self.offset.saturating_sub(n);
    }

    pub fn scroll_top(&mut self) {
        self.offset = 0;
    }

    pub fn scroll_bottom(&mut self) {
        self.offset = self.lines.len().saturating_sub(1);
    }

    pub fn next_file(&mut self) {
        if let Some(&boundary) = self.file_boundaries.iter().find(|&&b| b > self.offset) {
            self.offset = boundary;
        }
    }

    pub fn prev_file(&mut self) {
        if let Some(&boundary) = self
            .file_boundaries
            .iter()
            .rev()
            .find(|&&b| b < self.offset)
        {
            self.offset = boundary;
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        self.scroll_down(page_size);
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.scroll_up(page_size);
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let inner_height = area.height.saturating_sub(2) as usize;

        let text_lines: Vec<Line> = self
            .lines
            .iter()
            .skip(self.offset)
            .take(inner_height)
            .map(|dl| {
                let style = match dl.kind {
                    DiffLineKind::Header => theme.diff_header,
                    DiffLineKind::Context => theme.diff_context,
                    DiffLineKind::Add => theme.diff_add,
                    DiffLineKind::Remove => theme.diff_remove,
                };
                Line::from(Span::styled(&dl.text, style))
            })
            .collect();

        let scroll_info = if self.lines.is_empty() {
            String::from("empty")
        } else {
            format!("{}/{}", self.offset + 1, self.lines.len())
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Diff ", theme.title))
            .title_bottom(Span::styled(format!(" {} ", scroll_info), theme.dim));

        let para = Paragraph::new(text_lines).block(block);
        frame.render_widget(para, area);
    }
}

/// Compute a simple line-level diff between two texts using LCS.
pub fn compute_line_diff(old: &str, new: &str, file_path: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut result = Vec::new();
    result.push(DiffLine {
        kind: DiffLineKind::Header,
        text: format!("--- a/{}", file_path),
    });
    result.push(DiffLine {
        kind: DiffLineKind::Header,
        text: format!("+++ b/{}", file_path),
    });

    // Simple LCS-based diff
    let m = old_lines.len();
    let n = new_lines.len();

    // Build LCS table
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

    // Backtrack to produce diff
    let mut i = m;
    let mut j = n;
    let mut ops: Vec<DiffLine> = Vec::new();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old_lines[i - 1] == new_lines[j - 1] {
            ops.push(DiffLine {
                kind: DiffLineKind::Context,
                text: format!(" {}", old_lines[i - 1]),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            ops.push(DiffLine {
                kind: DiffLineKind::Add,
                text: format!("+{}", new_lines[j - 1]),
            });
            j -= 1;
        } else {
            ops.push(DiffLine {
                kind: DiffLineKind::Remove,
                text: format!("-{}", old_lines[i - 1]),
            });
            i -= 1;
        }
    }

    ops.reverse();
    result.extend(ops);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_diff_identical() {
        let lines = compute_line_diff("hello\nworld", "hello\nworld", "test.txt");
        // Header + 2 context lines
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].kind, DiffLineKind::Header);
        assert_eq!(lines[2].kind, DiffLineKind::Context);
    }

    #[test]
    fn compute_diff_addition() {
        let lines = compute_line_diff("a", "a\nb", "test.txt");
        let adds: Vec<_> = lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Add)
            .collect();
        assert_eq!(adds.len(), 1);
        assert!(adds[0].text.contains('b'));
    }

    #[test]
    fn compute_diff_removal() {
        let lines = compute_line_diff("a\nb", "a", "test.txt");
        let removes: Vec<_> = lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Remove)
            .collect();
        assert_eq!(removes.len(), 1);
        assert!(removes[0].text.contains('b'));
    }

    #[test]
    fn scroll_bounds() {
        let mut view = DiffViewWidget::new();
        view.set_lines(vec![
            DiffLine {
                kind: DiffLineKind::Context,
                text: "a".into(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                text: "b".into(),
            },
        ]);
        view.scroll_down(100);
        assert_eq!(view.offset, 1);
        view.scroll_up(100);
        assert_eq!(view.offset, 0);
    }

    #[test]
    fn file_boundaries() {
        let mut view = DiffViewWidget::new();
        view.set_lines(vec![
            DiffLine {
                kind: DiffLineKind::Header,
                text: "--- file1".into(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                text: "a".into(),
            },
            DiffLine {
                kind: DiffLineKind::Header,
                text: "--- file2".into(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                text: "b".into(),
            },
        ]);
        assert_eq!(view.file_boundaries, vec![0, 2]);
        view.next_file();
        assert_eq!(view.offset, 2);
    }
}

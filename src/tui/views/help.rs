//! Help overlay — global keybindings plus the active tab's own keys.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::theme::Theme;
use crate::tui::types::TabId;

/// Render the help overlay centered on screen.
///
/// `tab_help` is the active tab's `short_help()` string — typically
/// space-separated `key:label` pairs (e.g. `"s:scout u:upload p:portal"`).
/// We split those into a per-tab section appended below the global keys.
pub fn render_help(frame: &mut Frame, theme: &Theme, active_tab: TabId, tab_help: &str) {
    let last_tab = TabId::ALL.len();
    let tab_range_label = format!("1-{}", last_tab.min(9));

    let mut entries: Vec<(String, String)> = vec![
        (tab_range_label, "Jump to tab".into()),
        ("Tab".into(), "Next tab".into()),
        ("Shift+Tab".into(), "Previous tab".into()),
        ("?".into(), "Toggle this help".into()),
        ("Esc".into(), "Close dialog/help".into()),
        ("q / Ctrl+C".into(), "Quit".into()),
        (String::new(), String::new()),
        ("j / Down".into(), "Move down".into()),
        ("k / Up".into(), "Move up".into()),
        ("g".into(), "Go to top".into()),
        ("G".into(), "Go to bottom".into()),
        ("r".into(), "Refresh data".into()),
    ];

    let tab_entries = parse_tab_help(tab_help);
    if !tab_entries.is_empty() {
        entries.push((String::new(), String::new()));
        entries.push((String::new(), format!("— {} tab —", active_tab.label())));
        entries.extend(tab_entries);
    }

    let screen = frame.area();
    let width = 44u16.min(screen.width.saturating_sub(4));
    let height = (entries.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let x = (screen.width.saturating_sub(width)) / 2;
    let y = (screen.height.saturating_sub(height)) / 2;
    let area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.brand)
        .title(Span::styled(" Help ", theme.title));

    let lines: Vec<Line> = entries
        .iter()
        .map(|(key, desc)| {
            if key.is_empty() && desc.is_empty() {
                Line::from("")
            } else if key.is_empty() {
                Line::from(Span::styled(desc.clone(), theme.dim))
            } else {
                Line::from(vec![
                    Span::styled(format!("{:>14}  ", key), theme.help_key),
                    Span::styled(desc.clone(), theme.help_desc),
                ])
            }
        })
        .collect();

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}

/// Split a `short_help()` string like `"s:scout u:upload p:portal"` into
/// (key, label) pairs. Tolerates `Enter:harvest`-style multi-char keys and
/// the trailing colon-less labels some tabs use as a separator.
fn parse_tab_help(s: &str) -> Vec<(String, String)> {
    s.split_whitespace()
        .filter_map(|tok| {
            // Find the first ':' (so that keys like `Ctrl+C:quit` still work).
            let mut parts = tok.splitn(2, ':');
            let key = parts.next()?.trim();
            let label = parts.next()?.trim();
            if key.is_empty() || label.is_empty() {
                None
            } else {
                Some((key.to_string(), label.to_string()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_pairs() {
        let parsed = parse_tab_help("s:scout u:upload p:portal");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], ("s".into(), "scout".into()));
        assert_eq!(parsed[2], ("p".into(), "portal".into()));
    }

    #[test]
    fn parses_multi_char_key() {
        let parsed = parse_tab_help("Enter:harvest h:harvest-all");
        assert_eq!(parsed[0].0, "Enter");
        assert_eq!(parsed[0].1, "harvest");
        assert_eq!(parsed[1].0, "h");
        assert_eq!(parsed[1].1, "harvest-all");
    }

    #[test]
    fn skips_tokens_without_colon() {
        let parsed = parse_tab_help("scout u:upload");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "u");
    }
}

//! Color palette and style definitions for the TUI dashboard.
//!
//! Maps the Go `DefaultTheme()` colors to ratatui `Style` values.

use ratatui::style::{Color, Modifier, Style};

/// Theme holds pre-built styles for the TUI.
pub struct Theme {
    pub brand: Style,
    pub tab_active: Style,
    pub tab_inactive: Style,
    pub cursor: Style,
    pub selected: Style,
    pub staged: Style,
    pub modified: Style,
    pub deleted: Style,
    pub untracked: Style,
    pub ignored: Style,
    pub diff_add: Style,
    pub diff_remove: Style,
    pub diff_header: Style,
    pub diff_context: Style,
    pub error: Style,
    pub success: Style,
    pub warning: Style,
    pub info: Style,
    pub status_bar: Style,
    pub help_key: Style,
    pub help_desc: Style,
    pub title: Style,
    pub dim: Style,
}

impl Theme {
    pub fn default_theme() -> Self {
        let purple = Color::Rgb(125, 86, 244);

        Self {
            brand: Style::default().fg(purple),
            tab_active: Style::default().fg(Color::White).bg(purple),
            tab_inactive: Style::default().fg(Color::Rgb(136, 136, 136)),
            cursor: Style::default().fg(purple).add_modifier(Modifier::BOLD),
            selected: Style::default().bg(Color::Rgb(68, 68, 68)),
            staged: Style::default().fg(Color::Rgb(0, 255, 0)),
            modified: Style::default().fg(Color::Rgb(85, 153, 255)),
            deleted: Style::default().fg(Color::Rgb(255, 85, 85)),
            untracked: Style::default().fg(Color::Rgb(255, 255, 85)),
            ignored: Style::default().fg(Color::Rgb(102, 102, 102)),
            diff_add: Style::default().fg(Color::Rgb(0, 255, 0)),
            diff_remove: Style::default().fg(Color::Rgb(255, 85, 85)),
            diff_header: Style::default()
                .fg(Color::Rgb(85, 153, 255))
                .add_modifier(Modifier::BOLD),
            diff_context: Style::default().fg(Color::White),
            error: Style::default()
                .fg(Color::Rgb(255, 85, 85))
                .add_modifier(Modifier::BOLD),
            success: Style::default().fg(Color::Rgb(0, 255, 0)),
            warning: Style::default().fg(Color::Rgb(255, 255, 85)),
            info: Style::default().fg(Color::Rgb(85, 255, 255)),
            status_bar: Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(40, 40, 40)),
            help_key: Style::default()
                .fg(purple)
                .add_modifier(Modifier::BOLD),
            help_desc: Style::default().fg(Color::Rgb(200, 200, 200)),
            title: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            dim: Style::default().fg(Color::Rgb(102, 102, 102)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_styles_are_non_default() {
        let theme = Theme::default_theme();
        // Active tab should have a background color set
        assert_ne!(theme.tab_active, Style::default());
        assert_ne!(theme.error, Style::default());
        assert_ne!(theme.staged, Style::default());
        assert_ne!(theme.brand, Style::default());
    }
}

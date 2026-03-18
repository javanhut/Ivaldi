//! Tab view trait and implementations.

pub mod status;
pub mod log;
pub mod diff;
pub mod timeline;
pub mod remote;
pub mod fuse;
pub mod help;

use crossterm::event::KeyEvent;
use ratatui::prelude::*;

use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};

/// Trait implemented by each tab view in the dashboard.
pub trait TabView {
    /// Handle a key event. Returns an Action indicating what happened.
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action;

    /// Render this tab into the given area.
    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme);

    /// Load or reload data for this tab from the repo.
    fn load_data(&mut self, ctx: &AppContext);

    /// Short help text describing this tab's keybindings.
    fn short_help(&self) -> &str;

    /// Whether this tab has an active text input (suppresses global key handling).
    fn has_active_input(&self) -> bool;
}

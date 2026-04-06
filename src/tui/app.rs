//! Root App struct, main event loop, and tab dispatch.

use std::io;
use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::prelude::*;

use crate::repo::Repo;
use crate::tui::components::status_bar::StatusBar;
use crate::tui::components::tab_bar::TabBar;
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext, StatusData, TabId};
use crate::tui::views::TabView;
use crate::tui::views::help::render_help;

use crate::tui::views::diff::DiffTabView;
use crate::tui::views::fuse::FuseView;
use crate::tui::views::log::LogView;
use crate::tui::views::remote::RemoteView;
use crate::tui::views::review::ReviewView;
use crate::tui::views::status::StatusView;
use crate::tui::views::timeline::TimelineView;

/// Run the TUI dashboard.
pub fn run(work_dir: &Path, ivaldi_dir: &Path) -> Result<(), String> {
    let repo = Repo::open(work_dir).map_err(|e| format!("Failed to open repo: {}", e))?;

    let ctx = AppContext {
        repo,
        work_dir: work_dir.to_path_buf(),
        ivaldi_dir: ivaldi_dir.to_path_buf(),
    };

    let mut app = App::new(ctx);
    app.run().map_err(|e| format!("TUI error: {}", e))
}

struct App {
    ctx: AppContext,
    active_tab: TabId,
    theme: Theme,
    show_help: bool,
    status_data: StatusData,
    message: Option<(String, bool)>, // (text, is_error)
    message_ttl: u8,

    status_view: StatusView,
    log_view: LogView,
    diff_view: DiffTabView,
    timeline_view: TimelineView,
    remote_view: RemoteView,
    fuse_view: FuseView,
    review_view: ReviewView,
}

impl App {
    fn new(ctx: AppContext) -> Self {
        Self {
            ctx,
            active_tab: TabId::Status,
            theme: Theme::default_theme(),
            show_help: false,
            status_data: StatusData::default(),
            message: None,
            message_ttl: 0,

            status_view: StatusView::new(),
            log_view: LogView::new(),
            diff_view: DiffTabView::new(),
            timeline_view: TimelineView::new(),
            remote_view: RemoteView::new(),
            fuse_view: FuseView::new(),
            review_view: ReviewView::new(),
        }
    }

    fn active_view(&self) -> &dyn TabView {
        match self.active_tab {
            TabId::Status => &self.status_view,
            TabId::Log => &self.log_view,
            TabId::Diff => &self.diff_view,
            TabId::Timelines => &self.timeline_view,
            TabId::Remote => &self.remote_view,
            TabId::Fuse => &self.fuse_view,
            TabId::Review => &self.review_view,
        }
    }

    fn load_active_tab(&mut self) {
        match self.active_tab {
            TabId::Status => self.status_view.load_data(&self.ctx),
            TabId::Log => self.log_view.load_data(&self.ctx),
            TabId::Diff => self.diff_view.load_data(&self.ctx),
            TabId::Timelines => self.timeline_view.load_data(&self.ctx),
            TabId::Remote => self.remote_view.load_data(&self.ctx),
            TabId::Fuse => self.fuse_view.load_data(&self.ctx),
            TabId::Review => self.review_view.load_data(&self.ctx),
        }
        self.refresh_status();
    }

    fn refresh_status(&mut self) {
        let timeline = self.ctx.repo.current_timeline().unwrap_or_default();
        let seal_name = self
            .ctx
            .repo
            .walk_history(&timeline)
            .ok()
            .and_then(|h| h.first().map(|e| e.seal_name.clone()))
            .unwrap_or_default();

        self.status_data.timeline = timeline;
        self.status_data.seal_name = seal_name;
    }

    fn switch_tab(&mut self, tab: TabId) {
        if tab != self.active_tab {
            self.active_tab = tab;
            self.load_active_tab();
        }
    }

    fn run(&mut self) -> io::Result<()> {
        let mut terminal = crate::tui::init_terminal()?;

        // Load initial tab data
        self.load_active_tab();

        loop {
            // Draw
            terminal.draw(|frame| self.render(frame))?;

            // Poll for events with timeout (for background task polling)
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    let has_input = self.active_view().has_active_input();

                    // Global keys (only when no active input)
                    if !has_input {
                        let mut handled = true;
                        match key.code {
                            KeyCode::Char('q') => break,
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                break;
                            }
                            KeyCode::Char('?') => {
                                self.show_help = !self.show_help;
                            }
                            KeyCode::Char('1') => self.switch_tab(TabId::Status),
                            KeyCode::Char('2') => self.switch_tab(TabId::Log),
                            KeyCode::Char('3') => self.switch_tab(TabId::Diff),
                            KeyCode::Char('4') => self.switch_tab(TabId::Timelines),
                            KeyCode::Char('5') => self.switch_tab(TabId::Remote),
                            KeyCode::Char('6') => self.switch_tab(TabId::Fuse),
                            KeyCode::Char('7') => self.switch_tab(TabId::Review),
                            KeyCode::Tab => {
                                let next = (self.active_tab.index() + 1) % TabId::ALL.len();
                                if let Some(tab) = TabId::from_index(next) {
                                    self.switch_tab(tab);
                                }
                            }
                            KeyCode::BackTab => {
                                let prev = if self.active_tab.index() == 0 {
                                    TabId::ALL.len() - 1
                                } else {
                                    self.active_tab.index() - 1
                                };
                                if let Some(tab) = TabId::from_index(prev) {
                                    self.switch_tab(tab);
                                }
                            }
                            KeyCode::Esc => {
                                if self.show_help {
                                    self.show_help = false;
                                }
                                // Esc falls through to view if help not showing
                                else {
                                    handled = false;
                                }
                            }
                            _ => handled = false,
                        }

                        if handled {
                            continue;
                        }
                    } else {
                        // When input is active, only handle Ctrl+C globally
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            break;
                        }
                    }

                    // Dispatch to active view
                    let action = match self.active_tab {
                        TabId::Status => self.status_view.handle_event(&key, &mut self.ctx),
                        TabId::Log => self.log_view.handle_event(&key, &mut self.ctx),
                        TabId::Diff => self.diff_view.handle_event(&key, &mut self.ctx),
                        TabId::Timelines => self.timeline_view.handle_event(&key, &mut self.ctx),
                        TabId::Remote => self.remote_view.handle_event(&key, &mut self.ctx),
                        TabId::Fuse => self.fuse_view.handle_event(&key, &mut self.ctx),
                        TabId::Review => self.review_view.handle_event(&key, &mut self.ctx),
                    };

                    self.handle_action(action);
                }
            }

            // Poll background operations (remote tab)
            if let Some(action) = self.remote_view.poll_background() {
                self.handle_action(action);
            }

            // Decay messages
            if self.message.is_some() {
                self.message_ttl = self.message_ttl.saturating_sub(1);
                if self.message_ttl == 0 {
                    self.message = None;
                }
            }
        }

        crate::tui::restore_terminal()?;
        Ok(())
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::None | Action::Consumed => {}
            Action::Refresh => {
                self.load_active_tab();
            }
            Action::StatusUpdate(data) => {
                self.status_data = data;
            }
            Action::Error(msg) => {
                self.message = Some((msg, true));
                self.message_ttl = 30; // ~3 seconds at 100ms poll
            }
            Action::Success(msg) => {
                self.message = Some((msg, false));
                self.message_ttl = 30;
                self.load_active_tab(); // Refresh after success
            }
            Action::Quit => {} // Handled by loop
        }
    }

    fn render(&self, frame: &mut Frame) {
        let screen = frame.area();

        // Layout: tab bar (1) | content (remaining - 2) | status bar (2)
        let tab_bar_area = Rect {
            x: screen.x,
            y: screen.y,
            width: screen.width,
            height: 1,
        };

        let status_bar_area = Rect {
            x: screen.x,
            y: screen.height.saturating_sub(2),
            width: screen.width,
            height: 2,
        };

        let content_area = Rect {
            x: screen.x,
            y: screen.y + 1,
            width: screen.width,
            height: screen.height.saturating_sub(3),
        };

        // Render tab bar
        TabBar::render(frame, tab_bar_area, self.active_tab, &self.theme);

        // Render content
        self.active_view().render(frame, content_area, &self.theme);

        // Render status bar
        StatusBar::render(
            frame,
            status_bar_area,
            &self.status_data,
            self.message.as_ref(),
            &self.theme,
        );

        // Help overlay (on top of everything)
        if self.show_help {
            render_help(frame, &self.theme);
        }
    }
}

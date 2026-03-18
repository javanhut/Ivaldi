//! Remote operations tab — scout, upload, sync, harvest.

use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::portal::PortalManager;
use crate::tui::components::dialog::Dialog;
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};
use crate::tui::views::TabView;

/// Result sent back from background network operations.
pub enum BgResult {
    ScoutDone(Result<Vec<String>, String>),
    UploadDone(Result<String, String>),
    SyncDone(Result<String, String>),
    HarvestDone(Result<String, String>),
}

pub struct RemoteView {
    portal_info: Option<String>,
    remote_timelines: Vec<String>,
    cursor: usize,
    dialog: Dialog,
    busy: bool,
    status_message: Option<(String, bool)>,
    pub bg_sender: Option<mpsc::Sender<BgResult>>,
    pub bg_receiver: Option<mpsc::Receiver<BgResult>>,
}

impl RemoteView {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            portal_info: None,
            remote_timelines: Vec::new(),
            cursor: 0,
            dialog: Dialog::new(""),
            busy: false,
            status_message: None,
            bg_sender: Some(tx),
            bg_receiver: Some(rx),
        }
    }

    /// Check for completed background operations.
    pub fn poll_background(&mut self) -> Option<Action> {
        if let Some(ref rx) = self.bg_receiver {
            match rx.try_recv() {
                Ok(result) => {
                    self.busy = false;
                    match result {
                        BgResult::ScoutDone(Ok(branches)) => {
                            self.remote_timelines = branches;
                            self.status_message =
                                Some((format!("Found {} remote timelines", self.remote_timelines.len()), false));
                        }
                        BgResult::ScoutDone(Err(e)) => {
                            self.status_message = Some((format!("Scout failed: {}", e), true));
                        }
                        BgResult::UploadDone(Ok(msg)) => {
                            self.status_message = Some((msg, false));
                        }
                        BgResult::UploadDone(Err(e)) => {
                            self.status_message = Some((format!("Upload failed: {}", e), true));
                        }
                        BgResult::SyncDone(Ok(msg)) => {
                            self.status_message = Some((msg, false));
                        }
                        BgResult::SyncDone(Err(e)) => {
                            self.status_message = Some((format!("Sync failed: {}", e), true));
                        }
                        BgResult::HarvestDone(Ok(msg)) => {
                            self.status_message = Some((msg, false));
                        }
                        BgResult::HarvestDone(Err(e)) => {
                            self.status_message = Some((format!("Harvest failed: {}", e), true));
                        }
                    }
                    Some(Action::Consumed)
                }
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => None,
            }
        } else {
            None
        }
    }

    fn do_scout(&mut self, ctx: &AppContext) -> Action {
        let pm = PortalManager::new(&ctx.ivaldi_dir);
        let portal = match pm.get_default() {
            Ok(Some(p)) => p,
            Ok(None) => return Action::Error("No portal configured. Use 'p' to set one.".into()),
            Err(e) => return Action::Error(format!("Portal error: {}", e)),
        };

        self.busy = true;
        self.status_message = Some(("Scouting remote timelines...".into(), false));

        let owner = portal.owner.clone();
        let repo = portal.repo.clone();

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                let result = (|| -> Result<Vec<String>, String> {
                    let client = crate::github::GitHubClient::new();
                    if !client.is_authenticated() {
                        return Err("Not authenticated. Run 'ivaldi auth login' first.".to_string());
                    }
                    crate::sync::scout(&client, &owner, &repo).map_err(|e| e.to_string())
                })();
                let _ = tx.send(BgResult::ScoutDone(result));
            });
        }

        Action::Consumed
    }

    fn do_upload(&mut self, ctx: &AppContext) -> Action {
        let pm = PortalManager::new(&ctx.ivaldi_dir);
        let portal = match pm.get_default() {
            Ok(Some(p)) => p,
            Ok(None) => return Action::Error("No portal configured".into()),
            Err(e) => return Action::Error(format!("Portal error: {}", e)),
        };

        self.busy = true;
        self.status_message = Some(("Uploading...".into(), false));

        let owner = portal.owner.clone();
        let repo_name = portal.repo.clone();
        let work_dir = ctx.work_dir.clone();

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                let result = (|| -> Result<String, String> {
                    let client = crate::github::GitHubClient::new();
                    if !client.is_authenticated() {
                        return Err("Not authenticated".to_string());
                    }
                    let repo = crate::repo::Repo::open(&work_dir).map_err(|e| e.to_string())?;
                    crate::sync::upload(&client, &repo, &owner, &repo_name, None, false)
                        .map(|r| format!("Upload complete: {}", r.branch))
                        .map_err(|e| e.to_string())
                })();
                let _ = tx.send(BgResult::UploadDone(result));
            });
        }

        Action::Consumed
    }

    fn do_sync(&mut self, ctx: &AppContext) -> Action {
        let pm = PortalManager::new(&ctx.ivaldi_dir);
        let portal = match pm.get_default() {
            Ok(Some(p)) => p,
            Ok(None) => return Action::Error("No portal configured".into()),
            Err(e) => return Action::Error(format!("Portal error: {}", e)),
        };

        self.busy = true;
        self.status_message = Some(("Syncing...".into(), false));

        let owner = portal.owner.clone();
        let repo_name = portal.repo.clone();
        let work_dir = ctx.work_dir.clone();

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                let result = (|| -> Result<String, String> {
                    let client = crate::github::GitHubClient::new();
                    if !client.is_authenticated() {
                        return Err("Not authenticated".to_string());
                    }
                    let mut repo = crate::repo::Repo::open(&work_dir).map_err(|e| e.to_string())?;
                    let timeline = repo.current_timeline().map_err(|e| e.to_string())?;
                    crate::sync::sync_timeline(&client, &mut repo, &owner, &repo_name, &timeline)
                        .map(|_| "Sync complete".to_string())
                        .map_err(|e| e.to_string())
                })();
                let _ = tx.send(BgResult::SyncDone(result));
            });
        }

        Action::Consumed
    }

    fn do_harvest(&mut self, ctx: &AppContext, names: Vec<String>) -> Action {
        let pm = PortalManager::new(&ctx.ivaldi_dir);
        let portal = match pm.get_default() {
            Ok(Some(p)) => p,
            Ok(None) => return Action::Error("No portal configured".into()),
            Err(e) => return Action::Error(format!("Portal error: {}", e)),
        };

        self.busy = true;
        self.status_message = Some(("Harvesting...".into(), false));

        let owner = portal.owner.clone();
        let repo_name = portal.repo.clone();
        let work_dir = ctx.work_dir.clone();

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                let result = (|| -> Result<String, String> {
                    let client = crate::github::GitHubClient::new();
                    if !client.is_authenticated() {
                        return Err("Not authenticated".to_string());
                    }
                    let mut repo = crate::repo::Repo::open(&work_dir).map_err(|e| e.to_string())?;
                    crate::sync::harvest(&client, &mut repo, &owner, &repo_name, &names)
                        .map(|harvested| format!("Harvested {} timeline(s)", harvested.len()))
                        .map_err(|e| e.to_string())
                })();
                let _ = tx.send(BgResult::HarvestDone(result));
            });
        }

        Action::Consumed
    }
}

impl TabView for RemoteView {
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        // Dialog mode (set portal)
        if self.dialog.visible {
            match event.code {
                KeyCode::Enter => {
                    let value = self.dialog.value().to_string();
                    self.dialog.hide();

                    if value.trim().is_empty() {
                        return Action::Consumed;
                    }

                    let pm = PortalManager::new(&ctx.ivaldi_dir);
                    if let Some(portal) = crate::portal::Portal::parse(&value) {
                        match pm.add(&portal) {
                            Ok(true) => {
                                self.portal_info = Some(portal.to_string_repr());
                                return Action::Success("Portal added".into());
                            }
                            Ok(false) => return Action::Error("Portal already exists".into()),
                            Err(e) => return Action::Error(format!("Failed: {}", e)),
                        }
                    } else {
                        return Action::Error("Invalid format. Use owner/repo".into());
                    }
                }
                KeyCode::Esc => {
                    self.dialog.hide();
                    return Action::Consumed;
                }
                _ => {
                    self.dialog.input.handle_key(event);
                    return Action::Consumed;
                }
            }
        }

        if self.busy {
            return Action::Consumed; // Block input while busy
        }

        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.remote_timelines.is_empty()
                    && self.cursor < self.remote_timelines.len() - 1
                {
                    self.cursor += 1;
                }
                Action::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                Action::Consumed
            }
            KeyCode::Char('s') => self.do_scout(ctx),
            KeyCode::Char('u') => self.do_upload(ctx),
            KeyCode::Char('y') => self.do_sync(ctx),
            KeyCode::Char('h') => {
                // Harvest all
                let names = self.remote_timelines.clone();
                if names.is_empty() {
                    Action::Error("No remote timelines discovered. Scout first.".into())
                } else {
                    self.do_harvest(ctx, names)
                }
            }
            KeyCode::Enter => {
                // Harvest selected
                if let Some(name) = self.remote_timelines.get(self.cursor).cloned() {
                    self.do_harvest(ctx, vec![name])
                } else {
                    Action::Consumed
                }
            }
            KeyCode::Char('p') => {
                self.dialog.show("Portal (owner/repo)");
                Action::Consumed
            }
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Portal info at top
        let portal_area = Rect {
            height: 2,
            ..area
        };
        let portal_text = match &self.portal_info {
            Some(p) => format!("Portal: {}", p),
            None => "No portal configured (press 'p' to add)".into(),
        };
        let portal_para = Paragraph::new(Span::styled(
            portal_text,
            if self.portal_info.is_some() {
                theme.info
            } else {
                theme.warning
            },
        ));
        frame.render_widget(portal_para, portal_area);

        // Status message
        if let Some((ref msg, is_error)) = self.status_message {
            let msg_area = Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: 1,
            };
            let style = if is_error { theme.error } else { theme.success };
            let para = Paragraph::new(Span::styled(msg.as_str(), style));
            frame.render_widget(para, msg_area);
        }

        // Remote timelines list
        let list_area = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: area.height.saturating_sub(3),
        };

        if self.remote_timelines.is_empty() {
            let msg = if self.busy {
                Paragraph::new(Span::styled("Working...", theme.warning))
            } else {
                Paragraph::new(Span::styled(
                    "No remote timelines. Press 's' to scout.",
                    theme.dim,
                ))
            };
            frame.render_widget(msg, list_area);
        } else {
            let items: Vec<ListItem> = self
                .remote_timelines
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let marker = if i == self.cursor { ">" } else { " " };
                    let text = format!("{} {}", marker, name);
                    let style = if i == self.cursor {
                        theme.cursor
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(Span::styled(text, style))
                })
                .collect();

            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    format!(" Remote Timelines ({}) ", self.remote_timelines.len()),
                    theme.title,
                ));

            let list = List::new(items).block(block);
            frame.render_widget(list, list_area);
        }

        // Help
        if area.height > 2 {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let help = Paragraph::new(Span::styled(
                " s:scout u:upload y:sync h:harvest-all Enter:harvest p:portal r:refresh",
                theme.dim,
            ));
            frame.render_widget(help, help_area);
        }

        // Dialog overlay
        self.dialog.render(frame, frame.area(), theme);
    }

    fn load_data(&mut self, ctx: &AppContext) {
        let pm = PortalManager::new(&ctx.ivaldi_dir);
        self.portal_info = pm
            .get_default()
            .ok()
            .flatten()
            .map(|p| p.to_string_repr());
    }

    fn short_help(&self) -> &str {
        "s:scout u:upload y:sync h:harvest p:portal"
    }

    fn has_active_input(&self) -> bool {
        self.dialog.visible
    }
}

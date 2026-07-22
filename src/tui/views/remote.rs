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
    /// Intermediate signal during auth login — the GitHub device code response
    /// arrived; the UI should display the user_code + verification_uri while the
    /// thread keeps polling for the access token.
    AuthLoginCode {
        user_code: String,
        verification_uri: String,
    },
    /// Auth login finished (success or failure). Clears the device-code prompt.
    AuthLoginDone(Result<(), String>),
    /// Auth status query finished. Each line is a "<Platform>: <description>"
    /// or "<Platform>: Not authenticated" string ready to display.
    AuthStatusDone(Vec<String>),
    /// Auth logout finished.
    AuthLogoutDone(Result<(), String>),
}

pub struct RemoteView {
    portal_info: Option<String>,
    remote_timelines: Vec<String>,
    cursor: usize,
    dialog: Dialog,
    busy: bool,
    status_message: Option<(String, bool)>,
    /// While a device-code login is in flight, holds the code + verification
    /// URL so the user can complete the prompt in their browser. Cleared when
    /// `AuthLoginDone` arrives.
    auth_device_prompt: Option<(String, String)>,
    /// Last result of an auth status query.
    auth_status_lines: Vec<String>,
    pub bg_sender: Option<mpsc::Sender<BgResult>>,
    pub bg_receiver: Option<mpsc::Receiver<BgResult>>,
}

impl Default for RemoteView {
    fn default() -> Self {
        Self::new()
    }
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
            auth_device_prompt: None,
            auth_status_lines: Vec::new(),
            bg_sender: Some(tx),
            bg_receiver: Some(rx),
        }
    }

    /// Check for completed background operations.
    pub fn poll_background(&mut self) -> Option<Action> {
        if let Some(ref rx) = self.bg_receiver {
            match rx.try_recv() {
                Ok(result) => {
                    // AuthLoginCode is *intermediate* — keep `busy` true until
                    // AuthLoginDone arrives so the user knows polling is still
                    // happening. All other results are terminal.
                    let terminal = !matches!(result, BgResult::AuthLoginCode { .. });
                    if terminal {
                        self.busy = false;
                    }
                    match result {
                        BgResult::ScoutDone(Ok(branches)) => {
                            self.remote_timelines = branches;
                            self.status_message = Some((
                                format!("Found {} remote timelines", self.remote_timelines.len()),
                                false,
                            ));
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
                        BgResult::AuthLoginCode {
                            user_code,
                            verification_uri,
                        } => {
                            self.auth_device_prompt = Some((user_code, verification_uri));
                            self.status_message =
                                Some(("Waiting for browser confirmation...".into(), false));
                        }
                        BgResult::AuthLoginDone(Ok(())) => {
                            self.auth_device_prompt = None;
                            self.status_message = Some(("Logged in to GitHub".into(), false));
                        }
                        BgResult::AuthLoginDone(Err(e)) => {
                            self.auth_device_prompt = None;
                            self.status_message = Some((format!("Login failed: {}", e), true));
                        }
                        BgResult::AuthStatusDone(lines) => {
                            self.auth_status_lines = lines;
                            self.status_message = Some(("Auth status updated".into(), false));
                        }
                        BgResult::AuthLogoutDone(Ok(())) => {
                            self.auth_status_lines.clear();
                            self.status_message = Some(("Logged out".into(), false));
                        }
                        BgResult::AuthLogoutDone(Err(e)) => {
                            self.status_message = Some((format!("Logout failed: {}", e), true));
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

    fn do_auth_login(&mut self) -> Action {
        if self.busy {
            return Action::Consumed;
        }
        self.busy = true;
        self.auth_device_prompt = None;
        self.status_message = Some(("Requesting device code...".into(), false));

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                use crate::auth::TokenStore;
                use crate::github::GitHubClient;
                use crate::portal::Platform;

                let device = match GitHubClient::request_device_code() {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tx.send(BgResult::AuthLoginDone(Err(e.to_string())));
                        return;
                    }
                };
                let _ = tx.send(BgResult::AuthLoginCode {
                    user_code: device.user_code.clone(),
                    verification_uri: device.verification_uri.clone(),
                });
                let result = (|| -> Result<(), String> {
                    let token = GitHubClient::poll_for_token(&device.device_code, device.interval)
                        .map_err(|e| e.to_string())?;
                    let store = TokenStore::new().map_err(|e| e.to_string())?;
                    store
                        .save_token(Platform::GitHub, token)
                        .map_err(|e| e.to_string())
                })();
                let _ = tx.send(BgResult::AuthLoginDone(result));
            });
        }
        Action::Consumed
    }

    fn do_auth_status(&mut self) -> Action {
        if self.busy {
            return Action::Consumed;
        }
        self.busy = true;
        self.status_message = Some(("Checking auth status...".into(), false));

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                use crate::auth;
                use crate::portal::Platform;
                let lines: Vec<String> =
                    [(Platform::GitHub, "GitHub"), (Platform::GitLab, "GitLab")]
                        .iter()
                        .map(|(platform, name)| match auth::resolve_auth(*platform) {
                            Some(method) => format!("{}: {}", name, method.description),
                            None => format!("{}: Not authenticated", name),
                        })
                        .collect();
                let _ = tx.send(BgResult::AuthStatusDone(lines));
            });
        }
        Action::Consumed
    }

    fn do_auth_logout(&mut self) -> Action {
        if self.busy {
            return Action::Consumed;
        }
        self.busy = true;
        self.status_message = Some(("Logging out...".into(), false));

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                use crate::auth::TokenStore;
                use crate::portal::Platform;
                let result = (|| -> Result<(), String> {
                    let store = TokenStore::new().map_err(|e| e.to_string())?;
                    store
                        .delete_token(Platform::GitHub)
                        .map_err(|e| e.to_string())
                })();
                let _ = tx.send(BgResult::AuthLogoutDone(result));
            });
        }
        Action::Consumed
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

        let portal_clone = portal.clone();

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                let result = {
                    let client = crate::github::GitHubClient::new();
                    crate::sync::scout(&client, &portal_clone).map_err(|e| e.to_string())
                };
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
                    let mut repo = crate::repo::Repo::open(&work_dir).map_err(|e| e.to_string())?;
                    let timeline = repo.current_timeline().map_err(|e| e.to_string())?;
                    let report = crate::git_remote::SmartHttpClient::new(client.token())
                        .push_repo(&mut repo, &owner, &repo_name, &timeline, false)
                        .map_err(|e| e.to_string())?;
                    if !report.unpack_ok {
                        return Err(format!(
                            "remote rejected pack: {}",
                            report.unpack_error.unwrap_or_else(|| "unknown".into())
                        ));
                    }
                    if let Some(r) = report.refs.iter().find(|r| r.error.is_some()) {
                        return Err(format!(
                            "{} rejected: {}",
                            r.name,
                            r.error.clone().unwrap_or_default()
                        ));
                    }
                    Ok(format!("Upload complete: {}", timeline))
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
                    // The user explicitly triggered this sync from the TUI,
                    // which is their consent (equivalent to `sync --yes`);
                    // this background thread has no stdin to prompt on.
                    // ponytail: a proper in-TUI confirm dialog showing the
                    // incoming count is the upgrade path.
                    crate::sync::sync_timeline(
                        &client,
                        &mut repo,
                        &owner,
                        &repo_name,
                        &timeline,
                        &mut |_, _| true,
                        false,
                    )
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

        let portal_clone = portal.clone();
        let work_dir = ctx.work_dir.clone();

        if let Some(tx) = self.bg_sender.clone() {
            std::thread::spawn(move || {
                let result = (|| -> Result<String, String> {
                    let client = crate::github::GitHubClient::new();
                    let mut repo = crate::repo::Repo::open(&work_dir).map_err(|e| e.to_string())?;
                    crate::sync::harvest(&client, &mut repo, &portal_clone, &names)
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
            // While a device-code login is in flight we still want the user to
            // be able to cancel via Esc, but everything else is blocked.
            if self.auth_device_prompt.is_some() && matches!(event.code, KeyCode::Esc) {
                self.auth_device_prompt = None;
                self.busy = false;
                self.status_message = Some(("Login cancelled".into(), true));
                return Action::Consumed;
            }
            return Action::Consumed;
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
            KeyCode::Char('L') => self.do_auth_login(),
            KeyCode::Char('A') => self.do_auth_status(),
            KeyCode::Char('O') => self.do_auth_logout(),
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Top header: portal line + status line
        let header_lines = 2u16;
        // Optional auth panel (device-code prompt and/or status query result).
        let auth_lines: u16 = self.auth_device_prompt.as_ref().map(|_| 3).unwrap_or(0)
            + self.auth_status_lines.len() as u16;

        let portal_text = match &self.portal_info {
            Some(p) => format!("Portal: {}", p),
            None => "No portal configured (press 'p' to add)".into(),
        };
        let portal_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
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

        // Auth panel (device-code prompt + status lines)
        if auth_lines > 0 {
            let mut y = area.y + header_lines;
            if let Some((code, url)) = &self.auth_device_prompt {
                let code_line = Paragraph::new(Line::from(vec![
                    Span::styled("Code: ", theme.dim),
                    Span::styled(code.clone(), theme.help_key),
                ]));
                frame.render_widget(
                    code_line,
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    },
                );
                y += 1;
                let url_line = Paragraph::new(Line::from(vec![
                    Span::styled("Open: ", theme.dim),
                    Span::styled(url.clone(), theme.info),
                ]));
                frame.render_widget(
                    url_line,
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    },
                );
                y += 1;
                let cancel_hint = Paragraph::new(Span::styled(
                    "Esc to cancel — polling for confirmation...",
                    theme.warning,
                ));
                frame.render_widget(
                    cancel_hint,
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    },
                );
                y += 1;
            }
            for line in &self.auth_status_lines {
                let para = Paragraph::new(Span::styled(line.clone(), theme.info));
                frame.render_widget(
                    para,
                    Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    },
                );
                y += 1;
            }
        }

        // Remote timelines list
        let list_y = area.y + header_lines + auth_lines;
        let list_area = Rect {
            x: area.x,
            y: list_y,
            width: area.width,
            height: area.height.saturating_sub(header_lines + auth_lines + 1),
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

            let block = Block::default().borders(Borders::ALL).title(Span::styled(
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
                " s:scout u:upload y:sync h:harvest-all Enter:harvest p:portal L:login A:auth-status O:logout r:refresh",
                theme.dim,
            ));
            frame.render_widget(help, help_area);
        }

        // Dialog overlay
        self.dialog.render(frame, frame.area(), theme);
    }

    fn load_data(&mut self, ctx: &AppContext) {
        let pm = PortalManager::new(&ctx.ivaldi_dir);
        self.portal_info = pm.get_default().ok().flatten().map(|p| p.to_string_repr());
    }

    fn short_help(&self) -> &str {
        "s:scout u:upload y:sync h:harvest p:portal L:login A:auth-status O:logout"
    }

    fn has_active_input(&self) -> bool {
        self.dialog.visible
    }
}

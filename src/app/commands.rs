//! Commands accepted from the two embedded local interfaces.
//!
//! The JSON wire format is shared with `ui/{manager,toolbar,controls}.js`.
//! Deserialization validates payload shapes; `allowed_from` checks which local
//! surface may perform each action. Remote device IPC never enters this path.

use super::{App, events::Surface};
use crate::{storage::profiles::ProfileInput, ui::locale::tr};
use anyhow::{Context, Result};
use serde::Deserialize;
use tao::window::Fullscreen;
use wry::WebViewExtWindows;
use zeroize::Zeroizing;

/// `kind` is a snake_case tag; field names are also part of the JS contract.
/// Keep this free of `Debug` because `SaveProfile` carries a plaintext password.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Command {
    UiReady,
    ShowManager,
    HideManager,
    NewProfile,
    SelectProfile {
        id: String,
    },
    SaveProfile {
        id: Option<String>,
        name: String,
        url: String,
        password: Option<String>,
        remember: bool,
        connect: bool,
    },
    ConnectProfile {
        id: String,
    },
    DeleteProfile {
        id: String,
    },
    ForgetPassword {
        id: String,
    },
    TrustCertificate {
        attempt: u64,
        fingerprint: String,
    },
    CancelConnect,
    Capture {
        enabled: bool,
    },
    Fullscreen,
    ToggleToolbarPin,
    RevealToolbar {
        visible: bool,
    },
    ToggleTheme,
    SuspendCapture,
}

impl Command {
    /// Defense in depth after `ui::control` validates the sender's document URL.
    fn allowed_from(&self, surface: Surface) -> bool {
        use Command::*;
        match self {
            RevealToolbar { .. } | ToggleToolbarPin => surface == Surface::Toolbar,
            NewProfile
            | SelectProfile { .. }
            | SaveProfile { .. }
            | DeleteProfile { .. }
            | ForgetPassword { .. }
            | TrustCertificate { .. }
            | CancelConnect
            | HideManager => surface == Surface::Manager,
            UiReady
            | ShowManager
            | ConnectProfile { .. }
            | Capture { .. }
            | Fullscreen
            | ToggleTheme
            | SuspendCapture => true,
        }
    }
}

impl App {
    /// Apply one authorized local action and publish any resulting UI state.
    /// Errors propagate to the event loop, which shows them in the manager.
    pub(super) fn command(&mut self, surface: Surface, message: &str) -> Result<()> {
        let command: Command = serde_json::from_str(message).context(tr("error.request"))?;
        // The manager remains visible during its exit animation. Ignore late
        // clicks from that outgoing form; readiness may still request a snapshot.
        if surface == Surface::Manager
            && self.manager_close_at.is_some()
            && !matches!(&command, Command::UiReady)
        {
            return Ok(());
        }
        if !command.allowed_from(surface) {
            return Ok(());
        }
        match command {
            Command::UiReady => self.publish(),
            Command::ShowManager => {
                self.show_manager();
                self.publish();
            }
            Command::HideManager => {
                if self.remote.is_some() && self.pending.is_none() {
                    self.show_remote();
                }
            }
            Command::NewProfile => self.edit(None),
            Command::SelectProfile { id } => {
                self.store.get(&id)?;
                self.edit(Some(id));
            }
            Command::SaveProfile {
                id,
                name,
                url,
                password,
                remember,
                connect,
            } => {
                // Invalidate any worker before editing its profile. The worker
                // may finish, but connection_result will discard its outcome.
                self.pending = None;
                let password = Zeroizing::new(password.unwrap_or_default());
                let profile = self.store.save(ProfileInput {
                    id,
                    name,
                    url,
                    password: &password,
                    remember,
                })?;
                self.selected = Some(profile.id.clone());
                self.form_version += 1;
                self.notice = Some(tr("notice.saved"));
                if connect {
                    // An empty field means reuse the saved password. A supplied
                    // password also works for this login when remember is false.
                    let password = if password.is_empty() {
                        profile.password()?.context(tr("error.password_required"))?
                    } else {
                        password
                    };
                    self.begin(profile, password);
                } else {
                    self.publish();
                }
            }
            Command::ConnectProfile { id } => {
                if surface == Surface::Toolbar
                    && self.stats.connected
                    && self.viewer_id() == Some(id.as_str())
                {
                    return Ok(());
                }
                let profile = self.store.get(&id)?.clone();
                self.selected = Some(id);
                self.form_version += 1;
                let password = profile.password()?.context(tr("error.enter_password"))?;
                self.begin(profile, password);
            }
            Command::DeleteProfile { id } => {
                self.pending = None;
                self.store.delete(&id)?;
                self.selected = self.store.all().first().map(|p| p.id.clone());
                self.form_version += 1;
                self.notice = Some(tr("notice.deleted"));
                self.publish();
            }
            Command::ForgetPassword { id } => {
                self.pending = None;
                self.store.forget(&id)?;
                self.form_version += 1;
                self.notice = Some(tr("notice.forgotten"));
                self.publish();
            }
            Command::TrustCertificate {
                attempt,
                fingerprint,
            } => {
                self.approve_certificate(attempt, fingerprint)?;
            }
            Command::CancelConnect => {
                self.pending = None;
                self.notice = Some(tr("notice.cancelled"));
                self.publish();
            }
            Command::Capture { enabled } => {
                // The local switch changes a persistent preference. The remote
                // bridge's capture message only requests a temporary acquisition.
                self.preferences.update(|s| s.keyboard_capture = enabled)?;
                self.click_capture(enabled);
                self.update_toolbar();
            }
            Command::ToggleTheme => {
                self.preferences.update(|s| s.theme = s.theme.toggle())?;
                let theme = self.preferences.settings.theme;
                self.window.set_theme(Some(theme.native()));
                let value = serde_json::to_string(&theme)?;
                for view in [&self.manager, &self.toolbar] {
                    let _ = view.set_theme(theme.webview());
                    let _ = view.evaluate_script(&format!("window.cometUI?.setTheme({value})"));
                }
                if let Some(viewer) = &self.remote {
                    let _ = viewer.view.set_theme(theme.webview());
                }
            }
            Command::SuspendCapture => {
                // Opening a local selector must release keys without disabling
                // the user's preference to capture on the next console click.
                self.release();
            }
            Command::RevealToolbar { visible } => {
                if self.auto_hide_toolbar() && self.toolbar_revealed != visible {
                    self.toolbar_revealed = visible;
                    self.resize();
                }
            }
            Command::ToggleToolbarPin => {
                self.preferences
                    .update(|s| s.toolbar_pinned = !s.toolbar_pinned)?;
                self.toolbar_revealed = false;
                self.resize();
            }
            Command::Fullscreen => {
                self.toolbar_revealed = false;
                self.window
                    .set_fullscreen(if self.window.fullscreen().is_some() {
                        None
                    } else {
                        Some(Fullscreen::Borderless(None))
                    });
                self.resize();
            }
        }
        Ok(())
    }
    /// Select/refill the editor explicitly, invalidating pending authentication.
    fn edit(&mut self, id: Option<String>) {
        self.pending = None;
        self.selected = id;
        self.form_version += 1;
        self.notice = None;
        self.publish();
    }
}

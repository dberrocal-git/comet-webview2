//! Asynchronous authentication and promotion of a successful login to a viewer.
//!
//! Workers own cloned inputs and send outcomes back through `AppEvent`. Cancelling
//! drops `pending`, not the worker thread; attempt IDs make late results inert.
//! The previous viewer remains available while another login is in progress.

use super::{App, Viewer, events::AppEvent};
use crate::{
    connection::{self, CertificateChange, Outcome},
    storage::profiles::Profile,
    ui::{self, locale::tr},
};
use anyhow::Result;
use tao::event_loop::EventLoopProxy;
use zeroize::Zeroizing;

/// One login, optionally paused for explicit certificate approval.
/// The password stays available for the retry after the certificate is pinned.
pub(super) struct PendingLogin {
    attempt: u64,
    profile: Profile,
    password: Zeroizing<String>,
    challenge: Option<CertificateChange>,
}

impl PendingLogin {
    /// Perform blocking DNS/TCP/TLS/HTTP work away from the window thread.
    /// Dropping the app or replacing the attempt does not interrupt this work.
    fn start(&self, proxy: EventLoopProxy<AppEvent>) {
        let attempt = self.attempt;
        let profile = self.profile.clone();
        let password = self.password.clone();
        std::thread::spawn(move || {
            let outcome = connection::connect(&profile, &password);
            let _ = proxy.send_event(AppEvent::Connection { attempt, outcome });
        });
    }

    /// Export only certificate-review data; never serialize the pending login.
    pub(super) fn trust_prompt(&self) -> Option<serde_json::Value> {
        let change = self.challenge.as_ref()?;
        Some(serde_json::json!({
            "attempt": self.attempt, "url": self.profile.url,
            "fingerprint": change.fingerprint, "previous": change.previous,
        }))
    }

    /// Bind approval to both the current attempt and the observed certificate.
    fn awaiting(&self, attempt: u64, fingerprint: &str) -> bool {
        self.attempt == attempt
            && self
                .challenge
                .as_ref()
                .is_some_and(|c| c.fingerprint == fingerprint)
    }
}

impl App {
    /// Replace the pending login, keeping any existing viewer hidden and intact.
    pub(super) fn begin(&mut self, profile: Profile, password: Zeroizing<String>) {
        self.show_manager();
        self.next_attempt += 1;
        let login = PendingLogin {
            attempt: self.next_attempt,
            profile,
            password,
            challenge: None,
        };
        login.start(self.proxy.clone());
        self.pending = Some(login);
        self.notice = Some(tr("notice.connecting"));
        self.publish();
    }

    /// Persist explicit trust before retrying on a fresh TLS connection.
    /// The retry checks the new connection's certificate before sending a password.
    pub(super) fn approve_certificate(&mut self, attempt: u64, fingerprint: String) -> Result<()> {
        let Some(login) = self
            .pending
            .as_mut()
            .filter(|p| p.awaiting(attempt, &fingerprint))
        else {
            return Ok(());
        };
        self.store.pin(&login.profile.id, fingerprint.clone())?;
        login.profile.certificate_sha256 = Some(fingerprint);
        login.challenge = None;
        login.start(self.proxy.clone());
        self.notice = Some(tr("notice.verifying"));
        self.publish();
        Ok(())
    }

    /// Consume only the current attempt; stale/cancelled outcomes are dropped.
    pub(super) fn connection_result(&mut self, attempt: u64, outcome: Outcome) {
        if self.close_at.is_some() {
            return;
        }
        // A cancelled worker may finish after a replacement login has started.
        let Some(mut login) = self.pending.take_if(|p| p.attempt == attempt) else {
            return;
        };
        match outcome {
            Outcome::TrustRequired(change) => {
                login.challenge = Some(change);
                self.pending = Some(login);
                self.notice = None;
            }
            Outcome::Failed(error) => self.notice = Some(error),
            Outcome::Authenticated(token) => {
                if let Err(error) = self.open_session(attempt, login.profile, &token) {
                    self.notice = Some(format!("{}: {error}", tr("error.open_viewer")));
                    self.show_manager();
                }
            }
        }
        self.publish();
    }

    /// Build the replacement first so a construction error leaves the old viewer.
    /// Release keys against the old bridge before assigning the new `Viewer`.
    fn open_session(&mut self, session: u64, profile: Profile, token: &str) -> Result<()> {
        let view = ui::remote(
            &self.window,
            &mut self.context,
            &profile,
            token,
            session,
            self.proxy.clone(),
            self.preferences.settings.theme,
        )?;
        self.release();
        self.notice = self
            .store
            .connected(&profile.id)
            .err()
            .map(|_| tr("notice.last_selection"));
        self.toolbar_revealed = false;
        self.remote = Some(Viewer {
            session,
            profile_id: profile.id,
            url: profile.url,
            view,
        });
        self.stats = Default::default();
        self.show_remote();
        Ok(())
    }
}

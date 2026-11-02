use super::{App, Viewer, events::AppEvent};
use crate::{
    connection::{self, CertificateChange, Outcome},
    storage::profiles::Profile,
    ui::{self, locale::tr},
};
use anyhow::Result;
use tao::event_loop::EventLoopProxy;
use zeroize::Zeroizing;

pub(super) struct PendingLogin {
    attempt: u64,
    profile: Profile,
    password: Zeroizing<String>,
    challenge: Option<CertificateChange>,
}

impl PendingLogin {
    fn start(&self, proxy: EventLoopProxy<AppEvent>) {
        let attempt = self.attempt;
        let profile = self.profile.clone();
        let password = self.password.clone();
        std::thread::spawn(move || {
            let outcome = connection::connect(&profile, &password);
            let _ = proxy.send_event(AppEvent::Connection { attempt, outcome });
        });
    }

    pub(super) fn trust_prompt(&self) -> Option<serde_json::Value> {
        let change = self.challenge.as_ref()?;
        Some(serde_json::json!({
            "attempt": self.attempt, "url": self.profile.url,
            "fingerprint": change.fingerprint, "previous": change.previous,
        }))
    }

    fn awaiting(&self, attempt: u64, fingerprint: &str) -> bool {
        self.attempt == attempt
            && self
                .challenge
                .as_ref()
                .is_some_and(|c| c.fingerprint == fingerprint)
    }
}

impl App {
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

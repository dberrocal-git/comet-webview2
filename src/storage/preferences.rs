//! User preferences stored in `settings.json`, independently of device profiles.
//!
//! Missing fields receive defaults, allowing older files to survive added options.
//! A missing file is created on first open; malformed values fail rather than
//! silently replacing the user's settings. Applying a setting to native windows
//! and WebViews belongs to the app layer after a successful save.

use super::{read_json, update_json, write_json};
use crate::ui::locale::tr;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Explicit persisted theme; there is no automatic system-theme mode.
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    #[default]
    Dark,
}
impl Theme {
    /// Convert the shared preference to the native window library's theme type.
    pub fn native(self) -> tao::window::Theme {
        match self {
            Self::Light => tao::window::Theme::Light,
            Self::Dark => tao::window::Theme::Dark,
        }
    }
    /// Convert the same preference to the WebView library's separate theme type.
    pub fn webview(self) -> wry::Theme {
        match self {
            Self::Light => wry::Theme::Light,
            Self::Dark => wry::Theme::Dark,
        }
    }
    /// Return the other supported theme without persisting or applying it.
    pub fn toggle(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}
/// Serialized choices, not transient focus, connection, or capture status.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// Catalog code loaded at startup; changing it in the file requires a restart.
    pub language: String,
    /// User's capture preference; active capture also requires app focus/readiness.
    pub keyboard_capture: bool,
    /// Keep the toolbar visible during a remote session, including fullscreen.
    pub toolbar_pinned: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            language: "en".into(),
            keyboard_capture: true,
            toolbar_pinned: false,
        }
    }
}
/// Owns the settings path and the last state successfully loaded or saved.
pub struct Preferences {
    path: PathBuf,
    pub settings: Settings,
}
impl Preferences {
    /// Load settings or persist defaults when the file does not yet exist.
    pub fn open(path: PathBuf) -> Result<Self> {
        let settings = match read_json(&path).with_context(|| tr("error.settings_read"))? {
            Some(settings) => settings,
            None => {
                let settings = Settings::default();
                write_json(&path, &settings).with_context(|| tr("error.save_settings"))?;
                settings
            }
        };
        Ok(Self { path, settings })
    }
    /// Save a staged change before exposing it through `settings`.
    pub fn update(&mut self, change: impl FnOnce(&mut Settings)) -> Result<()> {
        update_json(&self.path, &mut self.settings, |settings| {
            change(settings);
            Ok(())
        })
        .with_context(|| tr("error.save_settings"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_and_manual_capture_choice_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("comet-preferences-{}", uuid::Uuid::new_v4()));
        let path = dir.join("settings.json");
        let mut preferences = Preferences::open(path.clone()).unwrap();
        assert!(preferences.settings.theme == Theme::Dark);
        preferences
            .update(|settings| {
                settings.theme = Theme::Light;
                settings.keyboard_capture = false;
                settings.toolbar_pinned = true;
            })
            .unwrap();
        let restored = Preferences::open(path).unwrap();
        assert!(restored.settings.theme == Theme::Light);
        assert!(!restored.settings.keyboard_capture);
        assert!(restored.settings.toolbar_pinned);
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}

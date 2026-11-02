use super::{read_json, update_json, write_json};
use crate::ui::locale::tr;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    #[default]
    Dark,
}
impl Theme {
    pub fn native(self) -> tao::window::Theme {
        match self {
            Self::Light => tao::window::Theme::Light,
            Self::Dark => tao::window::Theme::Dark,
        }
    }
    pub fn webview(self) -> wry::Theme {
        match self {
            Self::Light => wry::Theme::Light,
            Self::Dark => wry::Theme::Dark,
        }
    }
    pub fn toggle(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub language: String,
    pub keyboard_capture: bool,
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
pub struct Preferences {
    path: PathBuf,
    pub settings: Settings,
}
impl Preferences {
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

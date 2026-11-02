use serde::Serialize;
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

#[derive(Clone, Serialize)]
pub struct Catalog {
    pub language: String,
    pub strings: BTreeMap<String, String>,
}
static ACTIVE: OnceLock<Catalog> = OnceLock::new();
static ENGLISH: OnceLock<Catalog> = OnceLock::new();
fn english() -> &'static Catalog {
    ENGLISH.get_or_init(|| Catalog {
        language: "en".into(),
        strings: serde_json::from_str(include_str!("../../locales/en.json"))
            .expect("Bundled English catalog must be valid"),
    })
}
pub fn valid_language(code: &str) -> bool {
    (2..=16).contains(&code.len()) && code.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
}
impl Catalog {
    pub fn load(directory: &Path, language: &str) -> Self {
        let mut catalog = english().clone();
        let merge = |catalog: &mut Catalog, path: &Path| -> bool {
            let Ok(bytes) = std::fs::read(path) else {
                return false;
            };
            if bytes.len() > 1024 * 1024 {
                return false;
            }
            let Ok(strings) = serde_json::from_slice::<BTreeMap<String, String>>(&bytes) else {
                return false;
            };
            catalog.strings.extend(strings);
            true
        };
        merge(&mut catalog, &directory.join("en.json"));
        if language != "en"
            && valid_language(language)
            && merge(&mut catalog, &directory.join(format!("{language}.json")))
        {
            catalog.language = language.into();
        }
        catalog
    }
    pub fn text(&self, key: &str) -> String {
        self.strings.get(key).cloned().unwrap_or_else(|| key.into())
    }
}
pub fn init(language: &str) {
    let directory = std::env::current_exe()
        .map(|exe| exe.with_file_name("locales"))
        .unwrap_or_else(|_| "locales".into());
    let _ = ACTIVE.set(Catalog::load(&directory, language));
}
pub fn current() -> &'static Catalog {
    ACTIVE.get().unwrap_or_else(|| english())
}
pub fn tr(key: &str) -> String {
    current().text(key)
}
pub fn format(key: &str, parameters: &[(&str, &str)]) -> String {
    let mut text = tr(key);
    for (name, value) in parameters {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogs_fall_back_and_cannot_escape_the_locale_folder() {
        let dir = std::env::temp_dir().join(format!("comet-locale-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("fr.json"), br#"{"action.connect":"Connexion"}"#).unwrap();
        let catalog = Catalog::load(&dir, "fr");
        assert_eq!(catalog.text("action.connect"), "Connexion");
        assert_eq!(catalog.text("action.save"), "Save");
        assert_eq!(Catalog::load(&dir, "../private").language, "en");
        assert!(!valid_language("../private"));
        std::fs::remove_file(dir.join("fr.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}

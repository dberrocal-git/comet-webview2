use super::{read_json, update_json};
use crate::ui::locale::tr;
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
use windows::Win32::{
    Foundation::{HLOCAL, LocalFree},
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub url: String,
    pub encrypted_password: Option<String>,
    pub certificate_sha256: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Config {
    version: u32,
    profiles: Vec<Profile>,
    last_profile_id: Option<String>,
}

impl Config {
    fn profile_mut(&mut self, id: &str) -> Result<&mut Profile> {
        self.profiles
            .iter_mut()
            .find(|p| p.id == id)
            .with_context(|| tr("error.profile_missing"))
    }
}

#[derive(Serialize)]
pub struct PublicProfile<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub url: &'a str,
    pub has_password: bool,
    pub certificate_sha256: &'a Option<String>,
}
impl Profile {
    fn binding(&self) -> String {
        // Keep this versioned prefix stable; DPAPI needs the same entropy to decrypt saved passwords.
        format!("Comet-WebView2/v1|{}|{}", self.id, self.url)
    }
    pub fn public(&self) -> PublicProfile<'_> {
        PublicProfile {
            id: &self.id,
            name: &self.name,
            url: &self.url,
            has_password: self.encrypted_password.is_some(),
            certificate_sha256: &self.certificate_sha256,
        }
    }
    pub fn password(&self) -> Result<Option<Zeroizing<String>>> {
        self.encrypted_password
            .as_ref()
            .map(|value| unprotect(value, &self.binding()))
            .transpose()
    }
}

pub struct ProfileInput<'a> {
    pub id: Option<String>,
    pub name: String,
    pub url: String,
    pub password: &'a str,
    pub remember: bool,
}
pub struct Store {
    path: PathBuf,
    config: Config,
}
impl Store {
    pub fn open(path: PathBuf) -> Result<Self> {
        let mut config = read_json(&path)
            .with_context(|| tr("error.config_read"))?
            .unwrap_or_else(|| Config {
                version: 1,
                profiles: Vec::new(),
                last_profile_id: None,
            });
        if config.version != 1 {
            bail!(tr("error.config_version"));
        }
        for profile in &mut config.profiles {
            if normalize_url(&profile.url)?.as_str() != profile.url {
                bail!(tr("error.profile_url"));
            }
            if let Some(pin) = &profile.certificate_sha256 {
                if pin.len() != 64 || !pin.bytes().all(|b| b.is_ascii_hexdigit()) {
                    bail!(tr("error.fingerprint"));
                }
            }
        }
        Ok(Self { path, config })
    }
    pub fn all(&self) -> &[Profile] {
        &self.config.profiles
    }
    pub fn find(&self, id: &str) -> Option<&Profile> {
        self.all().iter().find(|profile| profile.id == id)
    }
    pub fn get(&self, id: &str) -> Result<&Profile> {
        self.find(id).with_context(|| tr("error.profile_missing"))
    }
    pub fn last(&self) -> Option<&str> {
        self.config.last_profile_id.as_deref()
    }
    pub fn save(&mut self, input: ProfileInput<'_>) -> Result<Profile> {
        let url = normalize_url(&input.url)?;
        let name = if input.name.trim().is_empty() {
            url.host_str()
                .unwrap_or("KVM")
                .chars()
                .take(80)
                .collect::<String>()
        } else {
            input.name.trim().to_owned()
        };
        if name.chars().count() > 80 {
            bail!(tr("error.name_length"));
        }
        if input.password.len() > 4096 {
            bail!(tr("error.password_length"));
        }
        let url = url.to_string();
        self.update(|config| {
            let profile = match input.id {
                Some(id) => {
                    let existing = config.profile_mut(&id)?;
                    if existing.url != url {
                        // A new address must establish its own trust and credentials.
                        existing.certificate_sha256 = None;
                        existing.encrypted_password = None;
                    }
                    existing.url = url;
                    existing.name = name;
                    existing
                }
                None => {
                    config.profiles.push(Profile {
                        id: Uuid::new_v4().to_string(),
                        name,
                        url,
                        certificate_sha256: None,
                        encrypted_password: None,
                    });
                    config.profiles.last_mut().expect("just inserted")
                }
            };
            if !input.remember {
                profile.encrypted_password = None;
            } else if !input.password.is_empty() {
                profile.encrypted_password = Some(protect(input.password, &profile.binding())?);
            }
            Ok(profile.clone())
        })
    }
    pub fn delete(&mut self, id: &str) -> Result<()> {
        self.get(id)?;
        self.update(|config| {
            config.profiles.retain(|p| p.id != id);
            if config.last_profile_id.as_deref() == Some(id) {
                config.last_profile_id = None;
            }
            Ok(())
        })
    }
    pub fn forget(&mut self, id: &str) -> Result<()> {
        self.update(|config| {
            config.profile_mut(id)?.encrypted_password = None;
            Ok(())
        })
    }
    pub fn pin(&mut self, id: &str, pin: String) -> Result<()> {
        self.update(|config| {
            config.profile_mut(id)?.certificate_sha256 = Some(pin);
            Ok(())
        })
    }
    pub fn connected(&mut self, id: &str) -> Result<()> {
        self.update(|config| {
            config.last_profile_id = Some(id.into());
            Ok(())
        })
    }
    fn update<R>(&mut self, edit: impl FnOnce(&mut Config) -> Result<R>) -> Result<R> {
        update_json(&self.path, &mut self.config, edit).with_context(|| tr("error.save_profiles"))
    }
}

pub fn normalize_url(input: &str) -> Result<url::Url> {
    let input = input.trim();
    if input.len() > 1024 {
        bail!(tr("error.url_length"));
    }
    let text = if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let mut url = url::Url::parse(&text).context(tr("error.url"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.path() != "/"
    {
        bail!(tr("error.url_root"));
    }
    url.set_fragment(None);
    Ok(url)
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}
fn protect(password: &str, binding: &str) -> Result<String> {
    let mut output = CRYPT_INTEGER_BLOB::default();
    let input = blob(password.as_bytes());
    let entropy = blob(binding.as_bytes());
    unsafe {
        CryptProtectData(
            &input,
            None,
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .context(tr("error.encrypt"))?;
        let encoded = STANDARD.encode(std::slice::from_raw_parts(
            output.pbData,
            output.cbData as usize,
        ));
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(encoded)
    }
}
fn unprotect(value: &str, binding: &str) -> Result<Zeroizing<String>> {
    let bytes = STANDARD
        .decode(value)
        .context(tr("error.encrypted_password"))?;
    let input = blob(&bytes);
    let entropy = blob(binding.as_bytes());
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .context(tr("error.decrypt"))?;
        let plain = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = std::str::from_utf8(plain)
            .map(|password| Zeroizing::new(password.to_owned()))
            .context(tr("error.encrypted_password"));
        plain.zeroize();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn canonical_https_and_rejected_destinations() {
        assert_eq!(
            normalize_url(" 192.0.2.10/#/ ").unwrap().as_str(),
            "https://192.0.2.10/"
        );
        assert_eq!(
            normalize_url("https://example.test:8443/")
                .unwrap()
                .as_str(),
            "https://example.test:8443/"
        );
        for invalid in [
            "http://example.test/",
            "https://a:b@example.test/",
            "https://example.test/path",
            "https://example.test/?password=x",
            "",
        ] {
            assert!(normalize_url(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn dpapi_roundtrip_integrity_and_profile_binding() {
        let protected = protect("synthetic-password-123", "profile-a").unwrap();
        assert!(!protected.contains("synthetic-password"));
        assert_eq!(
            &*unprotect(&protected, "profile-a").unwrap(),
            "synthetic-password-123"
        );
        assert!(unprotect(&protected, "profile-b").is_err());
        let mut bytes = STANDARD.decode(protected).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(unprotect(&STANDARD.encode(bytes), "profile-a").is_err());
    }
    #[test]
    fn store_reload_forget_and_url_change_never_reuses_password() {
        let dir = std::env::temp_dir().join(format!("comet-test-{}", Uuid::new_v4()));
        let path = dir.join("connections.json");
        let mut store = Store::open(path.clone()).unwrap();
        assert!(
            store.all().is_empty(),
            "a new install must trust no devices"
        );
        let mut input = ProfileInput {
            id: None,
            name: "Test".into(),
            url: "https://example.test/".into(),
            password: "never-write-plaintext",
            remember: true,
        };
        let profile = store.save(input).unwrap();
        assert!(
            !fs::read_to_string(&path)
                .unwrap()
                .contains("never-write-plaintext")
        );
        store = Store::open(path.clone()).unwrap();
        assert_eq!(
            &*store.get(&profile.id).unwrap().password().unwrap().unwrap(),
            "never-write-plaintext"
        );
        let public = serde_json::to_string(&store.get(&profile.id).unwrap().public()).unwrap();
        assert!(!public.contains("encrypted_password"));
        input = ProfileInput {
            id: Some(profile.id.clone()),
            name: "Renamed".into(),
            url: "https://example.test/".into(),
            password: "",
            remember: true,
        };
        assert!(store.save(input).unwrap().password().unwrap().is_some());
        input = ProfileInput {
            id: Some(profile.id.clone()),
            name: "Test".into(),
            url: "https://other.test/".into(),
            password: "",
            remember: true,
        };
        assert!(store.save(input).unwrap().encrypted_password.is_none());
        store.forget(&profile.id).unwrap();
        store.delete(&profile.id).unwrap();
        assert!(store.get(&profile.id).is_err());
        let automatic = store
            .save(ProfileInput {
                id: None,
                name: String::new(),
                url: "https://unnamed.example.test/".into(),
                password: "",
                remember: false,
            })
            .unwrap();
        assert_eq!(automatic.name, "unnamed.example.test");
        // Delete only the two known test artifacts; never recursively remove a computed directory.
        fs::remove_file(dir.join("connections.json")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}

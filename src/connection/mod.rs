//! Blocking, certificate-pinned authentication against the Comet firmware API.
//!
//! The app calls `connect` on a worker thread. Each attempt opens a fresh TLS
//! connection, inspects its leaf certificate, and either requests explicit trust
//! or sends credentials over that same connection. This avoids trusting one
//! observed connection and sending the password over a different, unchecked one.
//! The returned token is handed to the remote WebView; device UI/transport setup
//! remains the firmware's responsibility.
//!
//! Zeroizing wrappers scrub selected owned strings, not the HTTP response buffer,
//! parsed JSON, or browser-owned copies of the session token.

mod response;
pub(crate) mod tls;

use crate::{storage::profiles::Profile, ui::locale};
use anyhow::{Context, Result, bail};
use native_tls::{TlsConnector, TlsStream};
use std::{
    io::Write,
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};
use zeroize::Zeroizing;

// Firmware API contract: profiles supply a password, not a configurable username.
const KVM_ACCOUNT: &str = "admin";

/// Result sent back to the UI thread; certificate approval triggers a new attempt.
pub enum Outcome {
    /// Nonempty firmware session token, zeroed when its owned wrapper is dropped.
    Authenticated(Zeroizing<String>),
    /// No credentials have been sent; show this certificate's fingerprint first.
    TrustRequired(CertificateChange),
    /// Localized failure suitable for the connection manager's notice text.
    Failed(String),
}
/// A first-use or changed-certificate challenge; `previous` supports comparison.
pub struct CertificateChange {
    pub fingerprint: String,
    pub previous: Option<String>,
}
/// Run one blocking attempt, turning operational errors into a UI outcome.
///
/// `profile.url` must already satisfy `storage::profiles::normalize_url`. This
/// function neither prompts nor changes the stored pin. It can finish after the
/// app cancels/replaces a login, so the caller must reject stale attempt IDs.
pub fn connect(profile: &Profile, password: &str) -> Outcome {
    match attempt(profile, password) {
        Ok(value) => value,
        Err(error) => Outcome::Failed(format!("{error:#}")),
    }
}
/// Verify identity before constructing or writing the credential-bearing POST.
fn attempt(profile: &Profile, password: &str) -> Result<Outcome> {
    let url = url::Url::parse(&profile.url)?;
    let host = match url.host().context(locale::tr("error.host"))? {
        url::Host::Domain(host) => host.to_string(),
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Ipv6(address) => address.to_string(),
    };
    let port = url.port_or_known_default().unwrap();
    let mut stream = tls_stream(&host, port)?;
    let certificate = stream
        .peer_certificate()?
        .context(locale::tr("error.no_certificate"))?;
    let fingerprint = tls::fingerprint(&certificate.to_der()?);
    // An unverified handshake only observes the public certificate. No HTTP
    // request or credential is sent until this exact TLS connection is pinned.
    if profile.certificate_sha256.as_deref() != Some(&fingerprint) {
        return Ok(Outcome::TrustRequired(CertificateChange {
            fingerprint,
            previous: profile.certificate_sha256.clone(),
        }));
    }
    // This URL-encoded copy contains the password too; give it the same explicit
    // zeroizing lifetime as the caller's password rather than an ordinary String.
    let body = Zeroizing::new(
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("user", KVM_ACCOUNT)
            .append_pair("passwd", password)
            .finish(),
    );
    // Preserve brackets for IPv6 and any explicit port in the HTTP Host header.
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    write!(
        stream,
        "POST /api/auth/login HTTP/1.1\r\nHost: {authority}\r\n\
        Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\
        Connection: close\r\nAccept: application/json\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())?;
    stream.flush()?;
    let (status, body) =
        response::read_response(stream).with_context(|| locale::tr("error.response_read"))?;
    match status {
        200 => {}
        401 | 403 => bail!(locale::tr("error.credentials")),
        429 => bail!(locale::tr("error.rate_limit")),
        _ => bail!(locale::format(
            "error.http",
            &[("status", &status.to_string())]
        )),
    }
    // Successful HTTP status alone is insufficient: firmware also returns an
    // application-level `ok` flag and a nested `result.token` string.
    let result: serde_json::Value =
        serde_json::from_slice(&body).context(locale::tr("error.incompatible_response"))?;
    if result["ok"] != true {
        bail!(locale::tr("error.login_rejected"));
    }
    let token = result["result"]["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context(locale::tr("error.no_session"))?;
    Ok(Outcome::Authenticated(Zeroizing::new(token.to_string())))
}
/// Connect to at most six resolved addresses, then establish inspectable TLS.
///
/// CA and hostname checks are deliberately disabled for self-signed device
/// certificates. This helper alone provides no authenticated peer identity:
/// `attempt` must compare the certificate pin before sending any HTTP bytes.
/// TCP attempts and socket I/O have timeouts; DNS resolution itself is blocking.
fn tls_stream(host: &str, port: u16) -> Result<TlsStream<TcpStream>> {
    let addresses = (host, port)
        .to_socket_addrs()
        .context(locale::tr("error.resolve"))?;
    let socket = addresses
        .take(6)
        .find_map(|address| TcpStream::connect_timeout(&address, Duration::from_secs(5)).ok())
        .with_context(|| locale::tr("error.connect"))?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    socket.set_write_timeout(Some(Duration::from_secs(10)))?;
    let connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()?;
    connector
        .connect(host, socket)
        .map_err(|_| anyhow::anyhow!(locale::tr("error.tls")))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires COMET_TEST_URL, COMET_TEST_SHA256 and COMET_TEST_PASSWORD"]
    fn real_kvm_authentication_and_pin_before_credentials() {
        let password = Zeroizing::new(std::env::var("COMET_TEST_PASSWORD").unwrap());
        let expected_pin = std::env::var("COMET_TEST_SHA256")
            .unwrap()
            .to_ascii_lowercase();
        let profile = Profile {
            id: "test".into(),
            name: "test".into(),
            url: crate::storage::profiles::normalize_url(&std::env::var("COMET_TEST_URL").unwrap())
                .unwrap()
                .to_string(),
            encrypted_password: None,
            certificate_sha256: None,
        };
        match connect(&profile, &password) {
            Outcome::TrustRequired(change) => {
                assert_eq!(change.fingerprint, expected_pin, "device identity differs");
            }
            _ => panic!("Expected public certificate challenge before login"),
        };
        // Exercise the exact saved-password path, including a disk reload.
        let dir = std::env::temp_dir().join(format!("comet-live-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("connections.json");
        let mut store = crate::storage::profiles::Store::open(path.clone()).unwrap();
        let saved = store
            .save(crate::storage::profiles::ProfileInput {
                id: None,
                name: "Integration test".into(),
                url: profile.url,
                password: &password,
                remember: true,
            })
            .unwrap();
        store.pin(&saved.id, expected_pin).unwrap();
        drop(store);
        let store = crate::storage::profiles::Store::open(path.clone()).unwrap();
        let restored = store.get(&saved.id).unwrap();
        let restored_password = restored.password().unwrap().unwrap();
        let authenticated = matches!(
            connect(restored, &restored_password),
            Outcome::Authenticated(_)
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
        assert!(
            authenticated,
            "Expected authenticated session after restoring DPAPI password"
        );
    }
}

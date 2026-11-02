//! Certificate identity helpers shared by native login and the remote WebView.
//!
//! Native login always compares the saved pin before credentials. WebView2's
//! handler below scopes *certificate-error exceptions* to that pin and HTTPS
//! origin; it is not invoked for certificates WebView2 already considers valid.
//! Navigation and IPC origin restrictions are installed separately in `ui::remote`.

use anyhow::Result;
use base64::Engine;
use sha2::{Digest, Sha256};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::*, ServerCertificateErrorDetectedEventHandler,
};
use windows::{
    Win32::System::Com::CoTaskMemFree,
    core::{Interface, PWSTR},
};
use wry::WebViewExtWindows;

/// Hash the complete leaf certificate, not its public key, as lowercase hex.
/// Certificate renewal therefore changes identity even if the key is reused.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Require an HTTPS scheme/host/effective-port match without URL credentials.
/// Paths and fragments may vary so firmware routes and API requests still work.
pub fn same_origin(value: &str, expected: &url::Origin) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.origin() == *expected
            && url.username().is_empty()
            && url.password().is_none()
    })
}
/// Compare WebView2's PEM certificate using the same DER hash as native TLS.
fn pem_matches(pem: &str, expected: &str) -> bool {
    let encoded: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .is_ok_and(|der| fingerprint(&der) == expected)
}
/// Copy a WebView2 string and consume its COM-allocated buffer.
///
/// # Safety
/// `value` must be a readable NUL-terminated string allocated with the COM task
/// allocator, whose ownership was transferred to this caller by WebView2.
unsafe fn take_string(value: PWSTR) -> String {
    // WebView2 returns a NUL-terminated string allocated with the COM task allocator.
    unsafe {
        let text = value.to_string().unwrap_or_default();
        CoTaskMemFree(Some(value.0.cast()));
        text
    }
}
/// Register a fail-closed certificate-error policy before navigating to a device.
///
/// Only the supplied certificate at the exact HTTPS origin can bypass a browser
/// certificate error. The WebView owns the registered handler for its lifetime;
/// this app does not detach it or change its captured origin/pin in place. Create
/// a new remote view when switching profiles or accepting a changed certificate.
pub fn install(webview: &wry::WebView, origin: url::Origin, fingerprint: String) -> Result<()> {
    // SAFETY: The COM interfaces come from a live WebView on its owning UI thread.
    // WebView2 transfers ownership of the returned PWSTR buffers to this handler.
    unsafe {
        let core: ICoreWebView2_14 = webview.webview().cast()?;
        let handler =
            ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    // Set cancellation before inspecting the request, so
                    // subsequent errors do not grant an exception.
                    args.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL)?;
                    let mut uri = PWSTR::null();
                    args.RequestUri(&mut uri)?;
                    let uri = take_string(uri);
                    let certificate = args.ServerCertificate()?;
                    let mut pem = PWSTR::null();
                    certificate.ToPemEncoding(&mut pem)?;
                    let pem = take_string(pem);
                    if same_origin(&uri, &origin) && pem_matches(&pem, &fingerprint) {
                        args.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW)?;
                    }
                }
                Ok(())
            }));
        // No removal is needed: the handler has the same lifetime as this view.
        let mut token = 0;
        core.add_ServerCertificateErrorDetected(&handler, &mut token)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprint_is_lowercase_sha256() {
        assert_eq!(
            fingerprint(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn certificate_exception_is_origin_scoped() {
        let origin = url::Url::parse("https://192.0.2.10").unwrap().origin();
        assert!(same_origin("https://192.0.2.10/#/", &origin));
        assert!(same_origin(
            "https://example.test:8443/api",
            &url::Url::parse("https://example.test:8443")
                .unwrap()
                .origin()
        ));
        for url in [
            "http://192.0.2.10/",
            "https://192.0.2.10.evil.test/",
            "https://192.0.2.10:444/",
            "https://user@192.0.2.10/",
            "https://example.com/",
        ] {
            assert!(!same_origin(url, &origin), "{url}");
        }
        assert!(!pem_matches("invalid", "invalid"));
    }
}

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

pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn same_origin(value: &str, expected: &url::Origin) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.origin() == *expected
            && url.username().is_empty()
            && url.password().is_none()
    })
}
fn pem_matches(pem: &str, expected: &str) -> bool {
    let encoded: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .is_ok_and(|der| fingerprint(&der) == expected)
}
unsafe fn take_string(value: PWSTR) -> String {
    // WebView2 returns a NUL-terminated string allocated with the COM task allocator.
    unsafe {
        let text = value.to_string().unwrap_or_default();
        CoTaskMemFree(Some(value.0.cast()));
        text
    }
}
pub fn install(webview: &wry::WebView, origin: url::Origin, fingerprint: String) -> Result<()> {
    unsafe {
        let core: ICoreWebView2_14 = webview.webview().cast()?;
        let handler =
            ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
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

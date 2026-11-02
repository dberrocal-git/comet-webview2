# Third-party components

This application uses open-source Rust crates, whose versions, source packages,
and individual license declarations are recorded by Cargo.lock and Cargo metadata.
Principal components include tao, wry, windows-rs, webview2-com, serde, serde_json,
anyhow, url, sha2, base64, native-tls, schannel, uuid, httparse, and zeroize.
Their upstream licenses continue to apply. Windows DPAPI and SChannel are
operating-system components; this application does not bundle a private key
or a separate cryptographic runtime for password storage.

Microsoft's WebView2 SDK loader is linked into the MSVC executable. The LLVM-MinGW
ZIP ships WebView2Loader.dll from webview2-com-sys. The WebView2 Runtime is a separately installed Microsoft
component: https://developer.microsoft.com/microsoft-edge/webview2/

The LLVM-MinGW build includes libunwind.dll, from LLVM's libunwind project.
LLVM components are licensed under Apache-2.0 WITH LLVM-exception:
https://github.com/llvm/llvm-project/blob/main/LICENSE.TXT

The GL.iNet interface is loaded at runtime directly from the user's device and
is not bundled in this distribution. GL.iNet and Comet are their owners' marks.

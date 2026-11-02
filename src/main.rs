//! Windows entry point. Application coordination lives in `app`; see
//! `DEVELOPMENT.md` for the module map and cross-language contracts.
//!
//! Release builds have no console, so startup failures need a native dialog
//! that still works when the window or WebView2 could not be created.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod connection;
mod keyboard;
mod storage;
mod ui;

fn main() {
    if let Err(error) = app::run() {
        use ui::locale;
        use windows::{
            Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
            core::PCWSTR,
        };
        let text: Vec<u16> = format!("{}\n\n{error:#}", locale::tr("error.startup"))
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let title: Vec<u16> = locale::tr("app.name")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // Both UTF-16 buffers are NUL-terminated and outlive the synchronous call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

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

mod keymap;

use anyhow::{Context, Result, bail};
use std::{mem::size_of, ptr};
use windows::{
    Win32::{
        Devices::HumanInterfaceDevice::*,
        Foundation::HWND,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LMENU,
                VK_RMENU,
            },
            WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow},
        },
    },
    core::{GUID, Interface},
};

// The SDK's 256-byte DIK layout, supplied by dinput8.lib (also in MinGW).
#[link(name = "dinput8", kind = "static", modifiers = "-bundle")]
unsafe extern "C" {
    static c_dfDIKeyboard: DIDATAFORMAT;
}

#[link(name = "dxguid", kind = "static", modifiers = "-bundle")]
unsafe extern "C" {}

type Key = (&'static str, bool);

struct Keyboard {
    hwnd: HWND,
    device: IDirectInputDevice8W,
    _input: IDirectInput8W,
    held: [bool; 256],
}

impl Keyboard {
    fn acquire(hwnd: HWND) -> Result<Self> {
        unsafe {
            let mut raw = ptr::null_mut();
            DirectInput8Create(
                GetModuleHandleW(None)?.into(),
                0x0800,
                &IDirectInput8W::IID,
                &mut raw,
                None,
            )?;
            let input = IDirectInput8W::from_raw(raw);
            let mut device = None;
            input.CreateDevice(&GUID_SysKeyboard, &mut device, None)?;
            let device = device.context("DirectInput keyboard missing")?;
            device.SetDataFormat(ptr::addr_of!(c_dfDIKeyboard).cast_mut())?;
            let mut buffer = DIPROPDWORD {
                diph: DIPROPHEADER {
                    dwSize: size_of::<DIPROPDWORD>() as u32,
                    dwHeaderSize: size_of::<DIPROPHEADER>() as u32,
                    dwObj: 0,
                    dwHow: DIPH_DEVICE,
                },
                dwData: 256,
            };
            // The SDK defines DIPROP_BUFFERSIZE as MAKEDIPROP(1), a pointer
            // sentinel, not the address of a GUID containing the number one.
            device.SetProperty(1usize as *const GUID, &mut buffer.diph)?;
            device.SetCooperativeLevel(hwnd, DISCL_FOREGROUND | DISCL_EXCLUSIVE)?;
            device.Acquire()?;
            Ok(Self {
                hwnd,
                device,
                _input: input,
                held: [false; 256],
            })
        }
    }

    fn read(&mut self, stats: &mut AltStats) -> Result<Vec<Key>> {
        let mut events = [DIDEVICEOBJECTDATA::default(); 64];
        let mut count = events.len() as u32;
        // The projected GetDeviceData function discards the success code that
        // indicates overflow. A lost key-up must end the capture session.
        let hr = unsafe {
            (self.device.vtable().GetDeviceData)(
                self.device.as_raw(),
                size_of::<DIDEVICEOBJECTDATA>() as u32,
                events.as_mut_ptr(),
                &mut count,
                0,
            )
        };
        if hr.0 == DI_BUFFEROVERFLOW {
            bail!("Keyboard buffer overflow");
        }
        hr.ok()?;
        let (changes, alt_pressed) = transitions(&events[..count as usize], &mut self.held, stats);
        // DirectInput delivers the physical Alt but the shell can still use
        // the logical modifier to switch local windows. Release only that
        // logical modifier. An unused scan position keeps this synthetic up
        // distinct from the real DIK_LMENU/RMENU break sent to the KVM.
        for (side, (offset, vk)) in [(0x38, VK_LMENU), (0xb8, VK_RMENU)].into_iter().enumerate() {
            if alt_pressed[side]
                && self.held[offset]
                && unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) } == self.hwnd
            {
                let up = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: vk,
                            wScan: 0x7f,
                            dwFlags: KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                };
                stats.local_releases +=
                    unsafe { SendInput(&[up], size_of::<INPUT>() as i32) } as u64;
            }
        }
        Ok(changes)
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.Unacquire();
        }
    }
}

fn transitions(
    events: &[DIDEVICEOBJECTDATA],
    held_keys: &mut [bool; 256],
    stats: &mut AltStats,
) -> (Vec<Key>, [bool; 2]) {
    let mut alt_pressed = [false; 2];
    let changes = events
        .iter()
        .filter_map(|e| {
            let index = e.dwOfs as usize;
            let down = e.dwData & 0x80 != 0;
            if index == 0x7f && !down {
                stats.tagged_releases += 1;
            }
            if index == 0x0f && down && (held_keys[0x38] || held_keys[0xb8]) {
                stats.alt_tabs += 1;
            }
            let held = held_keys.get_mut(index)?;
            if *held == down {
                return None;
            }
            *held = down;
            if down && (index == 0x38 || index == 0xb8) {
                alt_pressed[(index == 0xb8) as usize] = true;
            }
            keymap::code(e.dwOfs).map(|code| (code, down))
        })
        .collect();
    (changes, alt_pressed)
}

#[derive(Default, serde::Serialize)]
pub struct AltStats {
    local_releases: u64,
    tagged_releases: u64,
    alt_tabs: u64,
}

pub struct Capture {
    pub alt: AltStats,
    hwnd: HWND,
    keyboard: Option<Keyboard>,
    pub acquisitions: u64,
    pub last_error: Option<String>,
}

impl Capture {
    pub fn new(hwnd: isize) -> Self {
        Self {
            alt: AltStats::default(),
            hwnd: HWND(hwnd as *mut _),
            keyboard: None,
            acquisitions: 0,
            last_error: None,
        }
    }
    pub fn is_foreground(&self) -> bool {
        unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) == self.hwnd }
    }
    pub fn enabled(&self) -> bool {
        self.keyboard.is_some()
    }
    pub fn acquire(&mut self) {
        match Keyboard::acquire(self.hwnd) {
            Ok(device) => {
                self.keyboard = Some(device);
                self.acquisitions += 1;
                self.last_error = None;
            }
            Err(error) => self.last_error = Some(format!("{error:#}")),
        }
    }
    pub fn release(&mut self) {
        self.keyboard = None;
    }
    pub fn read(&mut self) -> Result<Vec<Key>> {
        match &mut self.keyboard {
            Some(keyboard) => keyboard.read(&mut self.alt),
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_alt_release_does_not_release_remote_alt() {
        let mut held = [false; 256];
        let mut stats = AltStats::default();
        let mut actual = Vec::new();
        for (offset, pressed) in [
            (0x38, true),
            (0x7f, false),
            (0x0f, true),
            (0x0f, true),
            (0x0f, false),
            (0x38, false),
        ] {
            let event = DIDEVICEOBJECTDATA {
                dwOfs: offset,
                dwData: if pressed { 0x80 } else { 0 },
                ..Default::default()
            };
            actual.extend(transitions(&[event], &mut held, &mut stats).0);
        }
        assert_eq!(
            actual,
            [
                ("AltLeft", true),
                ("Tab", true),
                ("Tab", false),
                ("AltLeft", false)
            ]
        );
    }

    #[test]
    fn buffered_dik_chords_keep_both_sides_of_modifiers() {
        for (dik, code) in [
            (0xdb, "MetaLeft"),
            (0xdc, "MetaRight"),
            (0x2f, "KeyV"),
            (0x38, "AltLeft"),
            (0xb8, "AltRight"),
            (0x0f, "Tab"),
            (0x1d, "ControlLeft"),
            (0x9d, "ControlRight"),
            (0x11, "KeyW"),
            (0x56, "IntlBackslash"),
        ] {
            assert_eq!(keymap::code(dik), Some(code));
        }
    }
}

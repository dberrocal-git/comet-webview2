//! Foreground, exclusive Windows keyboard capture and physical key translation.
//!
//! `app` polls [`Capture::read`] on its event-loop thread and forwards the ordered
//! `(KeyboardEvent.code, down)` pairs to `bridge.js` in the remote WebView. This
//! module owns only the local DirectInput device; the bridge owns the set of keys
//! sent to the KVM. Releasing capture must therefore also call the bridge's
//! `setCaptured(false)` to send remote key-ups (see `App::release`).
//!
//! Acquisition is explicit through a console click or toolbar action. Focus loss,
//! read failures and buffer overflow end the session rather than reacquiring
//! with an uncertain held-key state. The application may use trusted DOM typing
//! as a fallback when exclusive acquisition fails; this module has no DOM path.

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

/// A physical web key code and its pressed state; `true` is key-down.
type Key = (&'static str, bool);

/// One acquired DirectInput session, with a fresh held-key ledger.
///
/// Keep the parent COM interface alive for as long as its keyboard device. All
/// calls are made by the application event loop, including destruction.
struct Keyboard {
    hwnd: HWND,
    device: IDirectInputDevice8W,
    _input: IDirectInput8W,
    held: [bool; 256],
}

impl Keyboard {
    /// Acquire the system keyboard exclusively while our top-level window is
    /// foreground. Errors propagate so the caller can select basic typing mode.
    fn acquire(hwnd: HWND) -> Result<Self> {
        // SAFETY: from_raw adopts the interface returned by a successful COM
        // creation. SDK structures and property buffers stay live during calls.
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
            // Buffer transitions rather than sampling current state: short
            // presses and modifier ordering must survive between event-loop polls.
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

    /// Drain one batch in device order. Any input loss is fatal to this session:
    /// continuing could leave a modifier held on the remote computer indefinitely.
    fn read(&mut self, stats: &mut AltStats) -> Result<Vec<Key>> {
        let mut events = [DIDEVICEOBJECTDATA::default(); 64];
        let mut count = events.len() as u32;
        // The projected GetDeviceData function discards the success code that
        // indicates overflow. A lost key-up must end the capture session.
        // SAFETY: The device is live; count bounds the output array and the element
        // size matches DIDEVICEOBJECTDATA. The API updates count with records read.
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

/// Turn buffered device records into physical key transitions, without repeats.
///
/// The ledger covers all 256 DIK positions, including unmapped keys. Only mapped
/// positions are forwarded. The second result marks physical Alt downs in this
/// batch so `read` can neutralize the local shell's logical Alt state afterward.
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

/// Cumulative diagnostics for the local Alt workaround, not a capture state.
///
/// `local_releases` counts inputs accepted by `SendInput`, `tagged_releases`
/// counts observed synthetic breaks at 0x7f, and `alt_tabs` counts Tab-down
/// records received while the physical Alt ledger is set (including repeats).
#[derive(Default, serde::Serialize)]
pub struct AltStats {
    local_releases: u64,
    tagged_releases: u64,
    alt_tabs: u64,
}

/// Local capture owner and diagnostics, retained across acquisition sessions.
///
/// `enabled()` reports exclusive acquisition only. The application's separate
/// `capture_requested` flag also covers its DOM fallback and must not be inferred
/// from this value. This type does not inject events into the remote WebView.
pub struct Capture {
    pub alt: AltStats,
    hwnd: HWND,
    keyboard: Option<Keyboard>,
    pub acquisitions: u64,
    pub last_error: Option<String>,
}

impl Capture {
    /// Bind to the host's top-level HWND without acquiring the keyboard yet.
    pub fn new(hwnd: isize) -> Self {
        Self {
            alt: AltStats::default(),
            hwnd: HWND(hwnd as *mut _),
            keyboard: None,
            acquisitions: 0,
            last_error: None,
        }
    }
    /// Accept focus in any child WebView belonging to the host. A child gaining
    /// focus can blur the Tao window without the user leaving the application.
    pub fn is_foreground(&self) -> bool {
        unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) == self.hwnd }
    }
    /// Whether an exclusive DirectInput session is currently owned.
    pub fn enabled(&self) -> bool {
        self.keyboard.is_some()
    }
    /// Attempt acquisition and retain errors for diagnostics. The app calls this
    /// only when inactive; on failure it can still enable its DOM typing fallback.
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
    /// Unacquire locally by dropping the device. The caller must separately ask
    /// the bridge to release keys already delivered to the remote computer.
    pub fn release(&mut self) {
        self.keyboard = None;
    }
    /// Return an empty batch when inactive; propagate device errors unchanged so
    /// the application ends capture and releases the bridge's remote key ledger.
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

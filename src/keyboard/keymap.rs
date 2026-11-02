//! The vocabulary shared by DirectInput capture and the firmware keyboard bridge.
//!
//! DIK offsets identify physical positions, not characters or Win32 virtual keys.
//! Preserve left/right modifiers, numpad keys and extended navigation keys: the
//! remote computer's keyboard layout decides what a physical press means. Names
//! match web `KeyboardEvent.code` values used by `remote_keyboard_event`.

/// Map a DirectInput keyboard offset to a physical key name used by the KVM.
///
/// Return `None` for unsupported positions instead of inventing a character or
/// forwarding an unknown name. In particular, 0x7f is reserved by this app for the
/// synthetic local Alt-up and must never become a remote Alt-up. When extending
/// this table, check the firmware accepts the code and retain physical distinctions.
pub fn code(offset: u32) -> Option<&'static str> {
    Some(match offset {
        0x01 => "Escape",
        0x02 => "Digit1",
        0x03 => "Digit2",
        0x04 => "Digit3",
        0x05 => "Digit4",
        0x06 => "Digit5",
        0x07 => "Digit6",
        0x08 => "Digit7",
        0x09 => "Digit8",
        0x0a => "Digit9",
        0x0b => "Digit0",
        0x0c => "Minus",
        0x0d => "Equal",
        0x0e => "Backspace",
        0x0f => "Tab",
        0x10 => "KeyQ",
        0x11 => "KeyW",
        0x12 => "KeyE",
        0x13 => "KeyR",
        0x14 => "KeyT",
        0x15 => "KeyY",
        0x16 => "KeyU",
        0x17 => "KeyI",
        0x18 => "KeyO",
        0x19 => "KeyP",
        0x1a => "BracketLeft",
        0x1b => "BracketRight",
        0x1c => "Enter",
        0x1d => "ControlLeft",
        0x1e => "KeyA",
        0x1f => "KeyS",
        0x20 => "KeyD",
        0x21 => "KeyF",
        0x22 => "KeyG",
        0x23 => "KeyH",
        0x24 => "KeyJ",
        0x25 => "KeyK",
        0x26 => "KeyL",
        0x27 => "Semicolon",
        0x28 => "Quote",
        0x29 => "Backquote",
        0x2a => "ShiftLeft",
        0x2b => "Backslash",
        0x2c => "KeyZ",
        0x2d => "KeyX",
        0x2e => "KeyC",
        0x2f => "KeyV",
        0x30 => "KeyB",
        0x31 => "KeyN",
        0x32 => "KeyM",
        0x33 => "Comma",
        0x34 => "Period",
        0x35 => "Slash",
        0x36 => "ShiftRight",
        0x37 => "NumpadMultiply",
        0x38 => "AltLeft",
        0x39 => "Space",
        0x3a => "CapsLock",
        0x3b => "F1",
        0x3c => "F2",
        0x3d => "F3",
        0x3e => "F4",
        0x3f => "F5",
        0x40 => "F6",
        0x41 => "F7",
        0x42 => "F8",
        0x43 => "F9",
        0x44 => "F10",
        0x45 => "NumLock",
        0x46 => "ScrollLock",
        0x47 => "Numpad7",
        0x48 => "Numpad8",
        0x49 => "Numpad9",
        0x4a => "NumpadSubtract",
        0x4b => "Numpad4",
        0x4c => "Numpad5",
        0x4d => "Numpad6",
        0x4e => "NumpadAdd",
        0x4f => "Numpad1",
        0x50 => "Numpad2",
        0x51 => "Numpad3",
        0x52 => "Numpad0",
        0x53 => "NumpadDecimal",
        0x56 => "IntlBackslash",
        0x57 => "F11",
        0x58 => "F12",
        0x64 => "F13",
        0x65 => "F14",
        0x66 => "F15",
        0x67 => "F16",
        0x68 => "F17",
        0x69 => "F18",
        0x6a => "F19",
        0x6b => "F20",
        0x6c => "F21",
        0x6d => "F22",
        0x6e => "F23",
        0x6f => "F24",
        0x9c => "NumpadEnter",
        0x9d => "ControlRight",
        0xb5 => "NumpadDivide",
        0xb7 => "PrintScreen",
        0xb8 => "AltRight",
        0xc5 => "Pause",
        0xc7 => "Home",
        0xc8 => "ArrowUp",
        0xc9 => "PageUp",
        0xcb => "ArrowLeft",
        0xcd => "ArrowRight",
        0xcf => "End",
        0xd0 => "ArrowDown",
        0xd1 => "PageDown",
        0xd2 => "Insert",
        0xd3 => "Delete",
        0xdb => "MetaLeft",
        0xdc => "MetaRight",
        0xdd => "ContextMenu",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_layout_and_extended_keys() {
        assert_eq!(code(0x27), Some("Semicolon"));
        assert_eq!(code(0x56), Some("IntlBackslash"));
        assert_eq!(code(0x9c), Some("NumpadEnter"));
        assert_eq!(code(0x47), Some("Numpad7"));
        assert_eq!(code(0xc7), Some("Home"));
        assert_eq!(code(0x6f), Some("F24"));
        assert_eq!(code(0x7f), None); // Synthetic local Alt release.
    }
}

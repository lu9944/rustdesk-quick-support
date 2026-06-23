// Input injection: MouseEvent / KeyEvent -> OS events via enigo.
// Mirrors rustdesk_QS's input_service (chr -> Key::Layout, unicode/seq ->
// key_sequence, control_key via KEY_MAP), which is the proven working path on
// Windows (rdev's simulate is unreliable here).
use crate::proto_gen::message::{key_event::Union as KU, ControlKey as CK, KeyEvent, MouseEvent};
use enigo::{Enigo, Key, KeyboardControllable, MouseButton, MouseControllable};
use once_cell::sync::Lazy;
use std::sync::Mutex;

static ENIGO: Lazy<Mutex<Enigo>> = Lazy::new(|| Mutex::new(Enigo::new()));

fn enigo() -> std::sync::MutexGuard<'static, Enigo> {
    ENIGO.lock().unwrap_or_else(|e| e.into_inner())
}

const MOUSE_TYPE_MASK: i32 = 0x7;
const MOUSE_TYPE_MOVE: i32 = 0;
const MOUSE_TYPE_DOWN: i32 = 1;
const MOUSE_TYPE_UP: i32 = 2;
const MOUSE_TYPE_WHEEL: i32 = 3;

const MOUSE_BUTTON_LEFT: i32 = 0x01;
const MOUSE_BUTTON_RIGHT: i32 = 0x02;
const MOUSE_BUTTON_WHEEL: i32 = 0x04;

pub fn handle_mouse(evt: &MouseEvent) {
    let evt_type = evt.mask & MOUSE_TYPE_MASK;
    let buttons = evt.mask >> 3;
    let mut en = enigo();
    match evt_type {
        MOUSE_TYPE_MOVE => en.mouse_move_to(evt.x, evt.y),
        MOUSE_TYPE_DOWN => {
            if let Some(b) = map_button(buttons) {
                en.mouse_down(b).ok();
            }
        }
        MOUSE_TYPE_UP => {
            if let Some(b) = map_button(buttons) {
                en.mouse_up(b);
            }
        }
        MOUSE_TYPE_WHEEL => {
            en.mouse_scroll_x(evt.x);
            en.mouse_scroll_y(evt.y);
        }
        _ => {}
    }
}

fn map_button(buttons: i32) -> Option<MouseButton> {
    match buttons {
        MOUSE_BUTTON_LEFT => Some(MouseButton::Left),
        MOUSE_BUTTON_RIGHT => Some(MouseButton::Right),
        MOUSE_BUTTON_WHEEL => Some(MouseButton::Middle),
        _ => None,
    }
}

pub fn handle_key(evt: &KeyEvent) {
    let mut en = enigo();
    let down = evt.down;

    // Sync CapsLock with the controller so letter case (and IME behavior) match.
    let has_cap = evt
        .modifiers
        .iter()
        .any(|m| m.enum_value_or_default() == CK::CapsLock);
    if down && has_cap != en.get_key_state(Key::CapsLock) {
        let _ = en.key_down(Key::CapsLock);
        en.key_up(Key::CapsLock);
    }

    // Press Shift/Control/Alt/Meta modifiers that aren't already held, so
    // shortcuts (Ctrl+A, etc.) and Shift selection work. Release afterwards.
    let mut to_release: Vec<Key> = Vec::new();
    if down {
        for m in &evt.modifiers {
            if let Some(k) = modifier_to_enigo(m.enum_value_or_default()) {
                if !en.get_key_state(k.clone()) {
                    let _ = en.key_down(k.clone());
                    to_release.push(k);
                }
            }
        }
    }

    match &evt.union {
        Some(KU::ControlKey(ck)) => {
            if let Some(key) = control_key_to_enigo(ck.enum_value_or_default()) {
                if down {
                    let _ = en.key_down(key);
                } else {
                    en.key_up(key);
                }
            }
        }
        Some(KU::Chr(chr)) => {
            // Use key_down/up(Key::Layout) which sends WM_KEYDOWN with a VK,
            // letting the IME intercept & compose Chinese. enigo translates the
            // exact char (Layout('A')->'A', 'a'->'a'), so case is preserved.
            let key = Key::Layout(char::from_u32(*chr).unwrap_or('\0'));
            if down {
                let _ = en.key_down(key);
            } else {
                en.key_up(key);
            }
        }
        Some(KU::Unicode(chr)) => {
            if down {
                if let Ok(c) = char::try_from(*chr) {
                    en.key_sequence(&c.to_string());
                }
            }
        }
        Some(KU::Seq(seq)) => {
            if down {
                en.key_sequence(seq);
            }
        }
        Some(KU::Win2winHotkey(_)) => {}
        None => {}
    }

    for k in to_release.into_iter().rev() {
        en.key_up(k);
    }
}

fn modifier_to_enigo(k: CK) -> Option<Key> {
    Some(match k {
        CK::Alt | CK::Option => Key::Alt,
        CK::RAlt => Key::RightAlt,
        CK::Control => Key::Control,
        CK::RControl => Key::RightControl,
        CK::Shift => Key::Shift,
        CK::RShift => Key::RightShift,
        CK::Meta | CK::RWin => Key::Meta,
        _ => return None,
    })
}

fn control_key_to_enigo(k: CK) -> Option<Key> {
    Some(match k {
        CK::Alt | CK::Option => Key::Alt,
        CK::RAlt => Key::RightAlt,
        CK::Backspace => Key::Backspace,
        CK::CapsLock => Key::CapsLock,
        CK::Control => Key::Control,
        CK::RControl => Key::RightControl,
        CK::Delete => Key::Delete,
        CK::DownArrow => Key::DownArrow,
        CK::End => Key::End,
        CK::Escape => Key::Escape,
        CK::Home => Key::Home,
        CK::LeftArrow => Key::LeftArrow,
        CK::Meta | CK::RWin => Key::Meta,
        CK::PageDown => Key::PageDown,
        CK::PageUp => Key::PageUp,
        CK::Return => Key::Return,
        CK::RightArrow => Key::RightArrow,
        CK::Shift => Key::Shift,
        CK::RShift => Key::RightShift,
        CK::Space => Key::Space,
        CK::Tab => Key::Tab,
        CK::UpArrow => Key::UpArrow,
        CK::Insert => Key::Insert,
        CK::Scroll => Key::Scroll,
        CK::NumLock => Key::NumLock,
        CK::Pause => Key::Pause,
        CK::Snapshot => Key::Snapshot,
        CK::F1 => Key::F1,
        CK::F2 => Key::F2,
        CK::F3 => Key::F3,
        CK::F4 => Key::F4,
        CK::F5 => Key::F5,
        CK::F6 => Key::F6,
        CK::F7 => Key::F7,
        CK::F8 => Key::F8,
        CK::F9 => Key::F9,
        CK::F10 => Key::F10,
        CK::F11 => Key::F11,
        CK::F12 => Key::F12,
        CK::Numpad0 => Key::Numpad0,
        CK::Numpad1 => Key::Numpad1,
        CK::Numpad2 => Key::Numpad2,
        CK::Numpad3 => Key::Numpad3,
        CK::Numpad4 => Key::Numpad4,
        CK::Numpad5 => Key::Numpad5,
        CK::Numpad6 => Key::Numpad6,
        CK::Numpad7 => Key::Numpad7,
        CK::Numpad8 => Key::Numpad8,
        CK::Numpad9 => Key::Numpad9,
        CK::VolumeUp => Key::VolumeUp,
        CK::VolumeDown => Key::VolumeDown,
        CK::VolumeMute => Key::Mute,
        _ => return None,
    })
}

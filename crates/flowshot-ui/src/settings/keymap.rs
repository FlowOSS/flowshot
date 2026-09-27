//! Bidirectional `winit` 0.30 <-> `egui` 0.28 key mapping.
//!
//! egui-winit is deliberately NOT a dependency (its 0.28 release requires
//! winit ^0.29 while the workspace pins winit 0.30, and no egui release
//! pairs wgpu 0.20 with winit 0.30 - the plan's recorded fallback: feed
//! egui manually). This module mirrors the egui-winit 0.28 conversion
//! semantics: logical key first (correct on non-Latin layouts, egui#3653),
//! physical code as fallback, numpad merged into its main-cluster
//! counterpart. The reverse direction (egui key -> winit code) serves the
//! shortcuts recorder, which stores bindings as `KeyCode` (the
//! [`crate::editor::ToolShortcuts`] seam type); keys with no physical
//! source (Colon/Pipe/Questionmark) are unbindable and map to `None`.

use egui::Key;
use winit::keyboard::{Key as WinitKey, KeyCode, NamedKey, PhysicalKey};

/// Physical code -> egui key, in egui-winit 0.28 order; the reverse lookup
/// is first-match-wins, so main-cluster codes precede numpad aliases.
const PHYSICAL_TABLE: &[(KeyCode, Key)] = &[
    (KeyCode::ArrowDown, Key::ArrowDown),
    (KeyCode::ArrowLeft, Key::ArrowLeft),
    (KeyCode::ArrowRight, Key::ArrowRight),
    (KeyCode::ArrowUp, Key::ArrowUp),
    (KeyCode::Escape, Key::Escape),
    (KeyCode::Tab, Key::Tab),
    (KeyCode::Backspace, Key::Backspace),
    (KeyCode::Enter, Key::Enter),
    (KeyCode::NumpadEnter, Key::Enter),
    (KeyCode::Insert, Key::Insert),
    (KeyCode::Delete, Key::Delete),
    (KeyCode::Home, Key::Home),
    (KeyCode::End, Key::End),
    (KeyCode::PageUp, Key::PageUp),
    (KeyCode::PageDown, Key::PageDown),
    (KeyCode::Space, Key::Space),
    (KeyCode::Comma, Key::Comma),
    (KeyCode::Period, Key::Period),
    (KeyCode::Semicolon, Key::Semicolon),
    (KeyCode::Backslash, Key::Backslash),
    (KeyCode::Slash, Key::Slash),
    (KeyCode::NumpadDivide, Key::Slash),
    (KeyCode::BracketLeft, Key::OpenBracket),
    (KeyCode::BracketRight, Key::CloseBracket),
    (KeyCode::Backquote, Key::Backtick),
    (KeyCode::Quote, Key::Quote),
    (KeyCode::Cut, Key::Cut),
    (KeyCode::Copy, Key::Copy),
    (KeyCode::Paste, Key::Paste),
    (KeyCode::Minus, Key::Minus),
    (KeyCode::NumpadSubtract, Key::Minus),
    (KeyCode::NumpadAdd, Key::Plus),
    (KeyCode::Equal, Key::Equals),
    (KeyCode::Digit0, Key::Num0),
    (KeyCode::Numpad0, Key::Num0),
    (KeyCode::Digit1, Key::Num1),
    (KeyCode::Numpad1, Key::Num1),
    (KeyCode::Digit2, Key::Num2),
    (KeyCode::Numpad2, Key::Num2),
    (KeyCode::Digit3, Key::Num3),
    (KeyCode::Numpad3, Key::Num3),
    (KeyCode::Digit4, Key::Num4),
    (KeyCode::Numpad4, Key::Num4),
    (KeyCode::Digit5, Key::Num5),
    (KeyCode::Numpad5, Key::Num5),
    (KeyCode::Digit6, Key::Num6),
    (KeyCode::Numpad6, Key::Num6),
    (KeyCode::Digit7, Key::Num7),
    (KeyCode::Numpad7, Key::Num7),
    (KeyCode::Digit8, Key::Num8),
    (KeyCode::Numpad8, Key::Num8),
    (KeyCode::Digit9, Key::Num9),
    (KeyCode::Numpad9, Key::Num9),
    (KeyCode::KeyA, Key::A),
    (KeyCode::KeyB, Key::B),
    (KeyCode::KeyC, Key::C),
    (KeyCode::KeyD, Key::D),
    (KeyCode::KeyE, Key::E),
    (KeyCode::KeyF, Key::F),
    (KeyCode::KeyG, Key::G),
    (KeyCode::KeyH, Key::H),
    (KeyCode::KeyI, Key::I),
    (KeyCode::KeyJ, Key::J),
    (KeyCode::KeyK, Key::K),
    (KeyCode::KeyL, Key::L),
    (KeyCode::KeyM, Key::M),
    (KeyCode::KeyN, Key::N),
    (KeyCode::KeyO, Key::O),
    (KeyCode::KeyP, Key::P),
    (KeyCode::KeyQ, Key::Q),
    (KeyCode::KeyR, Key::R),
    (KeyCode::KeyS, Key::S),
    (KeyCode::KeyT, Key::T),
    (KeyCode::KeyU, Key::U),
    (KeyCode::KeyV, Key::V),
    (KeyCode::KeyW, Key::W),
    (KeyCode::KeyX, Key::X),
    (KeyCode::KeyY, Key::Y),
    (KeyCode::KeyZ, Key::Z),
    (KeyCode::F1, Key::F1),
    (KeyCode::F2, Key::F2),
    (KeyCode::F3, Key::F3),
    (KeyCode::F4, Key::F4),
    (KeyCode::F5, Key::F5),
    (KeyCode::F6, Key::F6),
    (KeyCode::F7, Key::F7),
    (KeyCode::F8, Key::F8),
    (KeyCode::F9, Key::F9),
    (KeyCode::F10, Key::F10),
    (KeyCode::F11, Key::F11),
    (KeyCode::F12, Key::F12),
    (KeyCode::F13, Key::F13),
    (KeyCode::F14, Key::F14),
    (KeyCode::F15, Key::F15),
    (KeyCode::F16, Key::F16),
    (KeyCode::F17, Key::F17),
    (KeyCode::F18, Key::F18),
    (KeyCode::F19, Key::F19),
    (KeyCode::F20, Key::F20),
    (KeyCode::F21, Key::F21),
    (KeyCode::F22, Key::F22),
    (KeyCode::F23, Key::F23),
    (KeyCode::F24, Key::F24),
    (KeyCode::F25, Key::F25),
    (KeyCode::F26, Key::F26),
    (KeyCode::F27, Key::F27),
    (KeyCode::F28, Key::F28),
    (KeyCode::F29, Key::F29),
    (KeyCode::F30, Key::F30),
    (KeyCode::F31, Key::F31),
    (KeyCode::F32, Key::F32),
    (KeyCode::F33, Key::F33),
    (KeyCode::F34, Key::F34),
    (KeyCode::F35, Key::F35),
];

/// The egui key for a physical code, when egui models it.
pub(super) fn egui_key_from_code(code: KeyCode) -> Option<Key> {
    PHYSICAL_TABLE
        .iter()
        .find(|(physical, _)| *physical == code)
        .map(|(_, key)| *key)
}

/// The winit code an egui key came from (recorder direction; first table
/// match wins, so numpad aliases resolve to the main cluster). `None` for
/// keys with no physical source - they cannot back a `KeyCode` binding.
pub(super) fn code_from_egui_key(key: Key) -> Option<KeyCode> {
    PHYSICAL_TABLE
        .iter()
        .find(|(_, mapped)| *mapped == key)
        .map(|(code, _)| *code)
}

/// The egui key for a logical (keymap-resolved) winit key.
pub(super) fn egui_key_from_logical(logical: &WinitKey) -> Option<Key> {
    match logical {
        WinitKey::Named(named) => egui_key_from_named(*named),
        WinitKey::Character(text) => Key::from_name(text.as_str()),
        WinitKey::Unidentified(_) | WinitKey::Dead(_) => None,
    }
}

fn egui_key_from_named(named: NamedKey) -> Option<Key> {
    Some(match named {
        NamedKey::Enter => Key::Enter,
        NamedKey::Tab => Key::Tab,
        NamedKey::ArrowDown => Key::ArrowDown,
        NamedKey::ArrowLeft => Key::ArrowLeft,
        NamedKey::ArrowRight => Key::ArrowRight,
        NamedKey::ArrowUp => Key::ArrowUp,
        NamedKey::End => Key::End,
        NamedKey::Home => Key::Home,
        NamedKey::PageDown => Key::PageDown,
        NamedKey::PageUp => Key::PageUp,
        NamedKey::Backspace => Key::Backspace,
        NamedKey::Delete => Key::Delete,
        NamedKey::Insert => Key::Insert,
        NamedKey::Escape => Key::Escape,
        NamedKey::Cut => Key::Cut,
        NamedKey::Copy => Key::Copy,
        NamedKey::Paste => Key::Paste,
        NamedKey::Space => Key::Space,
        NamedKey::F1 => Key::F1,
        NamedKey::F2 => Key::F2,
        NamedKey::F3 => Key::F3,
        NamedKey::F4 => Key::F4,
        NamedKey::F5 => Key::F5,
        NamedKey::F6 => Key::F6,
        NamedKey::F7 => Key::F7,
        NamedKey::F8 => Key::F8,
        NamedKey::F9 => Key::F9,
        NamedKey::F10 => Key::F10,
        NamedKey::F11 => Key::F11,
        NamedKey::F12 => Key::F12,
        NamedKey::F13 => Key::F13,
        NamedKey::F14 => Key::F14,
        NamedKey::F15 => Key::F15,
        NamedKey::F16 => Key::F16,
        NamedKey::F17 => Key::F17,
        NamedKey::F18 => Key::F18,
        NamedKey::F19 => Key::F19,
        NamedKey::F20 => Key::F20,
        NamedKey::F21 => Key::F21,
        NamedKey::F22 => Key::F22,
        NamedKey::F23 => Key::F23,
        NamedKey::F24 => Key::F24,
        NamedKey::F25 => Key::F25,
        NamedKey::F26 => Key::F26,
        NamedKey::F27 => Key::F27,
        NamedKey::F28 => Key::F28,
        NamedKey::F29 => Key::F29,
        NamedKey::F30 => Key::F30,
        NamedKey::F31 => Key::F31,
        NamedKey::F32 => Key::F32,
        NamedKey::F33 => Key::F33,
        NamedKey::F34 => Key::F34,
        NamedKey::F35 => Key::F35,
        _ => return None,
    })
}

/// The egui key for a key event: logical first, physical as the non-Latin
/// fallback (egui-winit semantics).
pub(super) fn egui_key_for_event(
    logical: &WinitKey,
    physical: PhysicalKey,
) -> Option<(Key, Option<Key>)> {
    let physical_key = match physical {
        PhysicalKey::Code(code) => egui_key_from_code(code),
        PhysicalKey::Unidentified(_) => None,
    };
    egui_key_from_logical(logical)
        .or(physical_key)
        .map(|active| (active, physical_key))
}

//! The editor key map (plan todo 20: "tool activation keys per F12 shortcut
//! map (P/D/A/S/R/C/M/T/B/I + configurable)").
//!
//! Defaults are the Flameshot `recognizedShortcuts` table
//! (`confighandler.cpp` L156+): single unmodified letters activate tools,
//! undo = Ctrl+Z, redo = Ctrl+Shift+Z (`TYPE_UNDO`/`TYPE_REDO`). The map is
//! DATA: [`ToolShortcuts::rebind`] is the configurable seam the settings
//! surface (todo 36) and the `[shortcuts]` config group write into. Tool
//! keys require EMPTY modifiers by design - Ctrl/Shift combos stay free for
//! the selection engine (Ctrl+C copy, Ctrl+A select-all, Ctrl+Q exit) and
//! shift+letter produces symbols on many layouts.

use winit::keyboard::KeyCode;

use super::kind::ToolKind;

/// The rebindable editor key map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolShortcuts {
    bindings: Vec<(ToolKind, KeyCode)>,
    undo: KeyCode,
    redo: KeyCode,
}

impl Default for ToolShortcuts {
    fn default() -> Self {
        Self {
            bindings: ToolKind::ALL
                .into_iter()
                .filter_map(|kind| kind.default_key().map(|key| (kind, key)))
                .collect(),
            undo: KeyCode::KeyZ,
            redo: KeyCode::KeyZ,
        }
    }
}

impl ToolShortcuts {
    /// The tool a plain (unmodified) key press activates, when bound.
    #[must_use]
    pub fn tool_for_key(&self, code: KeyCode) -> Option<ToolKind> {
        self.bindings
            .iter()
            .find(|(_, key)| *key == code)
            .map(|(kind, _)| *kind)
    }

    /// Rebinds a tool's activation key (`None` unbinds). Configurable-seam
    /// (todo 36 settings / `[shortcuts]` group).
    pub fn rebind(&mut self, kind: ToolKind, key: Option<KeyCode>) {
        self.bindings.retain(|(existing, _)| *existing != kind);
        if let Some(key) = key {
            self.bindings.push((kind, key));
        }
    }

    /// The current binding of a tool kind.
    #[must_use]
    pub fn key_for_tool(&self, kind: ToolKind) -> Option<KeyCode> {
        self.bindings
            .iter()
            .find(|(existing, _)| *existing == kind)
            .map(|(_, key)| *key)
    }

    /// Whether `code` with Ctrl (no Shift) is the undo binding.
    #[must_use]
    pub fn is_undo(&self, code: KeyCode) -> bool {
        code == self.undo
    }

    /// Whether `code` with Ctrl+Shift is the redo binding.
    #[must_use]
    pub fn is_redo(&self, code: KeyCode) -> bool {
        code == self.redo
    }

    /// Rebinds undo/redo (they share the Ctrl / Ctrl+Shift modifier
    /// convention; todo 25 wires the config `[shortcuts]` group).
    pub fn rebind_undo_redo(&mut self, undo: KeyCode, redo: KeyCode) {
        self.undo = undo;
        self.redo = redo;
    }
}

/// The digit a number key carries (top row or numpad; Flameshot accepts
/// `Qt::NoModifier` and `Qt::KeypadModifier` - winit reports the numpad as
/// its own physical codes).
#[must_use]
pub fn digit_for(code: KeyCode) -> Option<u32> {
    let digit = match code {
        KeyCode::Digit0 | KeyCode::Numpad0 => 0,
        KeyCode::Digit1 | KeyCode::Numpad1 => 1,
        KeyCode::Digit2 | KeyCode::Numpad2 => 2,
        KeyCode::Digit3 | KeyCode::Numpad3 => 3,
        KeyCode::Digit4 | KeyCode::Numpad4 => 4,
        KeyCode::Digit5 | KeyCode::Numpad5 => 5,
        KeyCode::Digit6 | KeyCode::Numpad6 => 6,
        KeyCode::Digit7 | KeyCode::Numpad7 => 7,
        KeyCode::Digit8 | KeyCode::Numpad8 => 8,
        KeyCode::Digit9 | KeyCode::Numpad9 => 9,
        _ => return None,
    };
    Some(digit)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn default_map_is_the_f12_table() {
        let shortcuts = ToolShortcuts::default();
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyP),
            Some(ToolKind::Pencil)
        );
        assert_eq!(shortcuts.tool_for_key(KeyCode::KeyD), Some(ToolKind::Line));
        assert_eq!(shortcuts.tool_for_key(KeyCode::KeyA), Some(ToolKind::Arrow));
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyS),
            Some(ToolKind::Selection)
        );
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyR),
            Some(ToolKind::Rectangle)
        );
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyC),
            Some(ToolKind::Circle)
        );
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyM),
            Some(ToolKind::Marker)
        );
        assert_eq!(shortcuts.tool_for_key(KeyCode::KeyT), Some(ToolKind::Text));
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyB),
            Some(ToolKind::Pixelate)
        );
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyI),
            Some(ToolKind::Invert)
        );
        assert_eq!(shortcuts.tool_for_key(KeyCode::KeyQ), None);
        assert!(shortcuts.is_undo(KeyCode::KeyZ));
        assert!(shortcuts.is_redo(KeyCode::KeyZ));
    }

    #[test]
    fn rebind_moves_and_unbinds() {
        let mut shortcuts = ToolShortcuts::default();
        shortcuts.rebind(ToolKind::Pencil, Some(KeyCode::KeyX));
        assert_eq!(shortcuts.tool_for_key(KeyCode::KeyP), None);
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyX),
            Some(ToolKind::Pencil)
        );
        assert_eq!(
            shortcuts.key_for_tool(ToolKind::Pencil),
            Some(KeyCode::KeyX)
        );
        shortcuts.rebind(ToolKind::Pencil, None);
        assert_eq!(shortcuts.key_for_tool(ToolKind::Pencil), None);
        // Counter/move ship unbound; binding them works.
        assert_eq!(shortcuts.key_for_tool(ToolKind::Counter), None);
        shortcuts.rebind(ToolKind::Counter, Some(KeyCode::KeyN));
        assert_eq!(
            shortcuts.tool_for_key(KeyCode::KeyN),
            Some(ToolKind::Counter)
        );
        shortcuts.rebind_undo_redo(KeyCode::KeyU, KeyCode::KeyY);
        assert!(shortcuts.is_undo(KeyCode::KeyU));
        assert!(shortcuts.is_redo(KeyCode::KeyY));
        assert!(!shortcuts.is_undo(KeyCode::KeyZ));
    }

    #[test]
    fn digits_come_from_both_rows() {
        assert_eq!(digit_for(KeyCode::Digit0), Some(0));
        assert_eq!(digit_for(KeyCode::Digit7), Some(7));
        assert_eq!(digit_for(KeyCode::Digit9), Some(9));
        assert_eq!(digit_for(KeyCode::Numpad3), Some(3));
        assert_eq!(digit_for(KeyCode::KeyP), None);
        assert_eq!(digit_for(KeyCode::Minus), None);
    }
}

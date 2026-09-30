//! The editor key map: tool activation keys per the F12 shortcut
//! map (P/D/A/S/R/C/M/T/B/I + configurable).
//!
//! Defaults are the Flameshot `recognizedShortcuts` table
//! (`confighandler.cpp` L156+): single unmodified letters activate tools,
//! undo = Ctrl+Z, redo = Ctrl+Shift+Z (`TYPE_UNDO`/`TYPE_REDO`). The map is
//! DATA: [`ToolShortcuts::rebind`] is the configurable seam the settings
//! surface and the `[shortcuts]` config group write into. Tool
//! keys require EMPTY modifiers by design - Ctrl/Shift combos stay free for
//! the selection engine (Ctrl+C copy, Ctrl+A select-all, Ctrl+Q exit) and
//! shift+letter produces symbols on many layouts.

use winit::keyboard::KeyCode;

use super::kind::ToolKind;

/// A z-order step the key map can dispatch (panel-driven by
/// default - the key slots ship unbound).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZOrderAction {
    /// Raise the selected object one step.
    Raise,
    /// Lower the selected object one step.
    Lower,
}

/// An in-session view aid the key map toggles (the aid-indicator chips
/// read the SAME slots, so a rebind is reflected everywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AidToggle {
    /// The pixel magnifier (ships on `L`, lens - F12 binds no magnifier
    /// key, so `L` is the documented unbound-key choice).
    Magnifier,
    /// The snapping grid overlay (ships on `F`, the grid-F precedent).
    Grid,
}

/// The rebindable editor key map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolShortcuts {
    bindings: Vec<(ToolKind, KeyCode)>,
    undo: KeyCode,
    redo: KeyCode,
    raise: Option<KeyCode>,
    lower: Option<KeyCode>,
    magnifier: KeyCode,
    grid: KeyCode,
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
            raise: None,
            lower: None,
            magnifier: KeyCode::KeyL,
            grid: KeyCode::KeyF,
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
    /// (the settings surface / `[shortcuts]` group).
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
    /// convention; the config `[shortcuts]` group wires them).
    pub fn rebind_undo_redo(&mut self, undo: KeyCode, redo: KeyCode) {
        self.undo = undo;
        self.redo = redo;
    }

    /// The current undo key (Ctrl+…; a settings-surface read seam).
    #[must_use]
    pub const fn undo_key(&self) -> KeyCode {
        self.undo
    }

    /// The current redo key (Ctrl+Shift+…; settings-surface read seam).
    #[must_use]
    pub const fn redo_key(&self) -> KeyCode {
        self.redo
    }

    /// The current z-order bindings `(raise, lower)`; `None` = unbound
    /// (the shipped default).
    #[must_use]
    pub const fn z_keys(&self) -> (Option<KeyCode>, Option<KeyCode>) {
        (self.raise, self.lower)
    }

    /// The z-order action a plain (unmodified) key press dispatches;
    /// `None` while the slots ship unbound (the plan's panel-driven
    /// default - config/QA rebind only).
    #[must_use]
    pub fn z_for_key(&self, code: KeyCode) -> Option<ZOrderAction> {
        if self.raise == Some(code) {
            return Some(ZOrderAction::Raise);
        }
        if self.lower == Some(code) {
            return Some(ZOrderAction::Lower);
        }
        None
    }

    /// Rebinds the z-order keys (`None` unbinds - the shipped default).
    pub fn rebind_z_order(&mut self, raise: Option<KeyCode>, lower: Option<KeyCode>) {
        self.raise = raise;
        self.lower = lower;
    }

    /// The aid a plain (unmodified) key press toggles. Tool keys are
    /// dispatched FIRST by the event surface, so a tool binding shadows an
    /// aid on the same key; between the aids, a duplicate binding resolves
    /// to the magnifier (the first slot - the z-order duplicate rule).
    #[must_use]
    pub fn aid_for_key(&self, code: KeyCode) -> Option<AidToggle> {
        if code == self.magnifier {
            return Some(AidToggle::Magnifier);
        }
        if code == self.grid {
            return Some(AidToggle::Grid);
        }
        None
    }

    /// The current magnifier toggle key (the aid-chip read seam).
    #[must_use]
    pub const fn magnifier_key(&self) -> KeyCode {
        self.magnifier
    }

    /// The current grid toggle key (the aid-chip read seam).
    #[must_use]
    pub const fn grid_key(&self) -> KeyCode {
        self.grid
    }

    /// Rebinds the aid toggle keys (the settings-surface seam).
    pub fn rebind_aids(&mut self, magnifier: KeyCode, grid: KeyCode) {
        self.magnifier = magnifier;
        self.grid = grid;
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
    fn aid_keys_default_to_l_and_f_and_rebind() {
        let mut shortcuts = ToolShortcuts::default();
        assert_eq!(
            shortcuts.aid_for_key(KeyCode::KeyL),
            Some(AidToggle::Magnifier)
        );
        assert_eq!(shortcuts.aid_for_key(KeyCode::KeyF), Some(AidToggle::Grid));
        assert_eq!(shortcuts.aid_for_key(KeyCode::KeyP), None);
        assert_eq!(shortcuts.magnifier_key(), KeyCode::KeyL);
        assert_eq!(shortcuts.grid_key(), KeyCode::KeyF);

        shortcuts.rebind_aids(KeyCode::KeyV, KeyCode::KeyH);
        assert_eq!(shortcuts.aid_for_key(KeyCode::KeyL), None);
        assert_eq!(shortcuts.aid_for_key(KeyCode::KeyF), None);
        assert_eq!(
            shortcuts.aid_for_key(KeyCode::KeyV),
            Some(AidToggle::Magnifier)
        );
        assert_eq!(shortcuts.aid_for_key(KeyCode::KeyH), Some(AidToggle::Grid));
        assert_eq!(shortcuts.magnifier_key(), KeyCode::KeyV);
        assert_eq!(shortcuts.grid_key(), KeyCode::KeyH);
    }

    #[test]
    fn duplicate_aid_binding_resolves_to_the_magnifier() {
        let mut shortcuts = ToolShortcuts::default();
        shortcuts.rebind_aids(KeyCode::KeyK, KeyCode::KeyK);
        assert_eq!(
            shortcuts.aid_for_key(KeyCode::KeyK),
            Some(AidToggle::Magnifier)
        );
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

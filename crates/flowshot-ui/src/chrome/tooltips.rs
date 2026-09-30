//! Toolbar tooltip text: the short description + the ACTIVE key binding.
//!
//! The key always comes from the live [`ToolShortcuts`] (rebindable - a
//! rebind moves the tooltip) or from the selection engine's fixed action
//! chords; nothing here hardcodes a key the map does not carry. Unbound
//! buttons show the description alone (no empty brackets).

use winit::keyboard::KeyCode;

use crate::editor::{ToolKind, ToolShortcuts};

use super::toolbar::ToolbarButton;

/// The tooltip text for one toolbar button: `Name — description [Key]`.
#[must_use]
pub fn button_tooltip(button: &ToolbarButton, shortcuts: &ToolShortcuts) -> String {
    let (text, key) = match button {
        ToolbarButton::Tool(kind) => {
            let (name, blurb) = tool_blurb(*kind);
            (
                format!("{name} — {blurb}"),
                shortcuts.key_for_tool(*kind).map(chord),
            )
        }
        ToolbarButton::Action(id) => action_blurb(id, shortcuts),
    };
    match key {
        Some(key) => format!("{text} [{key}]"),
        None => text,
    }
}

/// The human name + short description of a tool (the tooltip's text half).
#[must_use]
pub fn tool_blurb(kind: ToolKind) -> (&'static str, &'static str) {
    match kind {
        ToolKind::Pencil => ("Pencil", "freehand draw"),
        ToolKind::Line => ("Line", "straight line"),
        ToolKind::Arrow => ("Arrow", "pointed arrow"),
        ToolKind::Selection => ("Selection", "adjust the region"),
        ToolKind::Rectangle => ("Rectangle", "rectangle outline"),
        ToolKind::Circle => ("Circle", "ellipse or circle"),
        ToolKind::Marker => ("Marker", "translucent highlight"),
        ToolKind::Text => ("Text", "text annotation"),
        ToolKind::Counter => ("Counter", "numbered steps"),
        ToolKind::Pixelate => ("Pixelate", "secure pixelate"),
        ToolKind::Blur => ("Blur", "gaussian blur"),
        ToolKind::Invert => ("Invert", "invert colors"),
        ToolKind::Move => ("Move", "move the selection"),
        ToolKind::Eyedropper => ("Eyedropper", "pick a color"),
    }
}

/// The action buttons' text half plus their key chord. The fixed chords
/// mirror the selection engine's bindings (`selection/events.rs`: Ctrl+C
/// copies, Esc walks the cascade to close); undo/redo read the rebindable
/// slots. Save/pin/upload/open-app ship keyless (toolbar-only actions).
fn action_blurb(id: &str, shortcuts: &ToolShortcuts) -> (String, Option<String>) {
    match id {
        "copy" => ("Copy to clipboard".to_owned(), Some("Ctrl+C".to_owned())),
        "save" => ("Save to disk".to_owned(), None),
        "pin" => ("Pin to screen".to_owned(), None),
        "upload" => ("Upload".to_owned(), None),
        "undo" => (
            "Undo".to_owned(),
            Some(format!("Ctrl+{}", chord(shortcuts.undo_key()))),
        ),
        "redo" => (
            "Redo".to_owned(),
            Some(format!("Ctrl+Shift+{}", chord(shortcuts.redo_key()))),
        ),
        "open-app" => ("Open with another app".to_owned(), None),
        "exit" => ("Close the editor".to_owned(), Some("Esc".to_owned())),
        other => (other.to_owned(), None),
    }
}

/// Formats one key code as its display chord fragment (`KeyP` -> `P`,
/// `Digit3` -> `3`, named keys by their short label).
#[must_use]
pub fn chord(code: KeyCode) -> String {
    let named = match code {
        KeyCode::Enter | KeyCode::NumpadEnter => Some("Enter"),
        KeyCode::Escape => Some("Esc"),
        KeyCode::Space => Some("Space"),
        KeyCode::Tab => Some("Tab"),
        KeyCode::ArrowUp => Some("Up"),
        KeyCode::ArrowDown => Some("Down"),
        KeyCode::ArrowLeft => Some("Left"),
        KeyCode::ArrowRight => Some("Right"),
        KeyCode::Backspace => Some("Backspace"),
        KeyCode::Delete => Some("Del"),
        KeyCode::Insert => Some("Ins"),
        KeyCode::Home => Some("Home"),
        KeyCode::End => Some("End"),
        KeyCode::PageUp => Some("PgUp"),
        KeyCode::PageDown => Some("PgDn"),
        KeyCode::Minus => Some("-"),
        KeyCode::Equal => Some("="),
        KeyCode::Comma => Some(","),
        KeyCode::Period => Some("."),
        KeyCode::Slash => Some("/"),
        KeyCode::Backslash => Some("\\"),
        KeyCode::Semicolon => Some(";"),
        KeyCode::Quote => Some("'"),
        KeyCode::Backquote => Some("`"),
        KeyCode::BracketLeft => Some("["),
        KeyCode::BracketRight => Some("]"),
        _ => None,
    };
    if let Some(named) = named {
        return named.to_owned();
    }
    // The physical-key Debug names are structured (`KeyP`, `Digit3`,
    // `Numpad7`, `F12`): strip the row prefix into the glyph a keycap shows.
    let debug = format!("{code:?}");
    if let Some(letter) = debug.strip_prefix("Key") {
        return letter.to_owned();
    }
    if let Some(digit) = debug.strip_prefix("Digit") {
        return digit.to_owned();
    }
    if let Some(digit) = debug.strip_prefix("Numpad") {
        return format!("Num{digit}");
    }
    debug
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn tool_tooltip_carries_the_active_binding() {
        let shortcuts = ToolShortcuts::default();
        let pencil = ToolbarButton::Tool(ToolKind::Pencil);
        assert_eq!(
            button_tooltip(&pencil, &shortcuts),
            "Pencil — freehand draw [P]"
        );
        // Counter ships unbound: description alone, no empty brackets.
        let counter = ToolbarButton::Tool(ToolKind::Counter);
        assert_eq!(
            button_tooltip(&counter, &shortcuts),
            "Counter — numbered steps"
        );
    }

    #[test]
    fn rebind_moves_the_tooltip_key() {
        let mut shortcuts = ToolShortcuts::default();
        shortcuts.rebind(ToolKind::Pencil, Some(KeyCode::KeyX));
        let pencil = ToolbarButton::Tool(ToolKind::Pencil);
        assert_eq!(
            button_tooltip(&pencil, &shortcuts),
            "Pencil — freehand draw [X]"
        );
        shortcuts.rebind(ToolKind::Pencil, None);
        assert_eq!(
            button_tooltip(&pencil, &shortcuts),
            "Pencil — freehand draw"
        );
    }

    #[test]
    fn undo_redo_tooltips_follow_their_slots() {
        let mut shortcuts = ToolShortcuts::default();
        let undo = ToolbarButton::Action("undo".to_owned());
        let redo = ToolbarButton::Action("redo".to_owned());
        assert_eq!(button_tooltip(&undo, &shortcuts), "Undo [Ctrl+Z]");
        assert_eq!(button_tooltip(&redo, &shortcuts), "Redo [Ctrl+Shift+Z]");
        shortcuts.rebind_undo_redo(KeyCode::KeyU, KeyCode::KeyY);
        assert_eq!(button_tooltip(&undo, &shortcuts), "Undo [Ctrl+U]");
        assert_eq!(button_tooltip(&redo, &shortcuts), "Redo [Ctrl+Shift+Y]");
    }

    #[test]
    fn action_tooltips_match_the_engine_bindings() {
        let shortcuts = ToolShortcuts::default();
        let cases = [
            ("copy", "Copy to clipboard [Ctrl+C]"),
            ("save", "Save to disk"),
            ("pin", "Pin to screen"),
            ("upload", "Upload"),
            ("open-app", "Open with another app"),
            ("exit", "Close the editor [Esc]"),
            ("mystery", "mystery"),
        ];
        for (id, expected) in cases {
            let button = ToolbarButton::Action(id.to_owned());
            assert_eq!(button_tooltip(&button, &shortcuts), expected, "{id}");
        }
    }

    #[test]
    fn chord_formats_every_key_shape() {
        assert_eq!(chord(KeyCode::KeyP), "P");
        assert_eq!(chord(KeyCode::Digit3), "3");
        assert_eq!(chord(KeyCode::Numpad7), "Num7");
        assert_eq!(chord(KeyCode::Enter), "Enter");
        assert_eq!(chord(KeyCode::Escape), "Esc");
        assert_eq!(chord(KeyCode::Space), "Space");
        assert_eq!(chord(KeyCode::ArrowLeft), "Left");
        assert_eq!(chord(KeyCode::F12), "F12");
        assert_eq!(chord(KeyCode::Minus), "-");
    }
}

//! The editor's public vocabulary: per-event environment, shell effects,
//! and the update record. Types only - the state machine lives in
//! [`super::EditorState`], the event semantics in [`super::events`].

use std::time::Instant;

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use winit::keyboard::ModifiersState;

use crate::input::Action;

/// The per-event environment [`OverlayCore`](crate::OverlayCore) supplies
/// (the editor twin of `SelectionEnv`).
#[derive(Debug, Clone, Copy)]
pub struct EditorEnv {
    /// The current selection, global logical.
    pub selection: Option<LogicalRect>,
    /// The keyboard modifier snapshot.
    pub modifiers: ModifiersState,
    /// The event timestamp (digit-accumulator clock - injectable).
    pub now: Instant,
    /// Whether the color picker is visible (Esc-cascade stage 5, todo 26).
    pub picker_visible: bool,
    /// The last tracked cursor position (wheel hover, edit commits).
    pub mouse: Option<LogicalPoint>,
}

/// A shell-facing editor effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorEffect {
    /// Right-click: open the color wheel at the cursor (todo 26 seam - the
    /// F27 P2 priority, emitted by the editor since todo 20).
    ColorWheel,
}

impl EditorEffect {
    /// The stable tracing token (QA asserts on these, never prose).
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ColorWheel => "color-wheel",
        }
    }
}

impl From<EditorEffect> for Action {
    fn from(effect: EditorEffect) -> Self {
        match effect {
            EditorEffect::ColorWheel => Self::ColorWheel,
        }
    }
}

/// The outcome of one editor event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditorUpdate {
    /// Whether the editor consumed the event (the selection engine and the
    /// rest of the funnel skip it).
    pub consumed: bool,
    /// Effects for the shell.
    pub effects: Vec<EditorEffect>,
    /// Whether editor visuals changed (every window redraws - scene objects
    /// and outlines span monitors like the selection does).
    pub changed: bool,
    /// Selection rect to restore (undo/redo of move-selection).
    pub restore_selection: Option<LogicalRect>,
}

impl EditorUpdate {
    /// Not consumed, nothing changed.
    pub(super) fn pass() -> Self {
        Self::default()
    }

    /// Not consumed, but editor visuals changed (redraw every window).
    pub(super) fn passing(changed: bool) -> Self {
        Self {
            consumed: false,
            effects: Vec::new(),
            changed,
            restore_selection: None,
        }
    }

    /// Consumed.
    pub(super) fn eaten(changed: bool) -> Self {
        Self {
            consumed: true,
            effects: Vec::new(),
            changed,
            restore_selection: None,
        }
    }

    /// Consumed with shell effects.
    pub(super) fn with(effects: Vec<EditorEffect>, changed: bool) -> Self {
        for effect in &effects {
            tracing::info!(
                target: "flowshot_ui::editor",
                effect = effect.token(),
                "editor effect"
            );
        }
        Self {
            consumed: true,
            effects,
            changed,
            restore_selection: None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn effect_maps_to_the_shell_action_and_token() {
        assert_eq!(Action::from(EditorEffect::ColorWheel), Action::ColorWheel);
        assert_eq!(EditorEffect::ColorWheel.token(), "color-wheel");
    }

    #[test]
    fn update_constructors_set_the_consumed_contract() {
        assert_eq!(EditorUpdate::pass(), EditorUpdate::default());
        assert!(!EditorUpdate::passing(true).consumed);
        assert!(EditorUpdate::passing(true).changed);
        assert!(EditorUpdate::eaten(false).consumed);
        assert!(EditorUpdate::with(vec![EditorEffect::ColorWheel], false).consumed);
    }
}

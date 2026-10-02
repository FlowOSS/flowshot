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
    /// Whether the color picker is visible (Esc-cascade stage 5).
    pub picker_visible: bool,
    /// The last tracked cursor position (wheel hover, edit commits).
    pub mouse: Option<LogicalPoint>,
}

/// A shell-facing editor effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorEffect {
    /// Right-click: open the color wheel at the cursor (a chrome seam - the
    /// F27 P2 priority, emitted by the editor).
    ColorWheel,
    /// The eyedropper sampled a color (the funnel delivers it
    /// to the standalone color-pick sink - a binary-layer seam).
    ColorPicked(flowshot_core::scene::Color),
}

impl EditorEffect {
    /// The stable tracing token (QA asserts on these, never prose).
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ColorWheel => "color-wheel",
            Self::ColorPicked(_) => "color-picked",
        }
    }
}

impl From<EditorEffect> for Action {
    fn from(effect: EditorEffect) -> Self {
        match effect {
            EditorEffect::ColorWheel => Self::ColorWheel,
            EditorEffect::ColorPicked(_) => Self::ColorPicked,
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
    /// Whether the dispatched tool size was written (digits/wheel - the
    /// funnel flashes the chrome size notifier, the Flameshot
    /// `setToolSize` -> `NotifierBox::showMessage` parity).
    pub resized: bool,
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
            resized: false,
        }
    }

    /// Consumed.
    pub(super) fn eaten(changed: bool) -> Self {
        Self {
            consumed: true,
            effects: Vec::new(),
            changed,
            restore_selection: None,
            resized: false,
        }
    }

    /// Consumed, visuals changed, and the dispatched tool size was written
    /// (the digits/wheel adjusters - the funnel flashes the size notifier).
    pub(super) fn eaten_resized() -> Self {
        Self {
            consumed: true,
            effects: Vec::new(),
            changed: true,
            restore_selection: None,
            resized: true,
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
            resized: false,
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

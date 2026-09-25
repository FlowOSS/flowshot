//! The selection engine's public vocabulary: config keys, per-event
//! environment, shell effects, and the update record. Types only - the
//! state machine lives in [`super::SelectionState`].

use std::time::Instant;

use flowshot_core::config::EditorConfig;
use flowshot_core::geometry::LogicalRect;
use winit::keyboard::ModifiersState;

use crate::input::Action;

/// The config keys the selection engine consumes (`[editor]` group). No
/// other behavior constant is tunable (plan todo 16 "Must NOT").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionConfig {
    /// HUD position 0-5 ([`HudPosition::from_config`](super::HudPosition::from_config)).
    pub hud_position: u8,
    /// Milliseconds of inactivity before the HUD auto-hides (0 = never).
    pub hud_hide_time: u32,
    /// Whether a double-click inside the selection copies it.
    pub double_click_copies: bool,
}

impl Default for SelectionConfig {
    fn default() -> Self {
        Self::from_editor(&EditorConfig::default())
    }
}

impl SelectionConfig {
    /// The selection-relevant subset of the `[editor]` config group.
    #[must_use]
    pub const fn from_editor(editor: &EditorConfig) -> Self {
        Self {
            hud_position: editor.hud_position,
            hud_hide_time: editor.hud_hide_time,
            double_click_copies: editor.double_click_copies,
        }
    }
}

/// The per-event context [`OverlayCore`](crate::OverlayCore) supplies: the
/// layout bounds (the clamp target), the current modifier snapshot, and the
/// event time (the HUD/double-click clock - injectable for tests).
#[derive(Debug, Clone, Copy)]
pub struct SelectionEnv {
    /// The layout's union bounds; `None` for an empty layout.
    pub bounds: Option<LogicalRect>,
    /// The current keyboard modifiers.
    pub modifiers: ModifiersState,
    /// The event timestamp.
    pub now: Instant,
}

/// A shell-facing effect of a selection interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    /// Enter on a selection: run the accept/export path (todo 35 wires it).
    Accept,
    /// Ctrl+C or a configured double-click: copy the selection (todo 28/35).
    Copy,
    /// Ctrl+Q, or the Esc cascade reaching its end: close the overlay.
    Exit,
    /// Right-click: open the color wheel at the cursor (todo 26 seam; the
    /// position is the shared cursor track).
    ColorWheel,
}

impl Effect {
    /// The stable tracing token (QA asserts on these, never prose).
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Copy => "copy",
            Self::Exit => "exit",
            Self::ColorWheel => "color-wheel",
        }
    }
}

impl From<Effect> for Action {
    fn from(effect: Effect) -> Self {
        match effect {
            Effect::Accept => Self::Accept,
            Effect::Copy => Self::Copy,
            Effect::Exit => Self::Exit,
            Effect::ColorWheel => Self::ColorWheel,
        }
    }
}

/// The outcome of one selection-engine event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SelectionUpdate {
    /// Effects for the shell (mapped into the route report's actions).
    pub effects: Vec<Effect>,
    /// Whether the selection geometry or HUD visibility changed (the shell
    /// redraws EVERY window - a spanning selection and its HUD can live on
    /// any monitor).
    pub changed: bool,
}

impl SelectionUpdate {
    pub(super) fn unchanged() -> Self {
        Self::default()
    }

    pub(super) fn with(effects: Vec<Effect>, changed: bool) -> Self {
        for effect in &effects {
            tracing::info!(
                target: "flowshot_ui::selection",
                effect = effect.token(),
                "selection effect"
            );
        }
        Self { effects, changed }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn config_defaults_match_the_editor_group() {
        let editor = EditorConfig::default();
        let config = SelectionConfig::default();
        assert_eq!(config.hud_position, editor.hud_position);
        assert_eq!(config.hud_hide_time, editor.hud_hide_time);
        assert_eq!(config.double_click_copies, editor.double_click_copies);
        assert_eq!(config.hud_position, 4);
        assert_eq!(config.hud_hide_time, 3000);
        assert!(!config.double_click_copies);
    }

    #[test]
    fn from_editor_projects_the_three_keys() {
        let editor = EditorConfig {
            hud_position: 2,
            hud_hide_time: 500,
            double_click_copies: true,
            ..EditorConfig::default()
        };
        assert_eq!(
            SelectionConfig::from_editor(&editor),
            SelectionConfig {
                hud_position: 2,
                hud_hide_time: 500,
                double_click_copies: true,
            }
        );
    }
}

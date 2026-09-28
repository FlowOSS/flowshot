//! The tool taxonomy (draft F27/F12).
//!
//! [`ToolKind`] is the stable type enum every annotation tool
//! registers under. The string ids are the config-compat surface (F27:
//! "type-enum stability = config compat"): they match the `[ui]`
//! `toolbar_buttons` entries and the scene's `ToolObject::type_id` family.
//! The default activation keys are the Flameshot F12 shortcut map
//! (`confighandler.cpp` `recognizedShortcuts`): P/D/A/S/R/C/M/T/B/I -
//! counter and move ship unbound (Flameshot binds move to Ctrl+M; the
//! shipped key list omits both), rebindable via
//! [`super::keys::ToolShortcuts`].

use winit::keyboard::KeyCode;

/// Which annotation tool a [`Tool`](super::Tool) implementation provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    /// Freehand polyline.
    Pencil,
    /// Straight two-point line (Flameshot `TYPE_DRAWER`).
    Line,
    /// Arrow with head geometry.
    Arrow,
    /// Region selection tool (delegates to the engine).
    Selection,
    /// Rectangle outline with corner radius.
    Rectangle,
    /// Ellipse/circle.
    Circle,
    /// Translucent wide marker.
    Marker,
    /// Text with IME editing.
    Text,
    /// Numbered step bubbles (the id matches the scene's `counter`).
    Counter,
    /// Secure pixelate / blur.
    Pixelate,
    /// The gaussian blur variant of the pixelate tool (unbound by
    /// default like counter/move - the side panel exposes the mode).
    Blur,
    /// Region color inversion.
    Invert,
    /// Selection/object mover (mutation-funnel semantics; falls through to
    /// the selection engine per the Flameshot `startDrawObjectTool`
    /// exclusion).
    Move,
    /// Eyedropper / color picker (F12 `TYPE_GRAB_COLOR`).
    Eyedropper,
}

impl ToolKind {
    /// Every kind, in the Flameshot toolbar order (`buttonTypeOrder`); the
    /// blur variant trails its pixelate parent (no Flameshot toolbar slot -
    /// it is a pixelate mode); eyedropper trails move.
    pub const ALL: [Self; 14] = [
        Self::Pencil,
        Self::Line,
        Self::Arrow,
        Self::Selection,
        Self::Rectangle,
        Self::Circle,
        Self::Marker,
        Self::Text,
        Self::Counter,
        Self::Pixelate,
        Self::Blur,
        Self::Invert,
        Self::Move,
        Self::Eyedropper,
    ];

    /// The stable config/serde id (F27 type-enum stability).
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Pencil => "pencil",
            Self::Line => "line",
            Self::Arrow => "arrow",
            Self::Selection => "selection",
            Self::Rectangle => "rectangle",
            Self::Circle => "circle",
            Self::Marker => "marker",
            Self::Text => "text",
            Self::Counter => "counter",
            Self::Pixelate => "pixelate",
            Self::Blur => "blur",
            Self::Invert => "invert",
            Self::Move => "move",
            Self::Eyedropper => "eyedropper",
        }
    }

    /// Parses a stable id back into a kind (`None` for unknown strings -
    /// the config validate-on-read fallback).
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// The default F12 activation key; `None` ships unbound (counter, move).
    #[must_use]
    pub const fn default_key(self) -> Option<KeyCode> {
        match self {
            Self::Pencil => Some(KeyCode::KeyP),
            Self::Line => Some(KeyCode::KeyD),
            Self::Arrow => Some(KeyCode::KeyA),
            Self::Selection => Some(KeyCode::KeyS),
            Self::Rectangle => Some(KeyCode::KeyR),
            Self::Circle => Some(KeyCode::KeyC),
            Self::Marker => Some(KeyCode::KeyM),
            Self::Text => Some(KeyCode::KeyT),
            Self::Pixelate => Some(KeyCode::KeyB),
            Self::Invert => Some(KeyCode::KeyI),
            Self::Eyedropper => Some(KeyCode::KeyG),
            Self::Blur | Self::Counter | Self::Move => None,
        }
    }

    /// Whether this kind dispatches to the shared `draw_thickness` size
    /// slot (the others own independent `[tools.*]` slots - F27 per-tool
    /// size dispatch, see [`super::size::ToolSizes`]; blur shares the
    /// pixelate slot - Flameshot's blur is the pixelate tool's size-driven
    /// variant).
    #[must_use]
    pub const fn uses_shared_thickness(self) -> bool {
        !matches!(
            self,
            Self::Text
                | Self::Rectangle
                | Self::Marker
                | Self::Pixelate
                | Self::Blur
                | Self::Counter
                | Self::Eyedropper
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn ids_roundtrip_and_match_the_toolbar_config_vocabulary() {
        for kind in ToolKind::ALL {
            assert_eq!(ToolKind::from_id(kind.id()), Some(kind));
        }
        assert_eq!(ToolKind::from_id("nope"), None);
        // The config default toolbar ids parse as tool kinds.
        for id in [
            "arrow",
            "rectangle",
            "circle",
            "marker",
            "text",
            "pixelate",
            "counter",
        ] {
            assert!(ToolKind::from_id(id).is_some(), "{id} must be a tool id");
        }
    }

    #[test]
    fn default_keys_are_the_exact_f12_map() {
        let expected = [
            (ToolKind::Pencil, KeyCode::KeyP),
            (ToolKind::Line, KeyCode::KeyD),
            (ToolKind::Arrow, KeyCode::KeyA),
            (ToolKind::Selection, KeyCode::KeyS),
            (ToolKind::Rectangle, KeyCode::KeyR),
            (ToolKind::Circle, KeyCode::KeyC),
            (ToolKind::Marker, KeyCode::KeyM),
            (ToolKind::Text, KeyCode::KeyT),
            (ToolKind::Pixelate, KeyCode::KeyB),
            (ToolKind::Invert, KeyCode::KeyI),
            (ToolKind::Eyedropper, KeyCode::KeyG),
        ];
        for (kind, key) in expected {
            assert_eq!(kind.default_key(), Some(key));
        }
        assert_eq!(ToolKind::Counter.default_key(), None);
        assert_eq!(ToolKind::Move.default_key(), None);
        assert_eq!(ToolKind::Blur.default_key(), None, "blur ships unbound");
        // No two kinds share a key (the collision rule).
        let keys: Vec<_> = ToolKind::ALL
            .iter()
            .filter_map(|k| k.default_key())
            .collect();
        let mut unique = keys.clone();
        unique.sort_by_key(|code| format!("{code:?}"));
        unique.dedup();
        assert_eq!(keys.len(), unique.len());
    }

    #[test]
    fn shared_thickness_covers_exactly_the_non_independent_tools() {
        for kind in ToolKind::ALL {
            assert_eq!(
                kind.uses_shared_thickness(),
                !matches!(
                    kind,
                    ToolKind::Text
                        | ToolKind::Rectangle
                        | ToolKind::Marker
                        | ToolKind::Pixelate
                        | ToolKind::Blur
                        | ToolKind::Counter
                        | ToolKind::Eyedropper
                )
            );
        }
    }
}

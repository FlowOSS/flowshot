//! The tool taxonomy (clean-room from the Flameshot tool specs).
//!
//! [`ToolKind`] is the stable type enum every annotation tool
//! registers under. The string ids are the config-compat surface (Flameshot:
//! "type-enum stability = config compat"): they match the `[ui]`
//! `toolbar_buttons` entries and the scene's `ToolObject::type_id` family.
//! The default activation keys are the Flameshot shortcut map
//! (`confighandler.cpp` `recognizedShortcuts`): P/D/A/S/R/C/M/T/B/I -
//! counter and move ship unbound (Flameshot binds move to Ctrl+M; the
//! shipped key list omits both), rebindable via
//! [`super::keys::ToolShortcuts`].
//!
//! [`ToolKind::RectangleFill`] is a `FlowShot` EXTENSION: Flameshot has no
//! filled-rectangle `TYPE_*` (its `rectangletool.cpp` always fills), so it
//! carries no Flameshot slot and ships unbound - its mnemonic `F` is already
//! the grid aid toggle, so stealing it would break a shipped binding.

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
    /// Solid filled rectangle (a `FlowShot` extension; shares the scene's
    /// `rectangle` type with [`ToolKind::Rectangle`] and differs only by its
    /// `filled` flag and its `rectangle-filled` `ToolObject::type_id`).
    RectangleFill,
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
    /// Eyedropper / color picker (Flameshot `TYPE_GRAB_COLOR`).
    Eyedropper,
}

impl ToolKind {
    /// Every kind, in the Flameshot toolbar order (`buttonTypeOrder`); the
    /// filled rectangle sits beside its outline parent (no Flameshot slot -
    /// it is a `FlowShot` extension); the blur variant trails its pixelate
    /// parent (no Flameshot toolbar slot - it is a pixelate mode); eyedropper
    /// trails move.
    pub const ALL: [Self; 15] = [
        Self::Pencil,
        Self::Line,
        Self::Arrow,
        Self::Selection,
        Self::Rectangle,
        Self::RectangleFill,
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

    /// The stable config/serde id (Flameshot type-enum stability).
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Pencil => "pencil",
            Self::Line => "line",
            Self::Arrow => "arrow",
            Self::Selection => "selection",
            Self::Rectangle => "rectangle",
            Self::RectangleFill => "rectangle-filled",
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

    /// The default Flameshot activation key; `None` ships unbound (counter,
    /// move, blur, and the filled rectangle - whose mnemonic `F` is already
    /// the grid aid toggle, so it is rebindable-only).
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
            Self::Blur | Self::Counter | Self::Move | Self::RectangleFill => None,
        }
    }

    /// Whether this kind dispatches to the shared `draw_thickness` size
    /// slot (the others own independent `[tools.*]` slots - Flameshot per-tool
    /// size dispatch, see [`super::size::ToolSizes`]; blur shares the
    /// pixelate slot - Flameshot's blur is the pixelate tool's size-driven
    /// variant). The filled rectangle owns NO slot: its paint path fills the
    /// bounds, so both the stroke width and the corner radius are inert.
    #[must_use]
    pub const fn uses_shared_thickness(self) -> bool {
        !matches!(
            self,
            Self::Text
                | Self::Rectangle
                | Self::RectangleFill
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
            "pencil",
            "arrow",
            "rectangle",
            "rectangle-filled",
            "circle",
            "marker",
            "text",
            "pixelate",
            "counter",
        ] {
            assert!(ToolKind::from_id(id).is_some(), "{id} must be a tool id");
        }
        // The scene token the filled rectangle reports resolves back to its
        // own kind, so the layer-list row icon distinguishes fill from outline.
        assert_eq!(
            ToolKind::from_id(flowshot_core::scene::RectObject::FILLED_TYPE_ID),
            Some(ToolKind::RectangleFill)
        );
        assert_eq!(
            ToolKind::from_id(flowshot_core::scene::RectObject::TYPE_ID),
            Some(ToolKind::Rectangle)
        );
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
        // The FlowShot extension ships unbound: its mnemonic F is the grid
        // aid toggle, and stealing a shipped aid key is a user-visible
        // regression. It stays rebindable from the settings shortcut tab.
        assert_eq!(
            ToolKind::RectangleFill.default_key(),
            None,
            "the filled rectangle ships unbound (F is the grid aid)"
        );
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
                        | ToolKind::RectangleFill
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

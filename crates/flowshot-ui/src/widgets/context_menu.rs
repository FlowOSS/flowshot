//! Context menu widget.

use crate::render::{Color, DisplayList, Rect, ShadowSpec, Shape};
use flowshot_core::tokens::DesignTokens;

/// A context menu widget.
#[derive(Debug, Clone)]
pub struct ContextMenu {
    /// The bounding rectangle.
    pub rect: Rect,
}

impl ContextMenu {
    /// Creates a new context menu.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self { rect }
    }

    /// Draws the context menu into the display list.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let radius = tokens.radii.medium as f32 * scale;

        if let Some(spec) = ShadowSpec::from_token(&tokens.shadows.medium, scale) {
            list.shadow(self.rect, radius, spec);
        }

        list.fill(
            Shape::Rect {
                rect: self.rect,
                radius,
            },
            contrast,
        );
    }
}

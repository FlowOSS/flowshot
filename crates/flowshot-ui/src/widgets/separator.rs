//! Separator widget.

use crate::render::{Color, DisplayList, Rect, Shape};
use flowshot_core::tokens::DesignTokens;

/// A separator widget.
#[derive(Debug, Clone)]
pub struct Separator {
    /// The bounding rectangle.
    pub rect: Rect,
}

impl Separator {
    /// Creates a new separator.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self { rect }
    }

    /// Draws the separator into the display list.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let thickness = 1.0 * scale;
        let is_vertical = self.rect.size.height > self.rect.size.width;

        let line_rect = if is_vertical {
            Rect::from_parts(
                self.rect.origin.x + (self.rect.size.width - thickness) / 2.0,
                self.rect.origin.y,
                thickness,
                self.rect.size.height,
            )
        } else {
            Rect::from_parts(
                self.rect.origin.x,
                self.rect.origin.y + (self.rect.size.height - thickness) / 2.0,
                self.rect.size.width,
                thickness,
            )
        };

        list.fill(
            Shape::Rect {
                rect: line_rect,
                radius: 0.0,
            },
            contrast.with_alpha8(40),
        );
    }
}

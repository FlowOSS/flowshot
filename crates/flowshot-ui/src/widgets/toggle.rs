//! Toggle switch widget.

use crate::render::{Color, DisplayList, Rect, Shape};
use flowshot_core::tokens::DesignTokens;

/// A toggle switch widget.
#[derive(Debug, Clone)]
pub struct Toggle {
    /// The bounding rectangle.
    pub rect: Rect,
    /// Whether the toggle is checked.
    pub checked: bool,
}

impl Toggle {
    /// Creates a new toggle.
    #[must_use]
    pub fn new(rect: Rect, checked: bool) -> Self {
        Self { rect, checked }
    }

    /// Draws the toggle into the display list.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let track_width = 36.0 * scale;
        let track_height = 20.0 * scale;
        let track_rect = Rect::from_parts(
            self.rect.origin.x,
            self.rect.origin.y + (self.rect.size.height - track_height) / 2.0,
            track_width,
            track_height,
        );

        let bg_color = if self.checked {
            accent
        } else {
            contrast.with_alpha8(40)
        };

        // Track
        list.fill(
            Shape::Rect {
                rect: track_rect,
                radius: track_height / 2.0,
            },
            bg_color,
        );

        // Thumb
        let thumb_radius = 8.0 * scale;
        let thumb_x = if self.checked {
            track_rect.origin.x + track_width - thumb_radius * 2.0 - 2.0 * scale
        } else {
            track_rect.origin.x + 2.0 * scale
        };

        let thumb_rect = Rect::from_parts(
            thumb_x,
            track_rect.origin.y + (track_height - thumb_radius * 2.0) / 2.0,
            thumb_radius * 2.0,
            thumb_radius * 2.0,
        );

        list.fill(
            Shape::Rect {
                rect: thumb_rect,
                radius: thumb_radius,
            },
            Color::from_rgba8(255, 255, 255, 255),
        );
    }
}

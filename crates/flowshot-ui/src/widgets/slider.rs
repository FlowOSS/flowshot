//! Slider widget.

use crate::render::{Color, DisplayList, Rect, Shape};
use flowshot_core::tokens::DesignTokens;

/// A slider widget.
#[derive(Debug, Clone)]
pub struct Slider {
    /// The bounding rectangle.
    pub rect: Rect,
    /// The current value (0.0 to 1.0).
    pub value: f32,
}

impl Slider {
    /// Creates a new slider.
    #[must_use]
    pub fn new(rect: Rect, value: f32) -> Self {
        Self { rect, value }
    }

    /// Draws the slider into the display list.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let track_height = 4.0 * scale;
        let track_rect = Rect::from_parts(
            self.rect.origin.x,
            self.rect.origin.y + (self.rect.size.height - track_height) / 2.0,
            self.rect.size.width,
            track_height,
        );

        // Track background
        list.fill(
            Shape::Rect {
                rect: track_rect,
                radius: track_height / 2.0,
            },
            contrast.with_alpha8(40),
        );

        // Track fill
        let fill_width = self.rect.size.width * self.value.clamp(0.0, 1.0);
        if fill_width > 0.0 {
            let fill_rect = Rect::from_parts(
                track_rect.origin.x,
                track_rect.origin.y,
                fill_width,
                track_height,
            );
            list.fill(
                Shape::Rect {
                    rect: fill_rect,
                    radius: track_height / 2.0,
                },
                accent,
            );
        }

        // Thumb
        let thumb_radius = 8.0 * scale;
        let thumb_rect = Rect::from_parts(
            self.rect.origin.x + fill_width - thumb_radius,
            self.rect.origin.y + (self.rect.size.height - thumb_radius * 2.0) / 2.0,
            thumb_radius * 2.0,
            thumb_radius * 2.0,
        );

        list.fill(
            Shape::Rect {
                rect: thumb_rect,
                radius: thumb_radius,
            },
            accent,
        );
    }
}

//! Toggle switch widget.

use crate::render::{Color, DisplayList, Rect, Shape};
use flowshot_core::tokens::DesignTokens;

/// The unchecked track wash (0-255): the shared state-wash ramp's press
/// step (one ink at many alphas - the design system's state language).
const TRACK_WASH_ALPHA: u8 = super::PRESS_WASH_ALPHA;

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

    /// Draws the toggle into the display list. The track fills `self.rect`
    /// (the caller owns the footprint - the side panel sizes it from the
    /// spacing tokens); the thumb insets by half the small spacing step.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let track_rect = self.rect;
        let track_height = track_rect.size.height;

        let bg_color = if self.checked {
            accent
        } else {
            contrast.with_alpha8(TRACK_WASH_ALPHA)
        };

        list.fill(
            Shape::Rect {
                rect: track_rect,
                radius: track_height / 2.0,
            },
            bg_color,
        );

        let inset = tokens.spacing.small as f32 * scale / 2.0;
        let thumb_radius = track_height / 2.0 - inset;
        let thumb_x = if self.checked {
            track_rect.origin.x + track_rect.size.width - thumb_radius * 2.0 - inset
        } else {
            track_rect.origin.x + inset
        };

        let thumb_rect = Rect::from_parts(
            thumb_x,
            track_rect.origin.y + (track_height - thumb_radius * 2.0) / 2.0,
            thumb_radius * 2.0,
            thumb_radius * 2.0,
        );

        // The thumb ink derives from the track color (todo-41 audit: white
        // was hardcoded - unreadable on a light accent token).
        let thumb_ink = if self.checked {
            accent.readable_ink()
        } else {
            contrast.readable_ink()
        };
        list.fill(
            Shape::Rect {
                rect: thumb_rect,
                radius: thumb_radius,
            },
            thumb_ink,
        );
    }
}

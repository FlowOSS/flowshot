//! Icon button widget (toolbar pill).

use super::{Icon, button::ButtonState};
use crate::render::{Color, DisplayList, Rect, Shape, TextureId};
use flowshot_core::tokens::DesignTokens;

/// An icon button widget.
#[derive(Debug, Clone)]
pub struct IconButton {
    /// The bounding rectangle.
    pub rect: Rect,
    /// The current state.
    pub state: ButtonState,
    /// The icon.
    pub icon: Icon,
}

impl IconButton {
    /// Creates a new icon button.
    #[must_use]
    pub fn new(rect: Rect, icon: Icon) -> Self {
        Self {
            rect,
            state: ButtonState::Idle,
            icon,
        }
    }

    /// Sets the state.
    #[must_use]
    pub fn state(mut self, state: ButtonState) -> Self {
        self.state = state;
        self
    }

    /// Draws the icon button into the display list.
    pub fn draw(
        &self,
        list: &mut DisplayList,
        tokens: &DesignTokens,
        scale: f32,
        atlas: TextureId,
    ) {
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let radius = self.rect.size.height / 2.0; // Pill shape

        let bg_color = match self.state {
            ButtonState::Idle => contrast.with_alpha8(0),
            ButtonState::Hover => contrast.with_alpha8(20),
            ButtonState::Press => contrast.with_alpha8(40),
            ButtonState::Focus => contrast.with_alpha8(0),
            ButtonState::Disabled => contrast.with_alpha8(0),
        };

        if bg_color.a > 0.0 {
            list.fill(
                Shape::Rect {
                    rect: self.rect,
                    radius,
                },
                bg_color,
            );
        }

        if self.state == ButtonState::Focus {
            list.stroke(
                Shape::Rect {
                    rect: self.rect,
                    radius,
                },
                2.0 * scale,
                accent,
            );
        }

        let icon_rect = self.icon.rect();
        let icon_size = icon_rect[2] * scale;
        let dst = Rect::from_parts(
            self.rect.origin.x + (self.rect.size.width - icon_size) / 2.0,
            self.rect.origin.y + (self.rect.size.height - icon_size) / 2.0,
            icon_size,
            icon_size,
        );
        let src = Rect::from_parts(icon_rect[0], icon_rect[1], icon_rect[2], icon_rect[3]);

        list.image(atlas, dst, Some(src));
    }
}

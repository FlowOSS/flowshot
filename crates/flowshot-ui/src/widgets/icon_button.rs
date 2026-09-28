//! Icon button widget (toolbar pill).
//!
//! Motion state language: the background wash is ONE ink (the contrast
//! token) at the [`super::wash_alpha`] ramp - discrete through
//! [`ButtonState`], or continuous through the animated `wash` level the
//! chrome's hover/press tweens feed. `alpha` fades the whole button (the
//! toolbar's staggered reveal).

use super::{FOCUS_RING_WIDTH, Icon, button::ButtonState, wash_alpha};
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
    /// Animated wash level (0 idle, 1 hover, 2 press); overrides the
    /// state-derived background when set.
    pub wash: Option<f64>,
    /// Uniform fade multiplier in `[0, 1]` (the reveal seam).
    pub alpha: f32,
}

impl IconButton {
    /// Creates a new icon button.
    #[must_use]
    pub fn new(rect: Rect, icon: Icon) -> Self {
        Self {
            rect,
            state: ButtonState::Idle,
            icon,
            wash: None,
            alpha: 1.0,
        }
    }

    /// Sets the state.
    #[must_use]
    pub fn state(mut self, state: ButtonState) -> Self {
        self.state = state;
        self
    }

    /// Sets the animated wash level (hover/press feedback).
    #[must_use]
    pub fn wash(mut self, level: f64) -> Self {
        self.wash = Some(level);
        self
    }

    /// Sets the uniform fade multiplier (the reveal fade).
    #[must_use]
    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha.clamp(0.0, 1.0);
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

        let bg_alpha8 = match self.wash {
            Some(level) => wash_alpha(level),
            None => match self.state {
                ButtonState::Idle | ButtonState::Focus | ButtonState::Disabled => 0,
                ButtonState::Hover => super::HOVER_WASH_ALPHA,
                ButtonState::Press => super::PRESS_WASH_ALPHA,
            },
        };
        let bg_color = contrast.with_alpha(f32::from(bg_alpha8) / 255.0 * self.alpha);

        if bg_color.a > 0.0 {
            list.fill(
                Shape::Rect {
                    rect: self.rect,
                    radius,
                },
                bg_color,
            );
        }

        if self.state == ButtonState::Focus && self.alpha > 0.0 {
            list.stroke(
                Shape::Rect {
                    rect: self.rect,
                    radius,
                },
                FOCUS_RING_WIDTH * scale,
                accent.with_alpha(self.alpha),
            );
        }

        if self.alpha <= 0.0 {
            return;
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

        list.image_faded(atlas, dst, Some(src), self.alpha);
    }
}

//! Button widget.

use super::Icon;
use crate::render::{Color, DisplayList, Point, Rect, Shape, TextCommand, TextureId};
use flowshot_core::tokens::DesignTokens;

/// State of a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    /// Normal state.
    Idle,
    /// Mouse over.
    Hover,
    /// Mouse pressed.
    Press,
    /// Keyboard focused.
    Focus,
    /// Disabled.
    Disabled,
}

/// A button widget.
#[derive(Debug, Clone)]
pub struct Button<'a> {
    /// The bounding rectangle.
    pub rect: Rect,
    /// The current state.
    pub state: ButtonState,
    /// Optional icon.
    pub icon: Option<Icon>,
    /// Optional text label.
    pub label: Option<&'a str>,
    /// Optional tooltip text.
    pub tooltip: Option<&'a str>,
}

impl<'a> Button<'a> {
    /// Creates a new button.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            state: ButtonState::Idle,
            icon: None,
            label: None,
            tooltip: None,
        }
    }

    /// Sets the state.
    #[must_use]
    pub fn state(mut self, state: ButtonState) -> Self {
        self.state = state;
        self
    }

    /// Sets the icon.
    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the label.
    #[must_use]
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Sets the tooltip.
    #[must_use]
    pub fn tooltip(mut self, tooltip: &'a str) -> Self {
        self.tooltip = Some(tooltip);
        self
    }

    /// Draws the button into the display list.
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

        let radius = tokens.radii.medium as f32 * scale;

        let (bg_color, fg_color) = match self.state {
            ButtonState::Idle => (
                contrast.with_alpha8(0),
                contrast.with_alpha8(super::IDLE_INK_ALPHA),
            ),
            ButtonState::Hover => (
                contrast.with_alpha8(super::HOVER_WASH_ALPHA),
                contrast.with_alpha8(255),
            ),
            ButtonState::Press => (
                contrast.with_alpha8(super::PRESS_WASH_ALPHA),
                contrast.with_alpha8(255),
            ),
            ButtonState::Focus => (contrast.with_alpha8(0), accent),
            ButtonState::Disabled => (
                contrast.with_alpha8(0),
                contrast.with_alpha8(super::DISABLED_INK_ALPHA),
            ),
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
                super::FOCUS_RING_WIDTH * scale,
                accent,
            );
        }

        let mut x = self.rect.origin.x + tokens.spacing.medium as f32 * scale;
        let y = self.rect.origin.y + self.rect.size.height / 2.0;

        if let Some(icon) = self.icon {
            let icon_rect = icon.rect();
            let icon_size = icon_rect[2] * scale;
            let dst = Rect::from_parts(x, y - icon_size / 2.0, icon_size, icon_size);
            let src = Rect::from_parts(icon_rect[0], icon_rect[1], icon_rect[2], icon_rect[3]);

            list.image(atlas, dst, Some(src));

            x += icon_size + tokens.spacing.small as f32 * scale;
        }

        if let Some(label) = self.label {
            let font_size = tokens.typography.base_size as f32 * scale;
            let line_height = font_size * 1.2;

            list.text(TextCommand {
                position: Point::new(x, y - line_height / 2.0),
                text: label.to_owned(),
                font_size,
                line_height,
                color: fg_color,
                family: Some(tokens.typography.family.clone()),
                max_width: None,
            });
        }
    }
}

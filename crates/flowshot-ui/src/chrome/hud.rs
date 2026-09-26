//! The size-notifier HUD (plan todo 26).

use crate::editor::EditorState;
use crate::render::{Color, DisplayList, Point, Rect, Shape, TextCommand, TextureId};
use flowshot_core::tokens::DesignTokens;

/// The size-notifier HUD UI.
#[derive(Debug, Default)]
pub struct SizeHud {
    /// The bounding rectangle.
    pub rect: Rect,
    /// Whether the HUD is visible.
    pub visible: bool,
}

impl SizeHud {
    /// Draws the size HUD.
    pub fn draw(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        tokens: &DesignTokens,
        scale: f32,
        _atlas: TextureId,
    ) {
        if !self.visible {
            return;
        }

        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        list.fill(
            Shape::Rect {
                rect: self.rect,
                radius: tokens.radii.medium as f32 * scale,
            },
            contrast.with_alpha8(200),
        );

        let font_size = tokens.typography.base_size as f32 * scale;
        let line_height = font_size * 1.2;

        let text = format!("Size: {}", editor.tool_size());

        list.text(TextCommand {
            position: Point::new(
                self.rect.origin.x + tokens.spacing.medium as f32 * scale,
                self.rect.origin.y + (self.rect.size.height - line_height) / 2.0,
            ),
            text,
            font_size,
            line_height,
            color: Color::from_rgba8(255, 255, 255, 255),
            family: Some(tokens.typography.family.clone()),
            max_width: None,
        });
    }
}

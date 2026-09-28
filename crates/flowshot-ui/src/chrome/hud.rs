//! The size-notifier HUD.

use crate::editor::EditorState;
use crate::render::{Color, DisplayList, Point, Rect, Shape, TextCommand, TextureId};
use flowshot_core::tokens::DesignTokens;

/// The HUD box opacity (0-255): the selection HUD's alpha (F27
/// `capturewidget.cpp` paints the geometry box at 200).
const HUD_BOX_ALPHA: u8 = 200;
/// Text line height ratio (the render stack's standard, shared with the
/// selection metrics' `LINE_SPACING_RATIO`).
const LINE_HEIGHT_RATIO: f32 = 1.2;

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
        let background = contrast.with_alpha8(HUD_BOX_ALPHA);

        list.fill(
            Shape::Rect {
                rect: self.rect,
                radius: tokens.radii.medium as f32 * scale,
            },
            background,
        );

        let font_size = tokens.typography.base_size as f32 * scale;
        let line_height = font_size * LINE_HEIGHT_RATIO;

        let text = format!("Size: {}", editor.tool_size());

        list.text(TextCommand {
            position: Point::new(
                self.rect.origin.x + tokens.spacing.medium as f32 * scale,
                self.rect.origin.y + (self.rect.size.height - line_height) / 2.0,
            ),
            text,
            font_size,
            line_height,
            color: background.readable_ink(),
            family: Some(tokens.typography.family.clone()),
            max_width: None,
        });
    }
}

//! The editor toolbar (plan todo 26).

use crate::editor::paint::local_rect;
use crate::editor::{EditorState, ToolKind};
use crate::render::{Color, DisplayList, Rect, Shape, TextureId};
use crate::widgets::{ButtonState, IconButton, icons::Icon};
use flowshot_core::geometry::{LogicalRect, OutputInfo};
use flowshot_core::tokens::DesignTokens;

/// A button on the toolbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolbarButton {
    /// A drawing tool.
    Tool(ToolKind),
    /// An action (e.g. copy, save, undo).
    Action(String),
}

impl ToolbarButton {
    /// Parses a string identifier into a toolbar button.
    #[must_use]
    pub fn from_id(id: &str) -> Self {
        if let Some(kind) = ToolKind::from_id(id) {
            Self::Tool(kind)
        } else {
            Self::Action(id.to_string())
        }
    }

    /// Returns the icon for this button.
    #[must_use]
    pub fn icon(&self) -> Icon {
        match self {
            Self::Tool(kind) => icon_for_tool(*kind),
            Self::Action(id) => match id.as_str() {
                "copy" => Icon::Copy,
                "save" => Icon::Save,
                "pin" => Icon::Pin,
                "upload" => Icon::Upload,
                "undo" => Icon::Undo2,
                "redo" => Icon::Redo2,
                "open-app" => Icon::ExternalLink,
                "exit" => Icon::X,
                _ => Icon::Square, // Fallback
            },
        }
    }
}

/// The toolbar UI.
#[derive(Debug, Default)]
pub struct Toolbar {
    /// The configured buttons.
    pub buttons: Vec<ToolbarButton>,
}

impl Toolbar {
    /// Computes the layout of the toolbar and its buttons.
    #[must_use]
    pub fn layout(
        &self,
        selection: LogicalRect,
        tokens: &DesignTokens,
        scale: f32,
        output: &OutputInfo,
    ) -> (Rect, Vec<Rect>) {
        let local_sel = local_rect(output, selection);

        let button_size = 32.0 * scale;
        let padding = tokens.spacing.small as f32 * scale;
        let gap = tokens.spacing.small as f32 * scale;

        let width = padding * 2.0
            + self.buttons.len() as f32 * button_size
            + (self.buttons.len().saturating_sub(1)) as f32 * gap;
        let height = padding * 2.0 + button_size;

        let margin = tokens.spacing.medium as f32 * scale;

        let mut x = local_sel.origin.x + local_sel.size.width - width;
        if x < margin {
            x = margin;
        }

        let mut y = local_sel.origin.y + local_sel.size.height + margin;
        if y + height > output.physical_size.height.0 as f32 - margin {
            y = local_sel.origin.y - height - margin;
            if y < margin {
                y = local_sel.origin.y + local_sel.size.height - height - margin;
            }
        }

        let rect = Rect::from_parts(x, y, width, height);

        let mut button_rects = Vec::with_capacity(self.buttons.len());
        let mut bx = x + padding;
        let by = y + padding;

        for _ in &self.buttons {
            button_rects.push(Rect::from_parts(bx, by, button_size, button_size));
            bx += button_size + gap;
        }

        (rect, button_rects)
    }

    /// Draws the toolbar.
    pub fn draw(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        tokens: &DesignTokens,
        scale: f32,
        atlas: TextureId,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) {
        let Some(selection) = selection else {
            return;
        };
        if self.buttons.is_empty() {
            return;
        }

        let (rect, button_rects) = self.layout(selection, tokens, scale, output);

        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        list.fill(
            Shape::Rect {
                rect,
                radius: tokens.radii.medium as f32 * scale,
            },
            contrast,
        );

        for (button, brect) in self.buttons.iter().zip(button_rects) {
            let state = match button {
                ToolbarButton::Tool(kind) if editor.active_tool() == Some(*kind) => {
                    ButtonState::Focus
                }
                _ => ButtonState::Idle,
            };

            let icon_btn = IconButton::new(brect, button.icon()).state(state);
            icon_btn.draw(list, tokens, scale, atlas);
        }
    }
}

/// Maps a tool kind to its icon.
#[must_use]
pub fn icon_for_tool(kind: ToolKind) -> Icon {
    match kind {
        ToolKind::Pencil => Icon::Pencil,
        ToolKind::Line => Icon::Minus,
        ToolKind::Arrow => Icon::ArrowUpRight,
        ToolKind::Selection => Icon::MousePointer2,
        ToolKind::Rectangle => Icon::Square,
        ToolKind::Circle => Icon::Circle,
        ToolKind::Marker => Icon::Highlighter,
        ToolKind::Text => Icon::Type,
        ToolKind::Counter => Icon::ListOrdered,
        ToolKind::Pixelate => Icon::Grid3x3,
        ToolKind::Blur => Icon::Contrast,
        ToolKind::Invert => Icon::Contrast,
        ToolKind::Move => Icon::Move,
    }
}

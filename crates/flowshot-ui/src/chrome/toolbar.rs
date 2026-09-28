//! The editor toolbar (motion pass included).
//!
//! The reveal animation (D8(d)): the plate fades in while each button runs
//! a staggered fade+slide (per-button 120ms, the last landing at 180ms -
//! the plan's band), evaluated from the [`ChromeMotion`] timeline at the
//! frame's `now`. Hover/press feedback is the animated wash level (0 idle,
//! 1 hover, 2 press) the funnel feeds through [`ChromeMotion::set_hover`] /
//! [`ChromeMotion::set_press`].

use std::time::Instant;

use crate::chrome::motion::ChromeMotion;
use crate::editor::paint::local_rect;
use crate::editor::{EditorState, ToolKind};
use crate::render::{Color, DisplayList, Point, Rect, ShadowSpec, Shape, TextureId};
use crate::widgets::{ButtonState, IconButton, icons::Icon};
use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo};
use flowshot_core::tokens::DesignTokens;

use super::color_wheel::BUTTON_BASE_SIZE;

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

        let button_size = BUTTON_BASE_SIZE * scale;
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

    /// The toolbar button under a global logical point (`None` off every
    /// cell): the hover/press wash seam and the press funnel share it with
    /// the paint path's layout, so hit geometry can never disagree with the
    /// drawn cells (the FINAL layout - the reveal animation is visual only).
    #[must_use]
    pub fn button_at(
        &self,
        at: LogicalPoint,
        selection: Option<LogicalRect>,
        tokens: &DesignTokens,
        scale: f32,
        output: &OutputInfo,
    ) -> Option<usize> {
        let selection = selection?;
        let (_, buttons) = self.layout(selection, tokens, scale, output);
        let local = Point::new(
            crate::editor::paint::local_x(output, at.x.0),
            crate::editor::paint::local_y(output, at.y.0),
        );
        buttons.iter().position(|rect| rect.contains(local))
    }

    /// Draws the toolbar with the reveal + wash motion evaluated at `now`.
    pub fn draw(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        tokens: &DesignTokens,
        scale: f32,
        atlas: TextureId,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
        motion: &ChromeMotion,
        now: Instant,
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

        let plate = motion.reveal_background(now);
        if plate > 0.0 {
            if let Some(spec) = ShadowSpec::from_token(&tokens.shadows.medium, scale) {
                list.shadow(rect, tokens.radii.medium as f32 * scale, spec);
            }
            list.fill(
                Shape::Rect {
                    rect,
                    radius: tokens.radii.medium as f32 * scale,
                },
                contrast.with_alpha(plate),
            );
        }

        let count = self.buttons.len();
        let slide = tokens.spacing.medium as f32 * scale;
        for (index, (button, brect)) in self.buttons.iter().zip(button_rects).enumerate() {
            let progress = motion.reveal_progress(now, index, count);
            if progress <= 0.0 {
                continue;
            }
            let state = match button {
                ToolbarButton::Tool(kind) if editor.active_tool() == Some(*kind) => {
                    ButtonState::Focus
                }
                _ => ButtonState::Idle,
            };
            let washed = Rect::from_parts(
                brect.origin.x,
                brect.origin.y + (1.0 - progress) * slide,
                brect.size.width,
                brect.size.height,
            );
            let icon_btn = IconButton::new(washed, button.icon())
                .state(state)
                .wash(motion.wash(now, index))
                .alpha(progress);
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
        ToolKind::Eyedropper => Icon::Rainbow,
    }
}

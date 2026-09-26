//! The text annotation tool (plan todo 22, F27 text spec).
//!
//! Click places an edit session anchored at the press; typing flows through
//! the [`TextSession`] state machine (direct keyboard text AND winit `Ime`
//! events - the always-on model, draft D7); Ctrl+Return or a click outside
//! commits a [`TextObject`] (one undo unit via the editor funnel); Esc
//! cancels through the cascade's tool-widget stage. Point size = the
//! dispatched `[editor].font_size` slot + [`BASE_POINT_SIZE`] (F27
//! `m_font.setPointSize(m_size + BASE_POINT_SIZE)`), color = draw color,
//! family = `[editor].font_family`, and the bounding box carries the
//! Flameshot 5px padding (`texttool.cpp` `process()` `const int val = 5`).
//!
//! BORROW-MODIFIED (drag): Flameshot's drag MOVES the widget and its text
//! area auto-sizes; here a horizontal drag from the anchor defines the WRAP
//! width instead (the plan's "wrap within drag-defined box"), and the
//! committed object bakes the wrapped visual lines (the scene's
//! `draw_text` carries no wrap width - baking keeps editor and export on
//! the single shaping path).
//!
//! Re-edit: with the text tool active, a press on an existing text object
//! hands its data to [`Tool::edit_object_data`] - the session re-opens with
//! the old text preserved and fully selected (Flameshot `setEditMode` +
//! `selectAll` parity), and the editor funnel replaces the object as ONE
//! undo unit on commit.
//!
//! Split at the 250-LOC ceiling: the input handlers live in [`keys`], the
//! edit-overlay paint in [`paint`].

mod keys;
mod paint;

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{
    Color as SceneColor, PaintSink, Point as ScenePoint, TextObject, ToolObject, ToolObjectData,
};
use winit::event::{Ime, MouseButton};

use super::super::kind::ToolKind;
use super::super::size::BASE_POINT_SIZE;
use super::super::tool::{EditKey, EditorContext, Tool, ToolCursor};
use super::text_session::TextSession;
use crate::render::{f32_from_f64, f32_from_u32};

/// The bounding-box padding around the laid-out text (F27 parity:
/// `texttool.cpp` `process()` expands the text area by `val = 5` per side).
pub const TEXT_PADDING: f32 = 5.0;
/// The caret bar width in logical px.
const CARET_WIDTH: f32 = 2.0;
/// The preedit underline thickness in logical px.
const UNDERLINE_THICKNESS: f32 = 2.0;
/// A horizontal drag shorter than this keeps the box unwrapped (a plain
/// click never defines a wrap width).
const MIN_WRAP_WIDTH: f32 = 24.0;
/// The selection highlight alpha (the text color at quarter strength).
const SELECTION_ALPHA: u8 = 64;
/// The `[editor].font_size` default (the dispatched slot before the first
/// `on_size_changed`).
const DEFAULT_FONT_SLOT: u32 = 8;

/// The text annotation tool with real IME editing (todo 22).
#[derive(Debug)]
pub struct TextTool {
    session: Option<TextSession>,
    /// The text layout box's top-left, scene space (global logical).
    anchor: ScenePoint,
    /// The press anchor of an open drag (the wrap-box definition).
    drag_from: Option<LogicalPoint>,
    wrap_width: Option<f32>,
    color: SceneColor,
    tool_size: u32,
    family: String,
}

impl Default for TextTool {
    fn default() -> Self {
        Self {
            session: None,
            anchor: ScenePoint::new(0.0, 0.0),
            drag_from: None,
            wrap_width: None,
            color: SceneColor::new(0, 0, 0, 255),
            tool_size: DEFAULT_FONT_SLOT,
            family: String::new(),
        }
    }
}

impl TextTool {
    /// The rendered point size (F27 `m_size + BASE_POINT_SIZE`).
    fn point_size(&self) -> f32 {
        f32_from_u32(self.tool_size.saturating_add(BASE_POINT_SIZE))
    }
}

impl Tool for TextTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Text
    }

    fn pressed(&mut self, _ctx: &EditorContext<'_>, button: MouseButton, at: LogicalPoint) -> bool {
        // A press inside the edit box (the F27 P4/P2-exception route) moves
        // the caret; anything else falls through to `draw_start`.
        if button != MouseButton::Left {
            return false;
        }
        let Some(session) = &mut self.session else {
            return false;
        };
        let anchor = self.anchor;
        session.click(
            f32_from_f64(at.x.0) - anchor.x,
            f32_from_f64(at.y.0) - anchor.y,
        );
        true
    }

    fn draw_start(&mut self, ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.family.clone_from(&ctx.config.editor.font_family);
        self.anchor = ScenePoint::new(f32_from_f64(at.x.0), f32_from_f64(at.y.0));
        self.wrap_width = None;
        self.drag_from = Some(at);
        self.session = Some(TextSession::new(&self.family, self.point_size(), None));
        tracing::debug!(target: "flowshot_ui::editor", "text edit started");
    }

    fn draw_move(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        let (Some(from), Some(session)) = (self.drag_from, &mut self.session) else {
            return;
        };
        let width = f32_from_f64((at.x.0 - from.x.0).abs());
        if width >= MIN_WRAP_WIDTH {
            self.wrap_width = Some(width);
            session.set_wrap_width(Some(width));
        }
    }

    /// Release never commits text (the F27 lifecycle commits on
    /// Ctrl+Return or a click outside); it only closes the wrap-box drag.
    fn draw_end(
        &mut self,
        _ctx: &EditorContext<'_>,
        _at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        self.drag_from = None;
        None
    }

    fn edit_key(&mut self, ctx: &EditorContext<'_>, key: EditKey<'_>) -> bool {
        self.handle_edit_key(ctx, key)
    }

    fn ime(&mut self, _ctx: &EditorContext<'_>, ime: &Ime) -> bool {
        self.handle_ime(ime)
    }

    fn edit_object_data(&mut self, ctx: &EditorContext<'_>, data: &ToolObjectData) -> bool {
        let ToolObjectData::Text(text) = data else {
            return false;
        };
        self.family.clone_from(&ctx.config.editor.font_family);
        self.anchor = text.position;
        self.wrap_width = None;
        self.drag_from = None;
        self.session = Some(TextSession::with_text(
            &self.family,
            self.point_size(),
            None,
            &text.text,
        ));
        tracing::info!(
            target: "flowshot_ui::editor",
            chars = text.text.chars().count(),
            "text re-edit entered"
        );
        true
    }

    fn paint(&self, _ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        self.paint_session(sink);
    }

    fn bounding_rect(&self) -> Option<LogicalRect> {
        self.edit_rect()
    }

    /// Flameshot `TextTool::isValid() = !m_text.isEmpty()`: the edit session
    /// is committable once it holds text.
    fn is_valid(&self) -> bool {
        self.session
            .as_ref()
            .is_none_or(|session| !session.is_empty())
    }

    fn on_color_changed(&mut self, color: SceneColor) {
        self.color = color;
    }

    fn on_size_changed(&mut self, size: u32) {
        self.tool_size = size;
        let point_size = self.point_size();
        if let Some(session) = &mut self.session {
            session.set_point_size(point_size);
        }
    }

    /// Flameshot `TextTool::showMousePreview` is false - the caret IS the
    /// cursor while editing.
    fn show_mouse_preview(&self) -> bool {
        false
    }

    fn edit_rect(&self) -> Option<LogicalRect> {
        let session = self.session.as_ref()?;
        let (width, height) = session.layout_size();
        Some(LogicalRect::from_raw(
            f64::from(self.anchor.x) - f64::from(TEXT_PADDING),
            f64::from(self.anchor.y) - f64::from(TEXT_PADDING),
            f64::from(width) + f64::from(TEXT_PADDING * 2.0),
            f64::from(height) + f64::from(TEXT_PADDING * 2.0),
        ))
    }

    fn caret_rect(&self) -> Option<LogicalRect> {
        let session = self.session.as_ref()?;
        let caret = session.caret();
        Some(LogicalRect::from_raw(
            f64::from(self.anchor.x + caret.x),
            f64::from(self.anchor.y + caret.y),
            f64::from(CARET_WIDTH),
            f64::from(caret.height),
        ))
    }

    fn commit_edit(&mut self, _ctx: &EditorContext<'_>) -> Option<Box<dyn ToolObject>> {
        let session = self.session.take()?;
        self.drag_from = None;
        let text = session.baked_text();
        if text.is_empty() {
            // Flameshot `isValid() = !m_text.isEmpty()`: an empty commit
            // produces NO object (the plan's failure QA).
            tracing::debug!(target: "flowshot_ui::editor", "empty text commit discarded");
            return None;
        }
        Some(Box::new(TextObject::new(
            self.anchor,
            text,
            self.point_size(),
            self.color,
        )))
    }

    fn cancel_edit(&mut self) {
        if self.session.is_some() {
            tracing::debug!(target: "flowshot_ui::editor", "text edit cancelled");
        }
        self.session = None;
        self.drag_from = None;
        self.wrap_width = None;
    }

    fn cursor(&self) -> ToolCursor {
        if self.session.is_some() {
            ToolCursor::Hidden
        } else {
            ToolCursor::Crosshair
        }
    }
}

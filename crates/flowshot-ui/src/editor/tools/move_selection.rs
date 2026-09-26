//! The move-selection tool (plan todo 27, draft F12 `TYPE_MOVESELECTION`).
//!
//! The move-selection tool drags the entire selection contents-aware: it
//! moves the selection rect AND every annotation whose bounding box lies
//! within the selection. Annotations clipped to the selection move with it.
//!
//! Flameshot parity: `TYPE_MOVESELECTION` is excluded from
//! `startDrawObjectTool` (the routing funnel's `tool_is_move` flag). The
//! move tool falls through to the object/selection path, but when activated
//! explicitly (Ctrl+M), it drags the selection rect + contained objects.
//!
//! Implementation: the tool tracks the drag start position and current
//! position. On `draw_end`, it returns `None` (no scene object committed)
//! but signals the editor to translate the selection rect and all contained
//! objects by the drag delta. The editor's `move_selection` method handles
//! the actual translation.

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{PaintSink, ToolObject};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, Tool};

/// The move-selection tool (F12 `TYPE_MOVESELECTION` parity).
///
/// Drags the selection rect and all contained annotations. Activated by
/// Ctrl+M (the plan's default binding).
#[derive(Debug, Default)]
pub struct MoveSelectionTool {
    last: Option<LogicalPoint>,
    delta_accum: (f64, f64),
}

impl MoveSelectionTool {
    /// The incremental drag delta since the last call, resetting the accumulator.
    pub fn take_delta(&mut self) -> Option<(f64, f64)> {
        if self.last.is_some() {
            let delta = self.delta_accum;
            self.delta_accum = (0.0, 0.0);
            Some(delta)
        } else {
            None
        }
    }
}

impl Tool for MoveSelectionTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Move
    }

    fn draw_start(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.last = Some(at);
        self.delta_accum = (0.0, 0.0);
    }

    fn draw_move(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        if let Some(last) = self.last {
            self.delta_accum.0 += at.x.0 - last.x.0;
            self.delta_accum.1 += at.y.0 - last.y.0;
        }
        self.last = Some(at);
    }

    fn draw_end(
        &mut self,
        _ctx: &EditorContext<'_>,
        _at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        // The move tool does NOT commit a scene object. The editor handles
        // the selection + contained-objects translation via the delta.
        self.last = None;
        self.delta_accum = (0.0, 0.0);
        None
    }

    fn paint(&self, _ctx: &EditorContext<'_>, _sink: &mut dyn PaintSink) {
        // No preview: the selection rect and contained objects move live
        // via the editor's move_selection method.
    }

    fn bounding_rect(&self) -> Option<LogicalRect> {
        None
    }

    fn is_valid(&self) -> bool {
        // Valid only when a drag is in progress.
        self.last.is_some()
    }

    fn show_mouse_preview(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp
    )]

    use super::*;

    #[test]
    fn move_selection_tool_kind_is_move() {
        let tool = MoveSelectionTool::default();
        assert_eq!(tool.kind(), ToolKind::Move);
    }

    #[test]
    fn move_selection_tool_tracks_delta() {
        let mut tool = MoveSelectionTool::default();
        assert!(tool.take_delta().is_none());

        let start = LogicalPoint::from_raw(100.0, 200.0);
        tool.draw_start(&ctx(), start);
        assert_eq!(tool.take_delta(), Some((0.0, 0.0)));

        let current = LogicalPoint::from_raw(150.0, 250.0);
        tool.draw_move(&ctx(), current);
        assert_eq!(tool.take_delta(), Some((50.0, 50.0)));

        // After taking the delta, it should be reset.
        assert_eq!(tool.take_delta(), Some((0.0, 0.0)));

        tool.draw_end(&ctx(), current);
        assert!(tool.take_delta().is_none());
    }

    #[test]
    fn move_selection_tool_is_valid_during_drag() {
        let mut tool = MoveSelectionTool::default();
        assert!(!tool.is_valid());

        let start = LogicalPoint::from_raw(100.0, 200.0);
        tool.draw_start(&ctx(), start);
        assert!(tool.is_valid());

        tool.draw_end(&ctx(), start);
        assert!(!tool.is_valid());
    }

    fn ctx() -> EditorContext<'static> {
        use super::super::super::tool::EditorTools;
        use flowshot_core::config::Config;
        use winit::keyboard::ModifiersState;

        static TOOLS: std::sync::LazyLock<EditorTools> =
            std::sync::LazyLock::new(|| EditorTools::from_config(&Config::default()));

        EditorContext {
            frame: None,
            selection: None,
            color: flowshot_core::scene::Color::new(255, 0, 0, 255),
            tool_size: 3,
            mouse: LogicalPoint::from_raw(0.0, 0.0),
            modifiers: ModifiersState::empty(),
            circle_count: 0,
            config: &TOOLS,
        }
    }
}

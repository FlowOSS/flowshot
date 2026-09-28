//! The selection tool (draft F12 `TYPE_SELECTION`).
//!
//! The selection tool is a PARITY tool: it does NOT draw annotations. Its
//! sole purpose is to re-enter selection mode when the user presses `S`
//! (the default F12 binding). The selection engine owns the
//! selection geometry; this tool is a no-op placeholder that signals
//! "selection mode active" to the routing funnel.
//!
//! Flameshot parity: `TYPE_SELECTION` is a tool kind that delegates all
//! pointer handling to the selection widget. `FlowShot` mirrors this: the
//! selection tool's `draw_start`/`draw_move`/`draw_end` are no-ops, and
//! the routing funnel passes through to the selection engine when the
//! active tool is `Selection`.

use flowshot_core::geometry::{LogicalPoint, LogicalRect};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, Tool};

/// The selection tool (F12 `TYPE_SELECTION` parity).
///
/// A no-op tool that signals "selection mode active" to the routing funnel.
/// The selection engine owns the selection geometry; this tool
/// delegates all pointer handling to it.
#[derive(Debug, Default)]
pub struct SelectionTool;

impl Tool for SelectionTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Selection
    }

    // All draw lifecycle methods are no-ops: the selection engine handles
    // pointer events when this tool is active (the routing funnel's
    // `tool_is_move` exclusion pattern, extended to selection).

    fn draw_start(&mut self, _ctx: &EditorContext<'_>, _at: LogicalPoint) {}

    fn draw_move(&mut self, _ctx: &EditorContext<'_>, _at: LogicalPoint) {}

    fn draw_end(
        &mut self,
        _ctx: &EditorContext<'_>,
        _at: LogicalPoint,
    ) -> Option<Box<dyn flowshot_core::scene::ToolObject>> {
        None
    }

    fn paint(&self, _ctx: &EditorContext<'_>, _sink: &mut dyn flowshot_core::scene::PaintSink) {}

    fn bounding_rect(&self) -> Option<LogicalRect> {
        None
    }

    fn is_valid(&self) -> bool {
        false
    }

    fn show_mouse_preview(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn selection_tool_kind_is_selection() {
        let tool = SelectionTool;
        assert_eq!(tool.kind(), ToolKind::Selection);
    }

    #[test]
    fn selection_tool_is_not_valid_commits_nothing() {
        let tool = SelectionTool;
        assert!(!tool.is_valid());
        assert!(tool.bounding_rect().is_none());
        assert!(!tool.show_mouse_preview());
    }
}

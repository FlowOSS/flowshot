//! The seven shape tools of plan todo 21: pencil, line, arrow, rectangle,
//! ellipse, marker, invert - registered onto a [`ToolRegistry`] by
//! [`register_shape_tools`] (the composition roots: the todo-35 binary and
//! the QA harnesses).
//!
//! Shared mechanics live in [`geometry`]: the F27 two-point stroke with the
//! Ctrl drag conventions (`adjustedVector` clean-room: H/V/45deg for
//! line/arrow/marker, diagonal-only square/circle lock for rect/ellipse),
//! the freehand path stroke with Ramer-Douglas-Peucker simplification
//! (epsilon 0.5px at commit), and the F27 `mousePreviewRect` cursor dot.
//!
//! Every tool: press/drag/release lifecycle with a live preview painting
//! the EXACT committed geometry, commit as one scene object (one undo unit
//! via the framework's `commit_object`), zero-length drags commit nothing,
//! color from `[editor].draw_color` and size from the todo-20 dispatch
//! (shared `draw_thickness`; `[tools.marker].size`; the rectangle's slot IS
//! `[tools.rectangle].corner_radius`).

mod arrow;
mod counter;
mod geometry;
mod path;
mod pixelate;
mod point;
mod shape;
mod text;
mod text_font;
mod text_measure;
mod text_session;

#[cfg(test)]
mod pixelate_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_tests;

pub use arrow::ArrowTool;
pub use counter::CounterTool;
pub use geometry::RDP_EPSILON;
pub use path::{MARKER_ALPHA, MarkerTool, PencilTool};
pub use pixelate::PixelateTool;
pub use point::{InvertTool, LineTool};
pub use shape::{EllipseTool, RectTool};
pub use text::{TEXT_PADDING, TextTool};

use super::kind::ToolKind;
use super::registry::ToolRegistry;

/// Registers all seven todo-21 shape tools on `registry` (idempotent;
/// replaces any prior factory for the same kind).
pub fn register_shape_tools(registry: &mut ToolRegistry) {
    registry.register(ToolKind::Pencil, || Box::new(PencilTool::default()));
    registry.register(ToolKind::Line, || Box::new(LineTool::default()));
    registry.register(ToolKind::Arrow, || Box::new(ArrowTool::default()));
    registry.register(ToolKind::Rectangle, || Box::new(RectTool::default()));
    registry.register(ToolKind::Circle, || Box::new(EllipseTool::default()));
    registry.register(ToolKind::Marker, || Box::new(MarkerTool::default()));
    registry.register(ToolKind::Invert, || Box::new(InvertTool::default()));
}

/// Registers the todo-22 text tool (IME editing) on `registry`.
pub fn register_text_tool(registry: &mut ToolRegistry) {
    registry.register(ToolKind::Text, || Box::new(TextTool::default()));
}

/// Registers the todo-23 destructive region tools (secure pixelate + its
/// blur variant) on `registry`.
pub fn register_pixelate_tools(registry: &mut ToolRegistry) {
    registry.register(ToolKind::Pixelate, || Box::new(PixelateTool::default()));
    registry.register(ToolKind::Blur, || {
        Box::new(PixelateTool::new(crate::editor::EffectKind::Blur))
    });
}

/// Registers the todo-24 circle-count tool on `registry`.
pub fn register_counter_tool(registry: &mut ToolRegistry) {
    registry.register(ToolKind::Counter, || Box::new(CounterTool::default()));
}

//! The panel property seams: the selected-object half of the
//! side-panel size slider and the color-wheel pick.
//!
//! [`EditorState::mutate_object`] is the PUBLIC
//! property-change funnel (ONE undo unit per change); this module maps the
//! two panel writes onto it:
//!
//! - size: the dispatched tool-size slot (thickness / corner radius / font
//!   size / marker width - the [`ToolSizes`](super::ToolSizes) dispatch
//!   table) written into the selected object's own size field, so a panel
//!   slider press updates BOTH the next stroke (runtime slot) and the
//!   selected object (journal), Flameshot's object-property parity;
//! - color: the wheel's picked draw color written into the selected object's
//!   color field.
//!
//! Kinds without the property (invert is a region effect: no size, no
//! color; the counter tool owns the bubble's geometry) map to [`None`]
//! and record NO journal unit - a property write that changes nothing must
//! not pollute the undo history (the no-op guard discipline).

use flowshot_core::scene::{Color as SceneColor, ToolObject, ToolObjectData};

use crate::render::f32_from_u32;

use super::EditorState;
use super::size::BASE_POINT_SIZE;

/// Writes the dispatched tool size into the object's own size field;
/// [`None`] for kinds without size semantics (invert, counter). Text stores
/// the RENDERED point size (`slot + ` [`BASE_POINT_SIZE`], the text
/// tool's commit value), rectangle stores the corner radius (the Flameshot
/// `drawRectangleSize` dispatch).
#[must_use]
fn sized(data: ToolObjectData, size: u32) -> Option<ToolObjectData> {
    let value = f32_from_u32(size);
    Some(match data {
        ToolObjectData::Pencil(mut object) => {
            object.thickness = value;
            ToolObjectData::Pencil(object)
        }
        ToolObjectData::Line(mut object) => {
            object.thickness = value;
            ToolObjectData::Line(object)
        }
        ToolObjectData::Arrow(mut object) => {
            object.thickness = value;
            ToolObjectData::Arrow(object)
        }
        ToolObjectData::Marker(mut object) => {
            object.width = value;
            ToolObjectData::Marker(object)
        }
        ToolObjectData::Rectangle(mut object) => {
            object.corner_radius = value;
            ToolObjectData::Rectangle(object)
        }
        ToolObjectData::Ellipse(mut object) => {
            object.stroke_width = value;
            ToolObjectData::Ellipse(object)
        }
        ToolObjectData::Text(mut object) => {
            object.font_size = f32_from_u32(size.saturating_add(BASE_POINT_SIZE));
            ToolObjectData::Text(object)
        }
        ToolObjectData::Counter(_) | ToolObjectData::Invert(_) => return None,
    })
}

/// Writes the draw color into the object's color field; [`None`] for the
/// invert effect (a region operation with no color of its own).
#[must_use]
fn colorized(data: ToolObjectData, color: SceneColor) -> Option<ToolObjectData> {
    Some(match data {
        ToolObjectData::Pencil(mut object) => {
            object.color = color;
            ToolObjectData::Pencil(object)
        }
        ToolObjectData::Line(mut object) => {
            object.color = color;
            ToolObjectData::Line(object)
        }
        ToolObjectData::Arrow(mut object) => {
            object.color = color;
            ToolObjectData::Arrow(object)
        }
        ToolObjectData::Marker(mut object) => {
            object.color = color;
            ToolObjectData::Marker(object)
        }
        ToolObjectData::Rectangle(mut object) => {
            object.color = color;
            ToolObjectData::Rectangle(object)
        }
        ToolObjectData::Ellipse(mut object) => {
            object.color = color;
            ToolObjectData::Ellipse(object)
        }
        ToolObjectData::Text(mut object) => {
            object.color = color;
            ToolObjectData::Text(object)
        }
        ToolObjectData::Counter(mut object) => {
            object.color = color;
            ToolObjectData::Counter(object)
        }
        ToolObjectData::Invert(_) => return None,
    })
}

impl EditorState {
    /// Resizes the selected object as ONE undo unit (the panel size
    /// slider's selected-object half; the runtime slot write is the caller's
    /// [`EditorState::set_tool_size`]). `false` when nothing is selected or
    /// the kind has no size field - nothing is recorded.
    pub fn resize_selected(&mut self, size: u32) -> bool {
        self.property_of_selected(|data| sized(data, size))
    }

    /// Recolors the selected object as ONE undo unit (the color
    /// wheel's selected-object half; the draw-color write is the caller's
    /// [`EditorState::set_color`]). `false` when nothing is selected or the
    /// kind has no color (invert) - nothing is recorded.
    pub fn recolor_selected(&mut self, color: SceneColor) -> bool {
        self.property_of_selected(|data| colorized(data, color))
    }

    /// The shared property write: pre-maps the selected object's data so a
    /// kind WITHOUT the property records no journal unit, then goes through
    /// the [`EditorState::mutate_object`] funnel.
    fn property_of_selected(
        &mut self,
        edit: impl FnOnce(ToolObjectData) -> Option<ToolObjectData>,
    ) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let mapped = self
            .scene
            .get_object(id)
            .map(ToolObject::to_data)
            .and_then(edit);
        let Some(next) = mapped else {
            return false;
        };
        self.mutate_object(id, |_| next)
    }
}

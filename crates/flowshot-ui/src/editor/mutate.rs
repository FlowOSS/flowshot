//! Object mutation: the atomic move drag and the property-change seam
//! (draft F27 undo spec).
//!
//! F27: "object-move = one atomic unit (backup at first move, push at
//! release)" - a press on an object arms the drag, the FIRST motion with a
//! non-zero delta takes the before-snapshot and every motion translates the
//! object live (no journal traffic), and the release pushes exactly ONE
//! (before, after) pair. A click without motion records nothing.
//!
//! The core scene vocabulary is closed to the UI layer (the pixel-effect
//! precedent), and
//! [`ToolObject`](flowshot_core::scene::ToolObject) carries no translate
//! method, so mutation goes through the core's OWN persistence roundtrip:
//! [`Scene::to_data`] -> edit the [`ToolObjectData`] -> [`Scene::from_data`].
//! Ids, z-order, and counter numbers are preserved by construction (the
//! roundtrip is the serde-persistence path), and the same funnel serves the
//! side panel's property changes ([`EditorState::mutate_object`]).

use flowshot_core::geometry::LogicalPoint;
use flowshot_core::scene::{Point as ScenePoint, Rect as SceneRect, Scene, ToolObjectData};

use crate::render::f32_from_f64;

use super::EditorState;
use super::undo::Snapshot;

/// The armed object-drag state (press routed to object select; release
/// pending). `before` is `None` until the first non-zero motion (F27).
#[derive(Debug)]
pub(super) struct ObjectMove {
    id: usize,
    before: Option<Snapshot>,
    last: LogicalPoint,
}

/// Translates serializable object geometry by `(dx, dy)`; every variant's
/// position fields shift, non-geometric fields (color, text, count, style)
/// are untouched.
#[must_use]
pub(super) fn translated(data: ToolObjectData, dx: f32, dy: f32) -> ToolObjectData {
    fn point(at: ScenePoint, dx: f32, dy: f32) -> ScenePoint {
        ScenePoint::new(at.x + dx, at.y + dy)
    }
    fn rect(at: SceneRect, dx: f32, dy: f32) -> SceneRect {
        SceneRect::new(at.x + dx, at.y + dy, at.width, at.height)
    }
    match data {
        ToolObjectData::Rectangle(mut object) => {
            object.rect = rect(object.rect, dx, dy);
            ToolObjectData::Rectangle(object)
        }
        ToolObjectData::Ellipse(mut object) => {
            object.rect = rect(object.rect, dx, dy);
            ToolObjectData::Ellipse(object)
        }
        ToolObjectData::Arrow(mut object) => {
            object.from = point(object.from, dx, dy);
            object.to = point(object.to, dx, dy);
            ToolObjectData::Arrow(object)
        }
        ToolObjectData::Text(mut object) => {
            object.position = point(object.position, dx, dy);
            ToolObjectData::Text(object)
        }
        ToolObjectData::Counter(mut object) => {
            object.center = point(object.center, dx, dy);
            ToolObjectData::Counter(object)
        }
        ToolObjectData::Pencil(mut object) => {
            for vertex in &mut object.points {
                *vertex = point(*vertex, dx, dy);
            }
            ToolObjectData::Pencil(object)
        }
        ToolObjectData::Line(mut object) => {
            object.from = point(object.from, dx, dy);
            object.to = point(object.to, dx, dy);
            ToolObjectData::Line(object)
        }
        ToolObjectData::Marker(mut object) => {
            object.from = point(object.from, dx, dy);
            object.to = point(object.to, dx, dy);
            ToolObjectData::Marker(object)
        }
        ToolObjectData::Invert(mut object) => {
            object.rect = rect(object.rect, dx, dy);
            ToolObjectData::Invert(object)
        }
    }
}

impl EditorState {
    /// Replaces one object's data through the core persistence roundtrip
    /// WITHOUT touching the journal (the live half of a move drag; callers
    /// own the undo pairing). Ids and z-order are preserved.
    pub(super) fn replace_object(
        &mut self,
        id: usize,
        edit: impl FnOnce(ToolObjectData) -> ToolObjectData,
    ) -> bool {
        let mut data = self.scene.to_data();
        let Some(object) = data.objects.get_mut(id) else {
            tracing::warn!(target: "flowshot_ui::editor", object = id, "mutate missed");
            return false;
        };
        *object = edit(object.clone());
        match Scene::from_data(data) {
            Ok(scene) => {
                self.scene = scene;
                true
            }
            // Unreachable from a valid scene (z_order passes through
            // untouched); handled honestly (the no-panic discipline).
            Err(error) => {
                tracing::warn!(
                    target: "flowshot_ui::editor",
                    error = %error,
                    "scene rebuild refused; mutation dropped"
                );
                false
            }
        }
    }

    /// Mutates one object's data as ONE undo unit - the side panel's
    /// property-change funnel ("property change" is a
    /// mutation unit). `false` when the id is invalid (nothing recorded).
    pub fn mutate_object(
        &mut self,
        id: usize,
        edit: impl FnOnce(ToolObjectData) -> ToolObjectData,
    ) -> bool {
        let before = self.snapshot();
        if !self.replace_object(id, edit) {
            return false;
        }
        self.undo.push(before, self.snapshot());
        tracing::info!(
            target: "flowshot_ui::editor",
            object = id,
            undo_depth = self.undo.undo_depth(),
            "object mutated"
        );
        true
    }

    /// Arms the object drag after a select-object press (the F27
    /// `TYPE_MOVESELECTION` fall-through; no snapshot yet - a click
    /// without motion must not create a journal entry).
    pub(super) fn begin_object_move(&mut self, id: usize, at: LogicalPoint) {
        self.object_move = Some(ObjectMove {
            id,
            before: None,
            last: at,
        });
    }

    /// Extends the armed drag to `at`: backs up the scene at the first
    /// non-zero delta, then translates the object live. `true` when the
    /// scene changed (the caller redraws).
    pub(super) fn extend_object_move(&mut self, at: LogicalPoint) -> bool {
        let Some(mut drag) = self.object_move.take() else {
            return false;
        };
        let dx = at.x.0 - drag.last.x.0;
        let dy = at.y.0 - drag.last.y.0;
        drag.last = at;
        if dx == 0.0 && dy == 0.0 {
            self.object_move = Some(drag);
            return false;
        }
        if drag.before.is_none() {
            drag.before = Some(self.snapshot());
        }
        let moved = self.replace_object(drag.id, |data| {
            translated(data, f32_from_f64(dx), f32_from_f64(dy))
        });
        if moved {
            self.object_move = Some(drag);
        }
        moved
    }

    /// Ends the armed drag: pushes the single (before, after) move unit
    /// when any motion happened (F27 push-at-release). `true` when a unit
    /// was recorded.
    pub(super) fn finish_object_move(&mut self) -> bool {
        let Some(drag) = self.object_move.take() else {
            return false;
        };
        let Some(before) = drag.before else {
            return false;
        };
        self.undo.push(before, self.snapshot());
        tracing::info!(
            target: "flowshot_ui::editor",
            object = drag.id,
            undo_depth = self.undo.undo_depth(),
            "object moved"
        );
        true
    }

    /// Abandons the armed drag, rolling the live motions back (Esc cascade
    /// stage 2 mid-drag; a drag that never moved has nothing to roll back).
    pub(super) fn cancel_object_move(&mut self) {
        let Some(drag) = self.object_move.take() else {
            return;
        };
        if let Some(before) = drag.before {
            self.restore(before);
            tracing::debug!(target: "flowshot_ui::editor", "object move cancelled");
        }
    }

    /// Whether an object drag is armed (the motion/release routing input).
    #[must_use]
    pub(super) const fn object_move_armed(&self) -> bool {
        self.object_move.is_some()
    }
}

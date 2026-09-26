//! Scene operations of the editor (plan todo 20): stroke commit, delete,
//! undo/redo, and the object hit-test/selection behind F27 priority 5.
//!
//! Every mutation is ONE undo unit (a full before/after snapshot pair -
//! the core [`UndoStack`] contract, F27 "snapshot approach kept"): the
//! draw-end commit, the Delete removal (the core renumbers counters), and
//! the todo-25 additions (move-release, text-commit, property change) all
//! push through [`EditorState::commit_object`]-style pairs. Undo/redo
//! restore whole scenes, so object ids are invalidated - the selection is
//! cleared on every restore (Flameshot deselects on undo too).

use flowshot_core::geometry::LogicalPoint;
use flowshot_core::scene::{Point as ScenePoint, ToolObject};

use crate::render::f32_from_f64;

use super::EditorState;

impl EditorState {
    /// The annotation scene (objects + z-order).
    #[must_use]
    pub const fn scene(&self) -> &flowshot_core::scene::Scene {
        &self.scene
    }

    /// The undo history (depth/limit introspection for the todo-26 panel).
    #[must_use]
    pub const fn undo_stack(&self) -> &flowshot_core::scene::UndoStack {
        &self.undo
    }

    /// Commits a finished object to the scene as ONE undo unit (the
    /// draw-end / edit-commit / todo-25 mutation funnel).
    pub fn commit_object(&mut self, object: Box<dyn ToolObject>) -> usize {
        let before = self.scene.clone();
        let id = self.scene.add_object(object);
        self.undo.push(before, self.scene.clone());
        tracing::info!(
            target: "flowshot_ui::editor",
            object = self.scene.get_object(id).map_or("?", ToolObject::type_id),
            objects = self.scene.object_count(),
            undo_depth = self.undo.undo_depth(),
            "object committed"
        );
        id
    }

    /// The selected object id, when any.
    #[must_use]
    pub const fn selected_object(&self) -> Option<usize> {
        self.selected
    }

    /// Selects the topmost object under `at` (F27 P5); returns the id.
    /// `None` deselects (a press that hit nothing).
    pub fn select_object_at(&mut self, at: LogicalPoint) -> Option<usize> {
        let hit = self.object_at(at);
        if hit != self.selected {
            self.selected = hit;
            if let Some(id) = hit {
                tracing::debug!(
                    target: "flowshot_ui::editor",
                    object = id,
                    kind = self.scene.get_object(id).map_or("?", ToolObject::type_id),
                    "object selected"
                );
            }
        }
        hit
    }

    /// Clears the object selection (Esc cascade stage 2, click-elsewhere).
    pub fn deselect_object(&mut self) {
        if self.selected.is_some() {
            tracing::debug!(target: "flowshot_ui::editor", "object deselected");
        }
        self.selected = None;
    }

    /// The topmost object whose bounding rect contains `at` (paint order,
    /// top-down - the Flameshot `selectToolItemAtPos` z-scan).
    #[must_use]
    pub fn object_at(&self, at: LogicalPoint) -> Option<usize> {
        let point = ScenePoint::new(f32_from_f64(at.x.0), f32_from_f64(at.y.0));
        self.scene.z_order().iter().rev().copied().find(|&id| {
            self.scene
                .get_object(id)
                .is_some_and(|object| object.bounding_rect().contains(point))
        })
    }

    /// Removes the selected object as ONE undo unit; the core scene owns
    /// the counter renumbering (plan todo 20: "Delete removes selected w/
    /// counter renumber (core op)"). `false` when nothing is selected.
    pub fn delete_selected(&mut self) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let before = self.scene.clone();
        let Some(removed) = self.scene.remove_object(id) else {
            return false;
        };
        self.undo.push(before, self.scene.clone());
        self.selected = None;
        tracing::info!(
            target: "flowshot_ui::editor",
            kind = removed.type_id(),
            objects = self.scene.object_count(),
            "object deleted"
        );
        true
    }

    /// One undo step; `true` when the scene changed (silent no-op at the
    /// history start - the core contract).
    pub fn undo(&mut self) -> bool {
        let Some(scene) = self.undo.undo() else {
            tracing::debug!(target: "flowshot_ui::editor", "undo at history start");
            return false;
        };
        self.scene = scene;
        self.selected = None;
        tracing::info!(
            target: "flowshot_ui::editor",
            objects = self.scene.object_count(),
            "undo applied"
        );
        true
    }

    /// One redo step; `true` when the scene changed.
    pub fn redo(&mut self) -> bool {
        let Some(scene) = self.undo.redo() else {
            tracing::debug!(target: "flowshot_ui::editor", "redo at history end");
            return false;
        };
        self.scene = scene;
        self.selected = None;
        tracing::info!(
            target: "flowshot_ui::editor",
            objects = self.scene.object_count(),
            "redo applied"
        );
        true
    }
}

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
use super::effect::PixelEffect;
use super::undo::{EditorUndo, Snapshot};

impl EditorState {
    /// The annotation scene (objects + z-order).
    #[must_use]
    pub const fn scene(&self) -> &flowshot_core::scene::Scene {
        &self.scene
    }

    /// The undo history (depth/limit introspection for the todo-26 panel).
    #[must_use]
    pub const fn undo_stack(&self) -> &EditorUndo {
        &self.undo
    }

    /// The baked pixel-overlay layer in paint order (todo 23; the shell
    /// syncs its textures and the todo-38 export composites it).
    #[must_use]
    pub fn pixel_effects(&self) -> &[PixelEffect] {
        &self.effects
    }

    /// The current (scene, effects) snapshot - the journal payload.
    pub(super) fn snapshot(&self) -> Snapshot {
        (self.scene.clone(), self.effects.clone())
    }

    /// Restores a full snapshot (the undo/redo application point). The
    /// armed object drag is dropped WITHOUT rollback - the restored scene
    /// already supersedes it.
    pub(super) fn restore(&mut self, snapshot: Snapshot) {
        (self.scene, self.effects) = snapshot;
        self.selected = None;
        self.object_move = None;
    }

    /// Commits a finished object to the scene as ONE undo unit (the
    /// draw-end / edit-commit / todo-25 mutation funnel).
    pub fn commit_object(&mut self, object: Box<dyn ToolObject>) -> usize {
        let before = self.snapshot();
        let id = self.scene.add_object(object);
        self.undo.push(before, self.snapshot());
        tracing::info!(
            target: "flowshot_ui::editor",
            object = self.scene.get_object(id).map_or("?", ToolObject::type_id),
            objects = self.scene.object_count(),
            undo_depth = self.undo.undo_depth(),
            "object committed"
        );
        id
    }

    /// Commits a baked pixel effect as ONE undo unit (the todo-23
    /// destructive-op funnel; the editor assigns the identity that drives
    /// the effect's texture id).
    pub fn commit_effect(&mut self, effect: PixelEffect) -> u64 {
        let before = self.snapshot();
        let id = self.next_effect;
        self.next_effect = self.next_effect.saturating_add(1);
        let effect = effect.with_id(id);
        let kind = effect.kind();
        let rect = effect.rect();
        self.effects.push(effect);
        self.undo.push(before, self.snapshot());
        tracing::info!(
            target: "flowshot_ui::editor",
            effect = kind.token(),
            id,
            x = rect.x.0,
            y = rect.y.0,
            w = rect.width.0,
            h = rect.height.0,
            effects = self.effects.len(),
            undo_depth = self.undo.undo_depth(),
            "effect committed"
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

    /// Clears the object selection (Esc cascade stage 2, click-elsewhere);
    /// an armed object drag is cancelled with rollback (its live motions
    /// never reached the journal).
    pub fn deselect_object(&mut self) {
        if self.selected.is_some() {
            tracing::debug!(target: "flowshot_ui::editor", "object deselected");
        }
        self.cancel_object_move();
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
        let before = self.snapshot();
        let Some(removed) = self.scene.remove_object(id) else {
            return false;
        };
        self.undo.push(before, self.snapshot());
        self.selected = None;
        // The delete unit captured the post-move scene; dropping the drag
        // WITHOUT rollback keeps the journal the single source of truth.
        self.object_move = None;
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
        let Some(snapshot) = self.undo.undo() else {
            tracing::debug!(target: "flowshot_ui::editor", "undo at history start");
            return false;
        };
        self.restore(snapshot);
        tracing::info!(
            target: "flowshot_ui::editor",
            objects = self.scene.object_count(),
            effects = self.effects.len(),
            "undo applied"
        );
        true
    }

    /// One redo step; `true` when the scene changed.
    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.undo.redo() else {
            tracing::debug!(target: "flowshot_ui::editor", "redo at history end");
            return false;
        };
        self.restore(snapshot);
        tracing::info!(
            target: "flowshot_ui::editor",
            objects = self.scene.object_count(),
            effects = self.effects.len(),
            "redo applied"
        );
        true
    }
}

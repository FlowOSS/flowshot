//! The move-selection drag seam: the Move tool's delta
//! funnel - live translation of the selection rect plus every contained
//! scene object, first-motion snapshot arming, the release commit as ONE
//! undo unit, and the Esc rollback. Split from the facade at the 250-LOC
//! ceiling (the `events/pointer.rs` child-module pattern).

use super::{EditorState, ToolKind, mutate, tools};

impl EditorState {
    /// The move-selection drag delta, when the active tool is Move and a drag
    /// is in progress (the `OverlayCore` applies this to the selection state
    /// and contained objects).
    #[must_use]
    pub fn move_selection_delta(&mut self) -> Option<(f64, f64)> {
        if self.active_kind != Some(ToolKind::Move) {
            return None;
        }
        let tool = self.tool.as_mut()?;
        let move_tool = tool
            .as_any_mut()
            .downcast_mut::<tools::MoveSelectionTool>()?;
        move_tool.take_delta()
    }

    /// Translates the selection rect and all contained objects by the given
    /// delta (the move-selection tool's drag seam). Returns true when the
    /// scene changed.
    pub fn translate_selection_and_objects(
        &mut self,
        selection: &mut crate::selection::SelectionState,
        dx: f64,
        dy: f64,
    ) -> bool {
        if dx == 0.0 && dy == 0.0 {
            return false;
        }
        // Snapshot on first non-zero delta (the mutation discipline: backup at first move).
        // Include the selection rect in the snapshot for move-selection undo.
        if self.move_selection_before.is_none() {
            self.move_selection_before = Some(self.snapshot_with_selection(selection.rect()));
        }
        let Some(rect) = selection.rect() else {
            return false;
        };
        let new_rect = flowshot_core::geometry::LogicalRect::from_raw(
            rect.x.0 + dx,
            rect.y.0 + dy,
            rect.width.0,
            rect.height.0,
        );
        selection.set_rect(Some(new_rect));

        let mut changed = false;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "selection rect is bounded by screen dimensions"
        )]
        let scene_rect = flowshot_core::scene::Rect::new(
            new_rect.x.0 as f32,
            new_rect.y.0 as f32,
            new_rect.width.0 as f32,
            new_rect.height.0 as f32,
        );
        for id in 0..self.scene.object_count() {
            if let Some(object) = self.scene.get_object(id) {
                let bounds = object.bounding_rect();
                // Manual AABB intersection check.
                let intersects = bounds.x < scene_rect.x + scene_rect.width
                    && bounds.x + bounds.width > scene_rect.x
                    && bounds.y < scene_rect.y + scene_rect.height
                    && bounds.y + bounds.height > scene_rect.y;
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "delta values are bounded by screen dimensions"
                )]
                let delta_x = dx as f32;
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "delta values are bounded by screen dimensions"
                )]
                let delta_y = dy as f32;
                if intersects
                    && self.replace_object(id, |data| mutate::translated(data, delta_x, delta_y))
                {
                    changed = true;
                }
            }
        }
        changed
    }

    /// Commits the move-selection drag as one undo unit (called on release).
    /// Returns true when an undo unit was recorded.
    pub fn commit_move_selection(&mut self) -> bool {
        if self.active_kind != Some(ToolKind::Move) {
            return false;
        }
        let Some(before) = self.move_selection_before.take() else {
            return false;
        };
        self.undo.push(before, self.snapshot());
        tracing::info!(
            target: "flowshot_ui::editor",
            undo_depth = self.undo.undo_depth(),
            "move-selection committed"
        );
        true
    }

    /// Cancels the armed move-selection drag, rolling the live motions back
    /// (Esc cascade stage 1 mid-drag; a drag that never moved has nothing to
    /// roll back).
    pub fn cancel_move_selection(&mut self) {
        let Some(before) = self.move_selection_before.take() else {
            return;
        };
        self.restore(before);
        tracing::debug!(target: "flowshot_ui::editor", "move-selection cancelled");
    }
}

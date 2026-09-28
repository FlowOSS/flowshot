//! Z-order operations and the layer-list model.
//!
//! Raise/lower the SELECTED object through the core scene's z-order ops
//! (`raise`/`lower`/`raise_to_top`/`lower_to_bottom`), each as ONE
//! undo unit; an op that cannot move (already at the edge, nothing
//! selected) records nothing. Parity: z-order is PANEL-DRIVEN with NO
//! default keys - the side panel hosts the buttons
//! and the layer list this module models ([`EditorState::layers`]:
//! bottom-to-top entries, click = select, drag = [`EditorState::move_layer`]
//! which maps to the same core ops as one undo unit). Optional key slots
//! exist in [`ToolShortcuts`](super::ToolShortcuts) for config/QA rebinding
//! only (the blur-rebind precedent); they ship unbound.

use flowshot_core::scene::{Scene, ToolObject};

use super::EditorState;

/// One layer-list row (the side-panel model), listed BOTTOM-TO-TOP
/// (paint order, matching [`Scene::z_order`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerEntry {
    /// Paint position: `0` = bottom-most.
    pub z: usize,
    /// Scene object id (the select/mutate handle).
    pub id: usize,
    /// Stable type token for the row icon ([`ToolObject::type_id`] family).
    pub kind: &'static str,
}

impl EditorState {
    /// The layer list, bottom-to-top (the side-panel rows).
    #[must_use]
    pub fn layers(&self) -> Vec<LayerEntry> {
        self.scene
            .z_order()
            .iter()
            .enumerate()
            .filter_map(|(z, &id)| {
                self.scene.get_object(id).map(|object| LayerEntry {
                    z,
                    id,
                    kind: object.type_id(),
                })
            })
            .collect()
    }

    /// Selects a layer by id (panel click = select). `false` for an
    /// invalid id.
    pub fn select_layer(&mut self, id: usize) -> bool {
        let Some(kind) = self.scene.get_object(id).map(ToolObject::type_id) else {
            return false;
        };
        self.selected = Some(id);
        tracing::debug!(target: "flowshot_ui::editor", object = id, kind, "layer selected");
        true
    }

    /// Raises the selected object one step (panel button; one undo unit).
    /// `false` when nothing is selected or it is already top-most.
    pub fn raise_selected(&mut self) -> bool {
        self.z_op("raise", Scene::raise)
    }

    /// Lowers the selected object one step (panel button; one undo unit).
    /// `false` when nothing is selected or it is already bottom-most.
    pub fn lower_selected(&mut self) -> bool {
        self.z_op("lower", Scene::lower)
    }

    /// Moves the selected object to the top (one undo unit).
    pub fn raise_selected_to_top(&mut self) -> bool {
        self.z_op("raise-to-top", Scene::raise_to_top)
    }

    /// Moves the selected object to the bottom (one undo unit).
    pub fn lower_selected_to_bottom(&mut self) -> bool {
        self.z_op("lower-to-bottom", Scene::lower_to_bottom)
    }

    /// Reorders the layer at paint position `from_z` to `to_z` (panel
    /// drag-drop) as ONE undo unit, mapped onto the core single-step ops.
    /// `false` for out-of-range or identity reorders.
    pub fn move_layer(&mut self, from_z: usize, to_z: usize) -> bool {
        let count = self.scene.object_count();
        if from_z >= count || to_z >= count || from_z == to_z {
            return false;
        }
        let Some(&id) = self.scene.z_order().get(from_z) else {
            return false;
        };
        let before = self.snapshot();
        let step = if to_z > from_z {
            Scene::raise
        } else {
            Scene::lower
        };
        for _ in 0..to_z.abs_diff(from_z) {
            if !step(&mut self.scene, id) {
                break;
            }
        }
        self.undo.push(before, self.snapshot());
        tracing::info!(
            target: "flowshot_ui::editor",
            object = id,
            from = from_z,
            to = to_z,
            undo_depth = self.undo.undo_depth(),
            "layer moved"
        );
        true
    }

    /// The shared z-op wrapper: selected id + core op + one undo unit only
    /// when the paint position actually changed (the core single-steps
    /// return `false` at the edges; the to-top/to-bottom jumps succeed on
    /// any valid id, so the position check is the no-op guard - no no-op
    /// journal entries).
    fn z_op(&mut self, token: &'static str, op: fn(&mut Scene, usize) -> bool) -> bool {
        let Some(id) = self.selected else {
            tracing::debug!(target: "flowshot_ui::editor", op = token, "z-order without selection");
            return false;
        };
        let before_z = self.scene.z_index(id);
        let before = self.snapshot();
        if !op(&mut self.scene, id) || self.scene.z_index(id) == before_z {
            return false;
        }
        self.undo.push(before, self.snapshot());
        tracing::info!(
            target: "flowshot_ui::editor",
            object = id,
            op = token,
            z = self.scene.z_index(id).unwrap_or(usize::MAX),
            undo_depth = self.undo.undo_depth(),
            "z-order changed"
        );
        true
    }
}

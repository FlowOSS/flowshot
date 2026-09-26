//! The editor's unified undo journal (plan todo 23's destructive-op undo).
//!
//! The core [`UndoStack`](flowshot_core::scene::UndoStack) snapshots
//! [`Scene`]s only; todo 23's baked pixel effects are editor-local overlay
//! state (the core scene vocabulary is closed to this task), and a
//! destructive op must undo ATOMICALLY with the scene - one Ctrl+Z removes
//! exactly the last mutation whether it was an annotation or a redaction,
//! in true interleaved order. This journal therefore stores
//! `(scene, effects)` snapshot pairs with the core `UndoStack` semantics
//! MIRRORED EXACTLY (full before/after pairs, cursor, redo-tail truncation
//! on push, oldest-eviction over the limit, silent no-ops at the ends,
//! limit 0 disables history): todo 25's wiring (config `undo_limit`,
//! move-release units, z-order ops) keeps working against the same
//! contract, and effect snapshots are cheap because the baked buffers are
//! `Arc`-shared ([`PixelEffect::clone`] bumps refcounts, never pixels).

use flowshot_core::geometry::LogicalRect;
use flowshot_core::scene::Scene;

use super::effect::PixelEffect;

/// One undoable editor state: the annotation scene plus the baked
/// pixel-overlay layer, in paint order, plus the selection rect (for
/// move-selection undo).
pub type Snapshot = (Scene, Vec<PixelEffect>, Option<LogicalRect>);

/// Snapshot-pair undo history over [`Snapshot`]s (the core `UndoStack`
/// contract, extended payload).
#[derive(Debug, Clone)]
pub struct EditorUndo {
    snapshots: Vec<(Snapshot, Snapshot)>,
    cursor: usize,
    limit: usize,
}

impl EditorUndo {
    /// Creates a stack bounded by `limit` snapshots (0 disables history).
    #[must_use]
    pub const fn with_limit(limit: usize) -> Self {
        Self {
            snapshots: Vec::new(),
            cursor: 0,
            limit,
        }
    }

    /// Creates a stack bounded by the config `undo_limit` value (the core
    /// `UndoStack::from_undo_limit` parity).
    #[must_use]
    pub fn from_undo_limit(limit: u32) -> Self {
        Self::with_limit(usize::try_from(limit).unwrap_or(usize::MAX))
    }

    /// Records a modification as a full before/after snapshot pair:
    /// discards the redo tail, then evicts oldest entries while over limit
    /// (the core semantics, line for line).
    pub fn push(&mut self, before: Snapshot, after: Snapshot) {
        self.snapshots.truncate(self.cursor);
        self.snapshots.push((before, after));
        while self.snapshots.len() > self.limit {
            self.snapshots.remove(0);
        }
        self.cursor = self.snapshots.len();
    }

    /// Steps one modification back, returning the restored (before)
    /// snapshot; silent no-op (`None`) at the history start.
    pub fn undo(&mut self) -> Option<Snapshot> {
        let index = self.cursor.checked_sub(1)?;
        let snapshot = self.snapshots.get(index)?.0.clone();
        self.cursor = index;
        Some(snapshot)
    }

    /// Steps one modification forward, returning the reapplied (after)
    /// snapshot; silent no-op (`None`) at the history end.
    pub fn redo(&mut self) -> Option<Snapshot> {
        let snapshot = self.snapshots.get(self.cursor)?.1.clone();
        self.cursor = self.cursor.saturating_add(1);
        Some(snapshot)
    }

    /// Drops all history.
    pub fn clear(&mut self) {
        self.snapshots.clear();
        self.cursor = 0;
    }

    /// Whether [`EditorUndo::undo`] would restore a state.
    #[must_use]
    pub const fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    /// Whether [`EditorUndo::redo`] would reapply a state.
    #[must_use]
    pub const fn can_redo(&self) -> bool {
        self.cursor < self.snapshots.len()
    }

    /// Number of snapshots held (undo depth plus redo tail).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether no history is held.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Available undo steps from the current position (the core
    /// `undo_depth` parity).
    #[must_use]
    pub const fn undo_depth(&self) -> usize {
        self.cursor
    }

    /// The configured snapshot limit.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Updates the limit, evicting oldest entries if now over bound (the
    /// core `set_limit` parity).
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        while self.snapshots.len() > self.limit {
            self.snapshots.remove(0);
            self.cursor = self.cursor.saturating_sub(1);
        }
    }
}

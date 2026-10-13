//! The editor's unified undo journal (the destructive-op undo).
//!
//! Reuses the core [`UndoStack`] generic over the editor's extended
//! [`Snapshot`] payload, keeping the core contract (before/after snapshot
//! pairs, redo-tail truncation on push, oldest-eviction over the limit, and
//! atomic scene + effect restoration).

use flowshot_core::geometry::LogicalRect;
use flowshot_core::scene::{Scene, UndoStack};

use super::effect::PixelEffect;

/// One undoable editor state: the annotation scene plus the baked
/// pixel-overlay layer, in paint order, plus the selection rect (for
/// move-selection undo).
pub type Snapshot = (Scene, Vec<PixelEffect>, Option<LogicalRect>);

/// The editor's undo stack, powered by the core generic [`UndoStack`].
pub type EditorUndo = UndoStack<Snapshot>;

//! Scroll area widget.

use crate::render::{DisplayList, Rect};
use flowshot_core::tokens::DesignTokens;

/// A scroll area widget.
#[derive(Debug, Clone)]
pub struct ScrollArea {
    /// The bounding rectangle.
    pub rect: Rect,
}

impl ScrollArea {
    /// Creates a new scroll area.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self { rect }
    }

    /// Draws the scroll area into the display list.
    pub fn draw(&self, _list: &mut DisplayList, _tokens: &DesignTokens, _scale: f32) {
        // Scroll area just clips its content.
        // The actual clipping is done by the caller using `list.push_clip`.
    }
}

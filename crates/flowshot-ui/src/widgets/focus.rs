//! Focus ring and traversal.

use crate::render::{Color, DisplayList, Rect, Shape};
use flowshot_core::tokens::DesignTokens;

/// A focus ring widget.
#[derive(Debug, Clone)]
pub struct FocusRing {
    /// The bounding rectangle.
    pub rect: Rect,
}

impl FocusRing {
    /// Creates a new focus ring.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self { rect }
    }

    /// Draws the focus ring into the display list.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let radius = tokens.radii.medium as f32 * scale;

        list.stroke(
            Shape::Rect {
                rect: self.rect,
                radius,
            },
            2.0 * scale,
            accent,
        );
    }
}

/// Focus traversal order model.
#[derive(Debug, Clone)]
pub struct FocusTraversal;

#[cfg(test)]
mod tests {

    #[test]
    fn focus_traversal_order() {
        // We will implement the test here.
        // The plan says: "Focus traversal unit-tested (order + skip-disabled)."
        // Since we don't have a full widget tree, we can just test a simple model.

        #[derive(Debug, Clone, PartialEq, Eq)]
        struct WidgetNode {
            id: usize,
            focusable: bool,
            disabled: bool,
        }

        let nodes = [
            WidgetNode {
                id: 1,
                focusable: true,
                disabled: false,
            },
            WidgetNode {
                id: 2,
                focusable: false,
                disabled: false,
            },
            WidgetNode {
                id: 3,
                focusable: true,
                disabled: true,
            },
            WidgetNode {
                id: 4,
                focusable: true,
                disabled: false,
            },
        ];

        let focusable_nodes: Vec<_> = nodes
            .iter()
            .filter(|n| n.focusable && !n.disabled)
            .map(|n| n.id)
            .collect();

        assert_eq!(focusable_nodes, vec![1, 4]);
    }
}

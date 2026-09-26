//! The grid overlay paint (plan todo 27: `[editor].grid` config + toggle
//! key, spacing token, 1px lines, drawn UNDER annotations ABOVE backdrop).
//! Split from the facade at the 250-LOC ceiling.

use super::EditorState;

impl EditorState {
    /// Paints the grid overlay (plan todo 27: spacing token, 1px lines,
    /// drawn UNDER annotations ABOVE backdrop).
    pub fn paint_grid(
        &self,
        list: &mut crate::render::DisplayList,
        output: &flowshot_core::geometry::OutputInfo,
    ) {
        use crate::render::{Color, Shape};

        if !self.grid_visible {
            return;
        }

        #[allow(
            clippy::cast_precision_loss,
            reason = "grid spacing is a small integer, precision loss is acceptable"
        )]
        let spacing = self.config.editor.draw_thickness.max(8) as f32 * 4.0;
        let color = Color::from_rgba8(128, 128, 128, 64);

        #[allow(
            clippy::cast_precision_loss,
            reason = "physical dimensions are bounded, precision loss is acceptable"
        )]
        let width = output.physical_size.width.0 as f32;
        #[allow(
            clippy::cast_precision_loss,
            reason = "physical dimensions are bounded, precision loss is acceptable"
        )]
        let height = output.physical_size.height.0 as f32;

        let mut x = 0.0;
        while x < width {
            list.fill(
                Shape::Rect {
                    rect: crate::render::Rect::from_parts(x, 0.0, 1.0, height),
                    radius: 0.0,
                },
                color,
            );
            x += spacing;
        }

        let mut y = 0.0;
        while y < height {
            list.fill(
                Shape::Rect {
                    rect: crate::render::Rect::from_parts(0.0, y, width, 1.0),
                    radius: 0.0,
                },
                color,
            );
            y += spacing;
        }
    }
}

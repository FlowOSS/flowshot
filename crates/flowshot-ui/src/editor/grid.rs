//! The grid overlay paint (`[editor].grid` config + toggle
//! key, spacing token, 1px lines, drawn UNDER annotations ABOVE backdrop).
//! Split from the facade at the 250-LOC ceiling.

use super::EditorState;

/// The grid line ink: neutral gray at 25% (audit JUSTIFIED: the
/// grid overlays arbitrary wallpaper AND arbitrary annotations, so it must
/// not take the brand hue or the contrast token - a neutral ink is the only
/// color readable over every backdrop; the magnifier's `GRID_COLOR` is the
/// same convention at its own alpha).
const GRID_LINE_COLOR: crate::render::Color = crate::render::Color {
    r: 128.0 / 255.0,
    g: 128.0 / 255.0,
    b: 128.0 / 255.0,
    a: 64.0 / 255.0,
};

impl EditorState {
    /// Paints the grid overlay (spacing token, 1px lines,
    /// drawn UNDER annotations ABOVE backdrop).
    pub fn paint_grid(
        &self,
        list: &mut crate::render::DisplayList,
        output: &flowshot_core::geometry::OutputInfo,
    ) {
        use crate::render::Shape;

        if !self.grid_visible {
            return;
        }

        #[allow(
            clippy::cast_precision_loss,
            reason = "grid spacing is a small integer, precision loss is acceptable"
        )]
        let spacing = self.config.editor.draw_thickness.max(8) as f32 * 4.0;
        let color = GRID_LINE_COLOR;

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

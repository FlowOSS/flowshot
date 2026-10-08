//! The eyedropper tool.
//!
//! The eyedropper samples the frozen-frame pixel at the click position and
//! sets the draw color to the sampled value. Activated by `G` (the default
//! Flameshot binding for `TYPE_GRAB_COLOR`).
//!
//! Flameshot parity: the picker follows the cursor (magnifier-follows mode),
//! and a click samples the frozen-frame pixel. The sampled color is set as
//! the new draw color, and optionally copied to the clipboard (the plan's
//! "wayshot --color equivalence" seam).
//!
//! Implementation: the tool samples the frozen frame (installed by the
//! shell via `EditorState::install_frame`) at the click position. The
//! sampling converts global logical coordinates to physical pixels in the
//! frame, then reads the RGBA value. The editor's `set_color` method
//! applies the sampled color.

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{Color as SceneColor, PaintSink, ToolObject};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, FramePixels, Tool};

/// The eyedropper tool (Flameshot `TYPE_GRAB_COLOR` parity).
///
/// Samples the frozen-frame pixel at the click position and sets the draw
/// color. Activated by `G` key.
#[derive(Debug, Default)]
pub struct EyedropperTool {
    sampled: Option<SceneColor>,
}

impl EyedropperTool {
    /// The last sampled color, when any.
    #[must_use]
    pub const fn sampled(&self) -> Option<SceneColor> {
        self.sampled
    }

    /// Samples the frozen frame at the given global logical position.
    ///
    /// Returns `None` when no frame is installed or the position is outside
    /// the frame bounds.
    #[must_use]
    pub fn sample(frame: &FramePixels, at: LogicalPoint) -> Option<SceneColor> {
        // Convert global logical to frame-local physical.
        let local_x = (at.x.0 - frame.origin.x.0) * frame.scale;
        let local_y = (at.y.0 - frame.origin.y.0) * frame.scale;

        // Bounds check.
        if local_x < 0.0
            || local_y < 0.0
            || local_x >= f64::from(frame.width)
            || local_y >= f64::from(frame.height)
        {
            return None;
        }

        // Sample the pixel (nearest-neighbor).
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounds-checked above"
        )]
        let px = local_x as u32;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounds-checked above"
        )]
        let py = local_y as u32;
        let offset = ((py * frame.width + px) * 4) as usize;

        // FramePixels is RGBA8888 row-major.
        let r = frame.rgba.get(offset).copied()?;
        let g = frame.rgba.get(offset + 1).copied()?;
        let b = frame.rgba.get(offset + 2).copied()?;
        let a = frame.rgba.get(offset + 3).copied()?;

        Some(SceneColor::new(r, g, b, a))
    }
}

impl Tool for EyedropperTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Eyedropper
    }

    fn pressed(
        &mut self,
        ctx: &EditorContext<'_>,
        _button: winit::event::MouseButton,
        at: LogicalPoint,
    ) -> bool {
        // Sample the frame at the click position.
        if let Some(frame) = ctx.frame
            && let Some(color) = Self::sample(frame, at)
        {
            self.sampled = Some(color);
            tracing::info!(
                target: "flowshot_ui::editor",
                r = color.r,
                g = color.g,
                b = color.b,
                a = color.a,
                "eyedropper sampled"
            );
            return true;
        }
        // Click outside selection or no frame: ignored (plan failure case).
        tracing::debug!(target: "flowshot_ui::editor", "eyedropper click ignored");
        false
    }

    fn draw_start(&mut self, _ctx: &EditorContext<'_>, _at: LogicalPoint) {}

    fn draw_move(&mut self, _ctx: &EditorContext<'_>, _at: LogicalPoint) {}

    fn draw_end(
        &mut self,
        _ctx: &EditorContext<'_>,
        _at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        None
    }

    fn paint(&self, _ctx: &EditorContext<'_>, _sink: &mut dyn PaintSink) {}

    fn bounding_rect(&self) -> Option<LogicalRect> {
        None
    }

    fn is_valid(&self) -> bool {
        self.sampled.is_some()
    }

    fn show_mouse_preview(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use flowshot_core::geometry::LogicalPoint;

    fn fixture_frame() -> FramePixels {
        // 4x4 frame with a red pixel at (1, 1) and green at (2, 2).
        let mut rgba = vec![0u8; 4 * 4 * 4];
        // Red at (1, 1): offset = (1 * 4 + 1) * 4 = 20
        rgba[20] = 255;
        rgba[21] = 0;
        rgba[22] = 0;
        rgba[23] = 255;
        // Green at (2, 2): offset = (2 * 4 + 2) * 4 = 40
        rgba[40] = 0;
        rgba[41] = 255;
        rgba[42] = 0;
        rgba[43] = 255;

        FramePixels {
            rgba,
            width: 4,
            height: 4,
            scale: 1.0,
            origin: LogicalPoint::from_raw(0.0, 0.0),
        }
    }

    #[test]
    fn eyedropper_samples_exact_pixel_color() {
        let frame = fixture_frame();
        // Sample red at (1, 1).
        let color = EyedropperTool::sample(&frame, LogicalPoint::from_raw(1.0, 1.0));
        assert_eq!(color, Some(SceneColor::new(255, 0, 0, 255)));

        // Sample green at (2, 2).
        let color = EyedropperTool::sample(&frame, LogicalPoint::from_raw(2.0, 2.0));
        assert_eq!(color, Some(SceneColor::new(0, 255, 0, 255)));
    }

    #[test]
    fn eyedropper_click_outside_selection_ignored() {
        let frame = fixture_frame();
        // Sample outside the frame bounds.
        let color = EyedropperTool::sample(&frame, LogicalPoint::from_raw(10.0, 10.0));
        assert!(color.is_none());

        let color = EyedropperTool::sample(&frame, LogicalPoint::from_raw(-1.0, 0.0));
        assert!(color.is_none());
    }

    #[test]
    fn eyedropper_respects_frame_origin() {
        let mut frame = fixture_frame();
        frame.origin = LogicalPoint::from_raw(100.0, 100.0);

        // Sample at global (101, 101) = local (1, 1) = red.
        let color = EyedropperTool::sample(&frame, LogicalPoint::from_raw(101.0, 101.0));
        assert_eq!(color, Some(SceneColor::new(255, 0, 0, 255)));
    }

    #[test]
    fn eyedropper_respects_frame_scale() {
        let mut frame = fixture_frame();
        frame.scale = 2.0; // HiDPI: 2 physical pixels per logical pixel.

        // Sample at global (0.5, 0.5) = local physical (1, 1) = red.
        let color = EyedropperTool::sample(&frame, LogicalPoint::from_raw(0.5, 0.5));
        assert_eq!(color, Some(SceneColor::new(255, 0, 0, 255)));
    }
}

//! The destructive region tools (plan todo 23): secure pixelate + blur.
//!
//! One two-point drag tool type in the Flameshot `pixelatetool.cpp` mold
//! with the two F27 modes (blur is Flameshot's size-driven pixelate
//! variant, so both share the `[tools.pixelate].size` slot): the live
//! preview paints the drag region BLACK (the `drawSearchArea` parity - the
//! effect is only revealed on release), there is no cursor preview dot
//! (`paintMousePreview` is a no-op), and the release BAKES the redacted
//! pixels from the pristine frozen frame into a [`PixelEffect`] the editor
//! commits to its pixel-overlay layer as ONE undo unit (the
//! [`Tool::draw_end_effect`] channel - these tools produce no scene object;
//! see [`super::super::effect`] for the mechanism).
//!
//! Region rules (plan todo 23): the drag rect is clamped to the selection
//! bounds (when a selection exists) and to the frame; a zero-length drag,
//! an empty intersection, or a region whose F27 output grid collapses to
//! zero commits NOTHING (the 1x1 no-op failure path, no panic).

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{
    Color as SceneColor, PaintSink, Point as ScenePoint, Rect as SceneRect,
};

use super::super::blur::bake_blur;
use super::super::effect::{Bake, EffectKind, PixelEffect, frame_region};
use super::super::kind::ToolKind;
use super::super::pixelate::bake_pixelate;
use super::super::tool::{EditorContext, Tool};
use super::geometry::{Constrain, TwoPoint, logical};

/// The `drawSearchArea` preview color (Flameshot fills the pending region
/// black).
const PREVIEW_BLACK: SceneColor = SceneColor::new(0, 0, 0, 255);

/// The destructive region tool: the F27 secure pseudo-pixelation (default
/// mode, F12 key B - the ONLY pixelate mode; the insecure reversible mosaic
/// is dropped per Amendment #3) or its gaussian blur variant (unbound by
/// default like counter/move; the todo-26 panel exposes the mode switch,
/// QA harnesses rebind the key).
#[derive(Debug)]
pub struct PixelateTool {
    mode: EffectKind,
    stroke: TwoPoint,
    size: u32,
}

impl Default for PixelateTool {
    fn default() -> Self {
        Self::new(EffectKind::Pixelate)
    }
}

impl PixelateTool {
    /// Builds the tool for one of its two modes.
    #[must_use]
    pub fn new(mode: EffectKind) -> Self {
        Self {
            mode,
            stroke: TwoPoint::default(),
            size: 0,
        }
    }

    /// The mode this instance bakes.
    #[must_use]
    pub const fn mode(&self) -> EffectKind {
        self.mode
    }

    /// The release bake: endpoints -> normalized drag rect -> clamped to
    /// frame and selection -> physical bake region -> baked overlay effect.
    fn bake(
        &self,
        ctx: &EditorContext<'_>,
        from: ScenePoint,
        to: ScenePoint,
    ) -> Option<PixelEffect> {
        let frame = ctx.frame?;
        let rect = frame
            .logical_rect()
            .intersection(&logical_bounds(from, to))?;
        let rect = match ctx.selection {
            Some(selection) => rect.intersection(&selection)?,
            None => rect,
        };
        let region = frame_region(frame, rect)?;
        let pixels = match self.mode {
            EffectKind::Pixelate => bake_pixelate(frame, region, self.size),
            EffectKind::Blur => bake_blur(frame, region, self.size),
        }?;
        Some(PixelEffect::new(
            self.mode,
            Bake {
                rect,
                region,
                pixels,
            },
        ))
    }
}

impl Tool for PixelateTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        match self.mode {
            EffectKind::Pixelate => ToolKind::Pixelate,
            EffectKind::Blur => ToolKind::Blur,
        }
    }

    fn draw_start(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.stroke.start(at);
    }

    fn draw_move(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.stroke.extend(at);
    }

    fn draw_end_effect(
        &mut self,
        ctx: &EditorContext<'_>,
        at: LogicalPoint,
    ) -> Option<PixelEffect> {
        self.stroke.extend(at);
        let (from, to) = self.stroke.finish(ctx, Constrain::Free)?;
        self.bake(ctx, from, to)
    }

    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        if let Some((from, to)) = self.stroke.endpoints(ctx, Constrain::Free) {
            sink.fill_rect(SceneRect::from_points(from, to), PREVIEW_BLACK);
        }
    }

    fn bounding_rect(&self) -> Option<LogicalRect> {
        self.stroke.raw_bounds().map(logical)
    }

    fn is_valid(&self) -> bool {
        self.stroke.drawing()
    }

    fn on_size_changed(&mut self, size: u32) {
        self.size = size;
    }

    fn show_mouse_preview(&self) -> bool {
        false
    }
}

/// The normalized drag bounds in global logical space (scene f32 -> f64 is
/// exact; a zero-width/height drag survives here and is rejected by the
/// bake-region mapping or the grid formula).
fn logical_bounds(from: ScenePoint, to: ScenePoint) -> LogicalRect {
    let x0 = f64::from(from.x.min(to.x));
    let y0 = f64::from(from.y.min(to.y));
    let x1 = f64::from(from.x.max(to.x));
    let y1 = f64::from(from.y.max(to.y));
    LogicalRect::from_raw(x0, y0, x1 - x0, y1 - y0)
}

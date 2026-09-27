//! The overlay completion boundary (plan todo 38).
//!
//! When a capture-completing gesture fires (Enter accept, Ctrl+C /
//! double-click copy, `--instant` release, or a toolbar action button), the
//! shell renders the final export offscreen through the REAL production
//! render path (frozen backdrop 1:1 + the todo-23 pixel-effect quads + the
//! annotation scene - no dim, no selection chrome, no crosshair, no
//! magnifier, no grid), crops it per output in PHYSICAL pixels, composites
//! the crops into one image, and hands it to the installed
//! [`CompletionSink`]. The binary layer (todo 35/38 executor) owns encoding
//! and the post-capture actions; this crate stays platform-pure.
//!
//! # Pixel space of the export (physical-first, #4871)
//!
//! Each output contributes its crop at native physical resolution (the
//! window surface extent IS the output's post-transform physical size, and
//! the frozen frame renders into it 1:1). Destination placement follows
//! the grim composite model: an output's crop lands at its logical offset
//! from the selection origin multiplied by THAT output's own scale - never
//! an averaged factor. At uniform scale the canvas is exactly
//! `selection logical size * scale` with zero resampling; at mixed scales
//! crops keep their native pixels (documented: placement can leave a seam
//! pixel of overlap/gap, the same compromise the portal composite makes).

use flowshot_core::geometry::{LogicalRect, OutputLayout};
use flowshot_core::scene::Color as SceneColor;

/// Which gesture completed the capture (the binary layer maps it onto the
/// effective post-capture action set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    /// Enter / instant release / toolbar accept: run the configured
    /// `[save].actions` sequence merged with the invocation flags.
    Accept,
    /// Ctrl+C / double-click / toolbar copy: clipboard only.
    Copy,
    /// Toolbar save: save to disk only.
    Save,
    /// Toolbar pin: pin to screen only.
    Pin,
    /// Toolbar upload: upload only.
    Upload,
    /// Toolbar open-app: save + open with another application.
    OpenWith,
}

/// The exported image: upright RGBA bytes, physical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedImage {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// Row-major RGBA bytes (`width * height * 4`).
    pub rgba: Vec<u8>,
}

/// One completed capture handed to the binary layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// The completing gesture.
    pub kind: CompletionKind,
    /// The accepted selection in global logical space (the
    /// `--print-geometry` / region-memory source).
    pub selection: LogicalRect,
    /// The rendered export.
    pub image: ExportedImage,
}

/// The completion callback (the `RegionSink`/`DrawColorSink` pattern: the
/// lib stays pure, the binary layer owns encoding, clipboard, and files).
pub type CompletionSink = Box<dyn Fn(Completion) + Send + 'static>;

/// The standalone color-pick callback (`flowshot color`): fired when the
/// eyedropper samples a pixel.
pub type ColorPickSink = Box<dyn Fn(SceneColor) + Send + 'static>;

/// One window's offscreen export render (post-transform physical RGBA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedOutput {
    /// The layout output index this render belongs to.
    pub output_index: usize,
    /// Surface width in physical pixels.
    pub width: u32,
    /// Surface height in physical pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub rgba: Vec<u8>,
}

/// Composites the per-window export renders covering `selection` into one
/// physical-first image (the module-header placement model).
///
/// Returns `None` when the selection does not intersect the layout, the
/// canvas rounds to zero pixels, or a cropped output has no render.
#[must_use]
pub fn composite_selection(
    layout: &OutputLayout,
    selection: LogicalRect,
    rendered: &[RenderedOutput],
) -> Option<ExportedImage> {
    let crops = layout.crop_rects(selection);
    if crops.is_empty() {
        return None;
    }
    let mut placements = Vec::with_capacity(crops.len());
    for crop in crops {
        let index = layout
            .outputs
            .iter()
            .position(|output| output.connector == crop.output.connector)?;
        let render = rendered
            .iter()
            .find(|render| render.output_index == index)?;
        let scale = crop.output.scale;
        let dest_x = round((crop.logical.x - selection.x).0 * scale);
        let dest_y = round((crop.logical.y - selection.y).0 * scale);
        placements.push(Placement {
            render,
            src_x: crop.physical.x.0,
            src_y: crop.physical.y.0,
            src_width: crop.physical.width.0,
            src_height: crop.physical.height.0,
            dest_x,
            dest_y,
        });
    }
    let origin_x = placements.iter().map(|p| p.dest_x).min()?;
    let origin_y = placements.iter().map(|p| p.dest_y).min()?;
    let width = placements
        .iter()
        .map(|p| (p.dest_x - origin_x).saturating_add(p.src_width))
        .max()?;
    let height = placements
        .iter()
        .map(|p| (p.dest_y - origin_y).saturating_add(p.src_height))
        .max()?;
    let (canvas_width, canvas_height) =
        (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    if canvas_width == 0 || canvas_height == 0 {
        return None;
    }
    let mut rgba = vec![0u8; canvas_width * canvas_height * 4];
    for placement in &placements {
        placement.blit(&mut rgba, canvas_width, origin_x, origin_y);
    }
    Some(ExportedImage {
        width: u32::try_from(canvas_width).ok()?,
        height: u32::try_from(canvas_height).ok()?,
        rgba,
    })
}

/// One output's physical crop and its destination in the export canvas.
#[derive(Debug)]
struct Placement<'a> {
    render: &'a RenderedOutput,
    src_x: i32,
    src_y: i32,
    src_width: i32,
    src_height: i32,
    dest_x: i32,
    dest_y: i32,
}

impl Placement<'_> {
    fn blit(&self, dest: &mut [u8], dest_width: usize, origin_x: i32, origin_y: i32) {
        let (Ok(src_width), Ok(src_height), Ok(src_x), Ok(src_y)) = (
            usize::try_from(self.src_width),
            usize::try_from(self.src_height),
            usize::try_from(self.src_x),
            usize::try_from(self.src_y),
        ) else {
            return;
        };
        let (Ok(dest_x), Ok(dest_y)) = (
            usize::try_from(self.dest_x - origin_x),
            usize::try_from(self.dest_y - origin_y),
        ) else {
            return;
        };
        let render_width = usize::try_from(self.render.width).unwrap_or(0);
        let len = src_width * 4;
        for row in 0..src_height {
            let dest_offset = ((dest_y + row) * dest_width + dest_x) * 4;
            let src_offset = ((src_y + row) * render_width + src_x) * 4;
            if let (Some(slot), Some(pixels)) = (
                dest.get_mut(dest_offset..dest_offset + len),
                self.render.rgba.get(src_offset..src_offset + len),
            ) {
                slot.copy_from_slice(pixels);
            }
        }
    }
}

/// Rounds half away from zero (the stitch.rs `round_to_i32` convention).
fn round(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let clamped = value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped to the representable i32 range above"
    )]
    let edge = clamped as i32;
    edge
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use flowshot_core::geometry::{
        Logical, LogicalRect, OutputInfo, PhysicalPx, PhysicalSize, Transform,
    };

    use super::*;

    fn output(
        connector: &str,
        logical: LogicalRect,
        physical: (i32, i32),
        scale: f64,
    ) -> OutputInfo {
        OutputInfo::new(
            connector,
            connector,
            logical,
            PhysicalSize::new(PhysicalPx(physical.0), PhysicalPx(physical.1)),
            scale,
            Transform::Normal,
        )
        .unwrap()
    }

    fn region(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
        LogicalRect::new(Logical(x), Logical(y), Logical(width), Logical(height))
    }

    fn render(index: usize, width: u32, height: u32, byte: u8) -> RenderedOutput {
        RenderedOutput {
            output_index: index,
            width,
            height,
            rgba: vec![byte; usize::try_from(width * height * 4).unwrap()],
        }
    }

    #[test]
    fn single_output_crop_lands_at_physical_resolution() {
        // Given one scale-1 output 8x6 and a 4x2 selection at (2,1),
        let layout = OutputLayout::new(vec![output("A", region(0.0, 0.0, 8.0, 6.0), (8, 6), 1.0)]);
        let rendered = [render(0, 8, 6, 7)];
        // When compositing,
        let image = composite_selection(&layout, region(2.0, 1.0, 4.0, 2.0), &rendered).unwrap();
        // Then the export is exactly 4x2 physical pixels.
        assert_eq!((image.width, image.height), (4, 2));
        assert_eq!(image.rgba.len(), 4 * 2 * 4);
        assert!(image.rgba.iter().all(|byte| *byte == 7));
    }

    #[test]
    fn scale_two_output_exports_physical_pixels_not_logical() {
        // Given one scale-2 output: logical 8x6, physical 16x12,
        let layout =
            OutputLayout::new(vec![output("H", region(0.0, 0.0, 8.0, 6.0), (16, 12), 2.0)]);
        let rendered = [render(0, 16, 12, 9)];
        // When compositing a logical 4x2 selection,
        let image = composite_selection(&layout, region(2.0, 1.0, 4.0, 2.0), &rendered).unwrap();
        // Then the export is 8x4 PHYSICAL pixels (physical-first, #4871:
        // never halved, never doubled).
        assert_eq!((image.width, image.height), (8, 4));
    }

    #[test]
    fn spanning_selection_composites_both_monitors_side_by_side() {
        // Given two scale-1 outputs: A red-ish 8x6 at (0,0), B green-ish
        // 8x6 at (8,0),
        let layout = OutputLayout::new(vec![
            output("A", region(0.0, 0.0, 8.0, 6.0), (8, 6), 1.0),
            output("B", region(8.0, 0.0, 8.0, 6.0), (8, 6), 1.0),
        ]);
        let rendered = [render(0, 8, 6, 1), render(1, 8, 6, 2)];
        // When compositing a 6x2 selection spanning the seam at x=8,
        let image = composite_selection(&layout, region(6.0, 2.0, 6.0, 2.0), &rendered).unwrap();
        // Then the canvas is 6x2 with A's pixels left of the seam and B's
        // right (per-monitor crops correct - the flow-12 gate).
        assert_eq!((image.width, image.height), (6, 2));
        // Columns 0-1 of every row come from A (byte 1), columns 2-5 from
        // B (byte 2).
        for row in 0..2usize {
            for column in 0..6usize {
                let offset = (row * 6 + column) * 4;
                let expected = if column < 2 { 1 } else { 2 };
                assert_eq!(
                    &image.rgba[offset..offset + 4],
                    &[expected, expected, expected, expected],
                    "pixel ({column},{row})"
                );
            }
        }
    }

    #[test]
    fn crop_content_is_positionally_exact() {
        // Given a gradient render where every pixel encodes its position,
        let width = 8u32;
        let height = 6u32;
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[
                    u8::try_from(x).unwrap(),
                    u8::try_from(y).unwrap(),
                    0,
                    255,
                ]);
            }
        }
        let layout = OutputLayout::new(vec![output("A", region(0.0, 0.0, 8.0, 6.0), (8, 6), 1.0)]);
        let rendered = [RenderedOutput {
            output_index: 0,
            width,
            height,
            rgba,
        }];
        // When cropping the 3x2 region at (2,1),
        let image = composite_selection(&layout, region(2.0, 1.0, 3.0, 2.0), &rendered).unwrap();
        // Then pixel (0,0) of the export is source (2,1) and (2,1) of the
        // export is source (4,2).
        assert_eq!(&image.rgba[0..4], &[2, 1, 0, 255]);
        let offset = (3 + 2) * 4;
        assert_eq!(&image.rgba[offset..offset + 4], &[4, 2, 0, 255]);
    }

    #[test]
    fn selection_outside_the_layout_composites_nothing() {
        let layout = OutputLayout::new(vec![output("A", region(0.0, 0.0, 8.0, 6.0), (8, 6), 1.0)]);
        let rendered = [render(0, 8, 6, 1)];
        assert!(composite_selection(&layout, region(100.0, 100.0, 4.0, 4.0), &rendered).is_none());
    }

    #[test]
    fn missing_render_for_a_cropped_output_is_none() {
        let layout = OutputLayout::new(vec![
            output("A", region(0.0, 0.0, 8.0, 6.0), (8, 6), 1.0),
            output("B", region(8.0, 0.0, 8.0, 6.0), (8, 6), 1.0),
        ]);
        // Only A rendered; the selection spans into B.
        let rendered = [render(0, 8, 6, 1)];
        assert!(composite_selection(&layout, region(6.0, 2.0, 6.0, 2.0), &rendered).is_none());
    }
}

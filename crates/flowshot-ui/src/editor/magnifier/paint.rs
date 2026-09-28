//! The magnifier's paint half: widget placement with the
//! four-edge flip, the CPU nearest-neighbor zoom buffer, and the
//! display-list commands for both shape variants (square border + arms,
//! circle clip + ring) plus the pixel grid. The readout bar lives in
//! [`super::readout`] (the 250-LOC ceiling split).
//!
//! Geometry is window-local PHYSICAL px (the display-list contract); the
//! sampled window arrives from [`super::sample`] in frame-physical space.

use flowshot_core::geometry::{LogicalPoint, OutputInfo};
use flowshot_core::tokens::DesignTokens;

use super::readout::paint_readout;
use super::{
    ARM_ALPHA, CURSOR_OFFSET, GRID_MIN_ZOOM, MAG_PIXELS, RENDERED_PX, WINDOW_PX, ZOOM,
    sample as sample_window,
};
use crate::editor::{EditorState, MagnifierShape};
use crate::render::{
    Color, DisplayList, Rect, Shape, Size, TextureId, f32_from_f64, f32_from_i32, f32_from_u32,
};

/// The magnifier zoom texture's id in the consumer-issued scheme (atlas =
/// 1, magnifier = `1 << 14`, cursor = `1 << 15`, backdrop = `1 << 16 + i`,
/// effects = `1 << 40 + id`).
const MAGNIFIER_TEXTURE_RAW: u64 = 1 << 14;

/// The circle variant's ring stroke width in physical px (F27: the border
/// pen `setWidth(4)`).
const RING_WIDTH: f32 = 4.0;

/// The pixel-grid line ink: neutral gray at 38% (the grid's `paint_grid`
/// precedent - the palette carries no neutral token yet; the settings
/// theming action is recorded in the notepad).
const GRID_COLOR: Color = Color {
    r: 128.0 / 255.0,
    g: 128.0 / 255.0,
    b: 128.0 / 255.0,
    a: 96.0 / 255.0,
};

/// The magnifier zoom texture id (stable; the shell uploads
/// [`MagnifierTexture`] under it before the frame renders).
#[must_use]
pub const fn magnifier_texture_id() -> TextureId {
    TextureId::new(MAGNIFIER_TEXTURE_RAW)
}

/// The CPU-built zoom window the shell uploads (nearest-neighbor by
/// construction - the renderer's linear image sampler would smear a 10x
/// pixel grid; see the facade header deviation note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnifierTexture {
    /// The texture id to register under.
    pub id: TextureId,
    /// Width in texels (`WINDOW_PX * ZOOM`).
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Row-major `RGBA8888` pixels.
    pub pixels: Vec<u8>,
}

/// The shell-supplied per-window paint input (the
/// [`crate::editor::EditorView`] pattern: everything one window's
/// magnifier paint needs beyond the list).
#[derive(Debug, Clone, Copy)]
pub struct MagnifierView {
    /// The window's surface extent, physical px.
    pub surface: (u32, u32),
    /// Window-local physical cursor position (placement space).
    pub cursor_local: (f64, f64),
    /// Global logical cursor position (sampling space).
    pub cursor_global: LogicalPoint,
}

impl super::MagnifierSample {
    /// The nearest-neighbor zoom buffer: every source pixel becomes a
    /// `ZOOM` x `ZOOM` block (`RENDERED_PX`^2 RGBA bytes).
    #[must_use]
    pub(super) fn zoomed(&self) -> Vec<u8> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "WINDOW_PX * 4 = 68 is a small positive constant"
        )]
        const ROW_BYTES: usize = (WINDOW_PX * 4) as usize;
        let cells = usize::try_from(ZOOM).unwrap_or(10);
        let (rows, _) = self.pixels.as_chunks::<ROW_BYTES>();
        let mut out =
            Vec::with_capacity(usize::try_from((WINDOW_PX * ZOOM).pow(2) * 4).unwrap_or(115_600));
        for row in rows {
            let mut line = Vec::with_capacity(cells * ROW_BYTES);
            let (pixels, _) = row.as_chunks::<4>();
            for pixel in pixels {
                for _ in 0..cells {
                    line.extend_from_slice(pixel);
                }
            }
            for _ in 0..cells {
                out.extend_from_slice(&line);
            }
        }
        out
    }
}

/// The widget rect for a cursor at window-local physical `cursor` on a
/// `surface`: centered `CURSOR_OFFSET + RENDERED/2` px down-right of the
/// cursor, flipping to the opposite side per axis when the widget would
/// leave the surface, then clamped fully on-screen (the corner acceptance:
/// a cursor at an exact corner keeps the whole widget visible).
pub(super) fn placement(cursor: (f64, f64), surface: (u32, u32)) -> Rect {
    let (sw, sh) = (f64::from(surface.0), f64::from(surface.1));
    let half = RENDERED_PX / 2.0;
    let mut center_x = cursor.0 + CURSOR_OFFSET + half;
    if center_x + half > sw {
        center_x = cursor.0 - CURSOR_OFFSET - half;
    }
    let mut center_y = cursor.1 + CURSOR_OFFSET + half;
    if center_y + half > sh {
        center_y = cursor.1 - CURSOR_OFFSET - half;
    }
    let left = (center_x - half).clamp(0.0, (sw - RENDERED_PX).max(0.0));
    let top = (center_y - half).clamp(0.0, (sh - RENDERED_PX).max(0.0));
    let edge = f32_from_f64(RENDERED_PX);
    Rect::from_parts(f32_from_f64(left), f32_from_f64(top), edge, edge)
}

/// Whether the pixel grid draws at `zoom` (plan ADDITION: 1px lines at
/// zoom >= 8).
pub(super) const fn grid_at(zoom: i64) -> bool {
    zoom >= GRID_MIN_ZOOM
}

impl EditorState {
    /// Appends this window's magnifier visuals to `list` (topmost layer -
    /// Flameshot's magnifier is a transparent-for-mouse child widget over
    /// the whole capture surface) and returns the zoom texture for the
    /// shell to upload. `None` (nothing painted) when the magnifier is
    /// hidden, no frame is installed, or the cursor is outside the frame.
    pub fn paint_magnifier(
        &self,
        list: &mut DisplayList,
        output: &OutputInfo,
        view: MagnifierView,
        tokens: &DesignTokens,
    ) -> Option<MagnifierTexture> {
        if !self.magnifier_visible {
            return None;
        }
        let sample = sample_window(self.frame.as_ref(), &self.effects, view.cursor_global)?;
        let rect = placement(view.cursor_local, view.surface);
        tracing::trace!(
            target: "flowshot_ui::editor",
            hex = %sample.center_hex(),
            "magnifier readout"
        );
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let arm = accent.with_alpha8(ARM_ALPHA);
        let zoom = f32_from_i32(i32::try_from(ZOOM).unwrap_or(10));
        match self.magnifier_shape {
            MagnifierShape::Square => {
                // Flameshot drawMagnifier: a border-color rect one px wider
                // than the widget, the zoomed pixmap over it, then the arms.
                list.fill(
                    Shape::Rect {
                        rect: expand(rect, 1.0),
                        radius: 0.0,
                    },
                    accent,
                );
                list.image(magnifier_texture_id(), rect, None);
                if grid_at(ZOOM) {
                    paint_grid_lines(list, rect);
                }
                paint_arms(list, rect, sample.arm_offset, zoom, arm);
            }
            MagnifierShape::Circle => {
                // Flameshot drawMagnifierCircle: elliptic clip over the
                // content, then a 4px ring (the rounded-rect clip at
                // radius = half the widget edge IS the circle).
                list.push_clip(rect, rect.size.width / 2.0);
                list.image(magnifier_texture_id(), rect, None);
                if grid_at(ZOOM) {
                    paint_grid_lines(list, rect);
                }
                paint_arms(list, rect, sample.arm_offset, zoom, arm);
                list.pop_clip();
                let radii = Size::new(rect.size.width / 2.0, rect.size.height / 2.0);
                list.stroke(
                    Shape::Ellipse {
                        center: rect.center(),
                        radii,
                    },
                    RING_WIDTH,
                    accent,
                );
            }
        }
        paint_readout(list, rect, &sample, tokens, output.scale, view.surface);
        let edge = u32::try_from(WINDOW_PX * ZOOM).unwrap_or(170);
        Some(MagnifierTexture {
            id: magnifier_texture_id(),
            width: edge,
            height: edge,
            pixels: sample.zoomed(),
        })
    }
}

fn expand(rect: Rect, by: f32) -> Rect {
    Rect::from_parts(
        rect.origin.x - by,
        rect.origin.y - by,
        rect.size.width + 2.0 * by,
        rect.size.height + 2.0 * by,
    )
}

/// The 1px pixel grid: `WINDOW_PX - 1` interior lines per axis at every
/// `ZOOM`-px cell boundary (the plan's ADDITION at zoom >= 8).
fn paint_grid_lines(list: &mut DisplayList, rect: Rect) {
    let window = u32::try_from(WINDOW_PX).unwrap_or(17);
    let step = rect.size.width / f32_from_u32(window);
    for index in 1..window {
        let offset = f32_from_u32(index) * step;
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(
                    rect.origin.x + offset,
                    rect.origin.y,
                    1.0,
                    rect.size.height,
                ),
                radius: 0.0,
            },
            GRID_COLOR,
        );
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(rect.origin.x, rect.origin.y + offset, rect.size.width, 1.0),
                radius: 0.0,
            },
            GRID_COLOR,
        );
    }
}

/// The four crosshair arms marking the pixel under the cursor (F27
/// `crossHairTop/Right/Bottom/Left` rects, `offset`-shifted so a clamped
/// window keeps the arms on the cursor's true row/column).
fn paint_arms(list: &mut DisplayList, rect: Rect, offset: (i32, i32), zoom: f32, color: Color) {
    let center = rect.center();
    let (ox, oy) = (f32_from_i32(offset.0), f32_from_i32(offset.1));
    let half = zoom / 2.0;
    let mag = f32_from_i32(i32::try_from(MAG_PIXELS).unwrap_or(8));
    // F27 crossHair rects, term by term: the arm band is one zoomed pixel
    // wide, centered on the cursor's window column/row (center + zoom *
    // offset, shifted half a cell onto the pixel), reaching MAG_PIXELS
    // cells to each window edge (shrinking under the edge clamp).
    let arms = [
        // top, right, bottom, left (x, y, width, height)
        (
            center.x + zoom * ox - half,
            center.y - zoom * mag - half,
            zoom,
            zoom * (mag + oy),
        ),
        (
            center.x + half + zoom * ox,
            center.y + zoom * oy - half,
            zoom * (mag - ox),
            zoom,
        ),
        (
            center.x + zoom * ox - half,
            center.y + half + zoom * oy,
            zoom,
            zoom * (mag - oy),
        ),
        (
            center.x - zoom * mag - half,
            center.y + zoom * oy - half,
            zoom * (mag + ox),
            zoom,
        ),
    ];
    for (x, y, width, height) in arms {
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(x, y, width, height),
                radius: 0.0,
            },
            color,
        );
    }
}

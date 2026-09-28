//! The RGB+hex readout bar (the plan's wayshot `--color`-equivalence
//! ADDITION): a contrast-token box with white text attached below the
//! widget, flipping above near the bottom edge (the size-HUD ink
//! precedent). Split from [`super::paint`] at the 250-LOC ceiling.

use flowshot_core::tokens::DesignTokens;

use super::super::paint::{LINE_HEIGHT_RATIO, local_len};
use super::MagnifierSample;
use crate::render::{
    Color, DisplayList, Point, Rect, Shape, TextCommand, f32_from_f64, f32_from_u32,
};

/// The readout box's text-width estimate: mean advance 0.62em (Inter
/// digits/hex fall in 0.55-0.66em; the box pads the estimate, exact
/// metrics need the shaper which the list builder does not run).
const TEXT_ADVANCE_ESTIMATE: f32 = 0.62;

/// The readout box opacity (0-255): the size-HUD box alpha (F27
/// `capturewidget.cpp` paints its geometry box at 200).
const READOUT_BOX_ALPHA: u8 = 200;

/// The readout bar under (or above, near the bottom edge) the widget:
/// contrast-token box + white text (the size-HUD ink precedent), the
/// `#RRGGBB R,G,B` string of the pixel under the crosshair.
pub(super) fn paint_readout(
    list: &mut DisplayList,
    rect: Rect,
    sample: &MagnifierSample,
    tokens: &DesignTokens,
    scale: f64,
    surface: (u32, u32),
) {
    let font_size = local_len(f64::from(tokens.typography.base_size), scale);
    let line = font_size * LINE_HEIGHT_RATIO;
    let pad = local_len(f64::from(tokens.spacing.small), scale);
    let text = sample.readout_text();
    let chars = f32_from_u32(u32::try_from(text.len()).unwrap_or(0));
    let width = (chars * font_size * TEXT_ADVANCE_ESTIMATE + 2.0 * pad).max(rect.size.width);
    let height = line + 2.0 * pad;
    let (sw, sh) = (f32_from_u32(surface.0), f32_from_u32(surface.1));
    let center_x = rect.origin.x + rect.size.width / 2.0;
    let mut x = center_x - width / 2.0;
    x = x.clamp(0.0, (sw - width).max(0.0));
    let gap = pad / 2.0;
    let mut y = rect.origin.y + rect.size.height + gap;
    if y + height > sh {
        y = rect.origin.y - gap - height;
    }
    y = y.clamp(0.0, (sh - height).max(0.0));
    let contrast = Color::from_hex_token(&tokens.palette.contrast)
        .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
    let background = contrast.with_alpha8(READOUT_BOX_ALPHA);
    let radius = f32_from_u32(tokens.radii.small) * f32_from_f64(scale);
    list.fill(
        Shape::Rect {
            rect: Rect::from_parts(x, y, width, height),
            radius,
        },
        background,
    );
    list.text(TextCommand {
        position: Point::new(x + pad, y + (height - line) / 2.0),
        text,
        font_size,
        line_height: line,
        color: background.readable_ink(),
        family: Some(tokens.typography.family.clone()),
        max_width: None,
    });
}

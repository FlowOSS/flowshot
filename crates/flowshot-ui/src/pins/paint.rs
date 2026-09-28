//! Pin frame painting: shadow -> image -> context menu.
//!
//! The drop shadow is the F27 pin shadow (blur `2 * MARGIN`, zero offset,
//! accent color - contrast on hover, Flameshot's `m_baseColor`/
//! `m_hoverColor` swap), faded by the pin opacity exactly like Qt's
//! `setWindowOpacity` fades the whole translucent window. The image quad
//! draws at `margin + offset` with the zoom-scaled extent; sampling is
//! always linear-filtered (pin zoom is ALWAYS antialiased - the
//! `antialiasingPinZoom` toggle is DROPPED per Amendment #3).
//!
//! [`frame_list`] is a PURE function of the state machine: the live shell
//! and the offscreen QA verify path (`examples/pin_window.rs
//! --verify-offscreen`) render the identical list through the identical
//! [`crate::render::Renderer`], so the offscreen readback is ground truth
//! for what the surface stores (the todo-15 verify-offscreen pattern).

use std::time::Instant;

use flowshot_core::tokens::DesignTokens;

use super::shell::PinEntry;
use super::spec::BLUR_RADIUS;
use super::state::PinState;
use crate::gpu::GpuContext;
use crate::render::{Color, DisplayList, Point, Rect, ShadowSpec, f32_from_f64, f32_from_u32};

pub(super) fn render_pin(gpu: &GpuContext, entry: &mut PinEntry) {
    let list = frame_list(&entry.state, Instant::now());
    if let Err(error) = entry
        .surface
        .render(gpu, Some((&mut entry.renderer, &list)), None)
    {
        tracing::error!(%error, pin = entry.id.raw(), "pin frame presentation failed");
    }
}

/// Builds one pin frame: shadow -> image quad -> context menu. `now`
/// evaluates the zoom transition (todo 41): the same instant the shell
/// schedules with, so offscreen renders are deterministic stills.
#[must_use]
pub fn frame_list(state: &PinState, now: Instant) -> DisplayList {
    let scale = f32_from_f64(state.scale_factor());
    let tokens = state.tokens();
    let mut list = DisplayList::new();
    let rect = image_rect(state, now);
    push_shadow(&mut list, tokens, state, rect, scale);
    list.image(super::TEXTURE_ID, rect, None);
    if let Some(menu) = state.menu() {
        menu.draw(&mut list, tokens, scale);
    }
    list
}

/// The image draw rect in window-local physical px: the shadow frame
/// margin plus the zoom-to-cursor offset, sized by the zoom scale - all at
/// the VISUAL (possibly mid-transition) zoom state.
#[must_use]
pub fn image_rect(state: &PinState, now: Instant) -> Rect {
    let margin = f32_from_f64(state.margin_px());
    let (offset_x, offset_y) = state.visual_offset(now);
    let (image_w, image_h) = state.image_size();
    let scale = state.visual_scale(now);
    Rect::from_parts(
        margin + f32_from_f64(offset_x),
        margin + f32_from_f64(offset_y),
        f32_from_u32(image_w) * f32_from_f64(scale),
        f32_from_u32(image_h) * f32_from_f64(scale),
    )
}

fn push_shadow(
    list: &mut DisplayList,
    tokens: &DesignTokens,
    state: &PinState,
    image_rect: Rect,
    scale: f32,
) {
    let hex = if state.hovered() {
        &tokens.palette.contrast
    } else {
        &tokens.palette.accent
    };
    let Some(base) = Color::from_hex_token(hex) else {
        return;
    };
    let spec = ShadowSpec {
        blur: f32_from_f64(BLUR_RADIUS) * scale,
        offset: Point::new(0.0, 0.0),
        color: base.with_alpha(state.opacity()),
    };
    list.shadow(image_rect, 0.0, spec);
}

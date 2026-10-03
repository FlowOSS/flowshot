//! Offscreen consent-dialog render QA (the `settings_offscreen` precedent,
//! no-visible-windows policy): drives the embedded egui surface headlessly
//! through [`flowshot_ui::consent::render_offscreen`] and asserts on the
//! readback pixels.
//!
//! Without a GPU adapter (no Vulkan, not even lavapipe) every test SKIPS
//! with a message - headless CI stays green, this machine runs for real.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::cast_precision_loss)]

use std::collections::HashSet;

use flowshot_ui::consent::{ConsentModel, ConsentWindowOptions, render_offscreen};
use flowshot_ui::gpu::{GpuContext, new_instance};

const WIDTH: u32 = 500;
const HEIGHT: u32 = 370;

struct Gpu {
    _instance: wgpu::Instance,
    ctx: GpuContext,
}

fn gpu_or_skip() -> Option<Gpu> {
    let instance = new_instance();
    match GpuContext::new_headless(&instance) {
        Ok(ctx) => Some(Gpu {
            _instance: instance,
            ctx,
        }),
        Err(error) => {
            eprintln!("SKIP (no usable GPU adapter): {error}");
            None
        }
    }
}

fn render(gpu: &Gpu, model: &mut ConsentModel) -> Vec<u8> {
    render_offscreen(
        &gpu.ctx,
        &ConsentWindowOptions::default(),
        model,
        WIDTH,
        HEIGHT,
        1.0,
    )
    .expect("consent offscreen render")
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = (y * WIDTH + x) as usize * 4;
    [
        pixels[offset],
        pixels[offset + 1],
        pixels[offset + 2],
        pixels[offset + 3],
    ]
}

#[test]
fn consent_dialog_renders_offscreen_with_token_themed_content() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut model = ConsentModel::default();
    let pixels = render(&gpu, &mut model);
    assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize);
    // The dark theme's panel fill = the contrast token (#0F172A).
    let panel = [0x0F_u8, 0x17, 0x2A, 0xFF];
    assert_eq!(pixel(&pixels, 0, 0), panel, "panel fill");
    let distinct: HashSet<[u8; 4]> = pixels.as_chunks::<4>().0.iter().copied().collect();
    assert!(
        distinct.len() > 50,
        "expected rich content, got {} distinct colors",
        distinct.len()
    );
    // The enabled "Save choice" primary button paints with the accent.
    let accent = [0x63_u8, 0x66, 0xF1];
    let accent_pixels = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| {
            p[0].abs_diff(accent[0]) <= 2
                && p[1].abs_diff(accent[1]) <= 2
                && p[2].abs_diff(accent[2]) <= 2
        })
        .count();
    assert!(accent_pixels > 100, "Save choice must paint accent-filled");
}

#[test]
fn checked_checkboxes_change_the_rendered_frame() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    // Given: the three checkbox states the QA renders pin - the default
    // (send pre-checked), both off, and both on
    let mut default = ConsentModel::default();
    let default_pixels = render(&gpu, &mut default);
    let mut off = ConsentModel::default();
    *off.send_mut() = false;
    let off_pixels = render(&gpu, &mut off);
    let mut on = ConsentModel::default();
    *on.details_mut() = true;
    let on_pixels = render(&gpu, &mut on);
    // Then: every state is pixel-distinct - the pre-checked default is
    // VISIBLY different from the opt-out, and checking details changes the
    // frame again
    assert_ne!(
        default_pixels, off_pixels,
        "the pre-checked send box must be visible in the default render"
    );
    assert_ne!(
        off_pixels, on_pixels,
        "the two checkbox states must be pixel-distinct"
    );
}

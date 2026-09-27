//! Offscreen settings-render QA (plan todo 36, no-visible-windows policy):
//! drives the embedded egui surface headlessly through
//! [`flowshot_ui::settings::render_offscreen`] and asserts on the readback
//! pixels - the Ruffle-pattern embed proves it renders WITHOUT a window,
//! display server, or Wayland connection.
//!
//! Without a GPU adapter (no Vulkan, not even lavapipe) every test SKIPS
//! with a message - headless CI stays green, this machine runs for real
//! (the tests/parity.rs precedent).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::collections::HashSet;
use std::sync::Once;

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::gpu::{GpuContext, OVERLAY_BACKENDS};
use flowshot_ui::settings::{SettingsModel, Tab, ThemeMode, render_offscreen};

struct Gpu {
    _instance: wgpu::Instance,
    ctx: GpuContext,
}

static TRACING: Once = Once::new();

fn init_tracing() {
    TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .init();
    });
}

fn gpu_or_skip() -> Option<Gpu> {
    init_tracing();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: OVERLAY_BACKENDS,
        ..wgpu::InstanceDescriptor::default()
    });
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

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;

fn render_tab(gpu: &Gpu, tab: Tab, model: &mut SettingsModel) -> Vec<u8> {
    model.set_active_tab(tab);
    render_offscreen(
        &gpu.ctx,
        &DesignTokens::default(),
        model,
        ThemeMode::Dark,
        WIDTH,
        HEIGHT,
        1.0,
    )
    .unwrap_or_else(|error| panic!("{tab:?} render failed: {error}"))
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
fn every_tab_renders_offscreen_with_token_themed_content() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut model = SettingsModel::default();
    // The dark theme's panel fill = the contrast token (#0F172A).
    let panel = [0x0F_u8, 0x17, 0x2A, 0xFF];
    for tab in Tab::ALL {
        let pixels = render_tab(&gpu, tab, &mut model);
        assert_eq!(
            pixels.len(),
            (WIDTH * HEIGHT * 4) as usize,
            "{tab:?}: full readback"
        );
        // The clear color lands in sRGB byte space (corner = untouched panel).
        assert_eq!(pixel(&pixels, 0, 0), panel, "{tab:?}: panel fill");
        assert_eq!(
            pixel(&pixels, WIDTH - 1, HEIGHT - 1),
            panel,
            "{tab:?}: panel fill (opposite corner)"
        );
        // Widgets + text rendered: the frame is not a uniform clear.
        let distinct: HashSet<[u8; 4]> = pixels.as_chunks::<4>().0.iter().copied().collect();
        assert!(
            distinct.len() > 50,
            "{tab:?}: expected rich content, got {} distinct colors",
            distinct.len()
        );
    }
}

#[test]
fn accent_token_is_present_in_the_rendered_frame() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut model = SettingsModel::default();
    // The selected tab button paints with the accent token (#6366F1).
    let accent = [0x63_u8, 0x66, 0xF1];
    let pixels = render_tab(&gpu, Tab::General, &mut model);
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
    assert!(
        accent_pixels > 0,
        "the accent token must paint the selected tab"
    );
}

#[test]
fn light_theme_renders_a_different_frame_than_dark() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut dark_model = SettingsModel::default();
    let dark = render_offscreen(
        &gpu.ctx,
        &DesignTokens::default(),
        &mut dark_model,
        ThemeMode::Dark,
        WIDTH,
        HEIGHT,
        1.0,
    )
    .unwrap();
    let mut light_model = SettingsModel::default();
    let light = render_offscreen(
        &gpu.ctx,
        &DesignTokens::default(),
        &mut light_model,
        ThemeMode::Light,
        WIDTH,
        HEIGHT,
        1.0,
    )
    .unwrap();
    assert_ne!(pixel(&dark, 0, 0), pixel(&light, 0, 0));
    // Light mode's panel is NOT the dark contrast token.
    assert_ne!(pixel(&light, 0, 0), [0x0F, 0x17, 0x2A, 0xFF]);
}

#[test]
fn edited_accent_changes_the_rendered_theme() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut model = SettingsModel::default();
    model.config_mut().ui.accent_color = "#FF0000".to_owned();
    let pixels = render_tab(&gpu, Tab::General, &mut model);
    let red_pixels = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 200 && p[1] < 60 && p[2] < 60)
        .count();
    assert!(
        red_pixels > 0,
        "the edited accent must paint live (tokens-driven live-apply)"
    );
}

#[test]
fn oversized_offscreen_target_is_a_typed_error() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut model = SettingsModel::default();
    let limit = gpu.ctx.device.limits().max_texture_dimension_2d;
    let result = render_offscreen(
        &gpu.ctx,
        &DesignTokens::default(),
        &mut model,
        ThemeMode::Dark,
        limit + 1,
        100,
        1.0,
    );
    assert!(
        matches!(&result, Err(flowshot_ui::UiError::TextureTooLarge { .. })),
        "expected TextureTooLarge, got {result:?}"
    );
}

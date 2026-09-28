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
    clippy::float_cmp,
    clippy::cast_sign_loss,
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

// --- the settings-rework grid: card surfaces, column alignment, action bar --

use flowshot_ui::settings::{FormMetrics, surfaces_for};

fn expected_card(model: &SettingsModel) -> [u8; 4] {
    let surfaces = surfaces_for(
        &DesignTokens::default(),
        &model.config().ui,
        ThemeMode::Dark,
    );
    let [r, g, b, _] = surfaces.card.to_array();
    [r, g, b, 0xFF]
}

#[test]
fn section_cards_render_on_the_derived_token_surface() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut model = SettingsModel::default();
    let pixels = render_tab(&gpu, Tab::General, &mut model);
    let card = expected_card(&model);
    let panel = [0x0F_u8, 0x17, 0x2A, 0xFF];
    // The card's left padding band carries the exact derived card surface
    // (window margin 16 + card padding 16 => x=20 is inside the padding).
    assert_eq!(pixel(&pixels, 20, 70), card, "card fill (title band)");
    assert_eq!(pixel(&pixels, 20, 300), card, "card fill (body band)");
    // The window margin band stays the panel fill (card != panel proves the
    // raised surface exists at all).
    assert_eq!(pixel(&pixels, 8, 300), panel, "window margin band");
    assert_ne!(card, panel, "cards must lift off the window surface");
}

#[test]
fn controls_share_one_left_edge_and_the_gutter_stays_clean() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut model = SettingsModel::default();
    let pixels = render_tab(&gpu, Tab::General, &mut model);
    let card = expected_card(&model);
    let m = FormMetrics::from_tokens(&DesignTokens::default());
    // At the QA width the label column is at its em cap, so the control x
    // is exact: content_left(32) + label(308) + gutter(8) = 348.
    let control_x = m.control_x(WIDTH as f32 - 100.0);
    assert_eq!(control_x, 348.0);

    // Controls (checkboxes, fields, rails, combos) all START at the control
    // column: scanning 2px inside it crosses several distinct widget bands.
    let x = control_x as u32 + 2;
    let mut runs = 0_u32;
    let mut in_run = false;
    for y in 90..580 {
        let hit = pixel(&pixels, x, y) != card;
        if hit && !in_run {
            runs += 1;
        }
        in_run = hit;
    }
    assert!(
        runs >= 3,
        "expected stacked controls at x={x}, got {runs} runs"
    );

    // Nothing crosses the gutter: the column between label edge and control
    // edge stays card fill (title separators excepted - a handful of rows).
    let gx = control_x as u32 - (m.gutter() as u32 / 2);
    let total = (90..580).count();
    let clean = (90..580).filter(|y| pixel(&pixels, gx, *y) == card).count();
    assert!(
        clean * 20 >= total * 19,
        "gutter column x={gx} must stay clean: {clean}/{total}"
    );
}

#[test]
fn action_bar_stays_visible_while_cards_scroll() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let mut model = SettingsModel::default();
    model.mark_dirty(); // Apply enabled => accent-filled primary button
    let pixels = render_tab(&gpu, Tab::General, &mut model);
    // The General tab's content overflows the 640px window, yet the Apply
    // button must render in the docked bottom bar (the pre-rework layout
    // pushed the bar off-window whenever content overflowed).
    let accent = [0x63_u8, 0x66, 0xF1];
    let count = (560..HEIGHT)
        .flat_map(|y| (0..300).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let p = pixel(&pixels, *x, *y);
            p[0].abs_diff(accent[0]) <= 2
                && p[1].abs_diff(accent[1]) <= 2
                && p[2].abs_diff(accent[2]) <= 2
        })
        .count();
    assert!(
        count > 200,
        "Apply must paint in the bottom band, got {count} accent px"
    );
}

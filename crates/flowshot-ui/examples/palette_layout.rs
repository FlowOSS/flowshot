//! Palette swatch-grid layout evidence (THIRD pass). The two prior passes
//! relied on `egui::horizontal_wrapped` inferring a wrap width from the card's
//! layout chain; it never did, so the 20 default swatches escaped the card's
//! right edge in one line. This harness renders the Interface tab with the
//! REAL default palette at one wide and two narrow widths, asserts on the
//! readback pixels that every swatch stays inside the card's control column
//! (no right-edge overflow) and wraps into multiple rows, and writes the PNGs
//! plus this assertion log into the evidence dir.
//!
//! Usage (from the workspace root):
//!
//! ```text
//! cargo run -p flowshot-ui --example palette_layout -- .omo/evidence/palette-layout-fix
//! ```
//!
//! Exits non-zero without a GPU adapter or if any assertion fails.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::path::PathBuf;
use std::process::ExitCode;

use flowshot_core::config::{Config, UiConfig};
use flowshot_core::tokens::DesignTokens;
use flowshot_ui::gpu::{GpuContext, new_instance};
use flowshot_ui::settings::{
    FormMetrics, SettingsModel, Tab, ThemeMode, fonts, render_offscreen, settings_style,
};

const HEIGHT: u32 = 1600;
const WIDTHS: [u32; 3] = [1280, 700, 460];
/// The reserved solid scrollbar's width (`settings_style`: small * 2).
const BAR_WIDTH: f32 = 8.0;
/// The remove / add button literals (mirror `settings::strings`, which is
/// crate-private); only used to measure cell geometry for the log.
const REMOVE_LABEL: &str = "\u{00D7}";
const ADD_LABEL: &str = "Add swatch";

/// Saturated editor-palette hues that appear nowhere else in the Interface tab
/// (the accent #6366F1, the contrast #0F172A, and the neutral widget surfaces
/// are all distinct), so a readback pixel matching one is definitely a palette
/// swatch - the signal the containment / wrap asserts key on.
const DISTINCTIVE: [[u8; 3]; 8] = [
    [0xED, 0x1C, 0x24],
    [0xFF, 0x7F, 0x27],
    [0xFF, 0xF2, 0x00],
    [0x22, 0xB1, 0x4C],
    [0x00, 0xA2, 0xE8],
    [0xA3, 0x49, 0xA4],
    [0xB5, 0xE6, 0x1D],
    [0xFF, 0xAE, 0xC9],
];

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error")),
        )
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowshot: palette layout evidence failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let dir = std::env::args().nth(1).map_or_else(
        || PathBuf::from(".omo/evidence/palette-layout-fix"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;

    let instance = new_instance();
    let gpu = GpuContext::new_headless(&instance).map_err(|error| error.to_string())?;
    let tokens = DesignTokens::default();
    let m = FormMetrics::from_tokens(&tokens);
    let (cell_w, gap, add_w) = measure_metrics(&tokens, &m);
    let swatches = Config::default().editor.color_palette.len();

    println!("=== palette swatch-grid layout evidence (third pass) ===");
    println!("default palette swatches = {swatches}");
    println!("measured cell_w = {cell_w:.2}  gap = {gap:.2}  add_w = {add_w:.2}");

    let mut all_pass = true;
    for width in WIDTHS {
        let mut model = SettingsModel::default();
        model.set_active_tab(Tab::Interface);
        let pixels = render_offscreen(
            &gpu,
            &tokens,
            &mut model,
            ThemeMode::Dark,
            width,
            HEIGHT,
            1.0,
        )
        .map_err(|error| error.to_string())?;

        let image = image::RgbaImage::from_vec(width, HEIGHT, pixels.clone())
            .ok_or_else(|| "readback size mismatch".to_owned())?;
        let path = dir.join(format!("palette-interface-{width}.png"));
        image
            .save(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;

        // The control column's right edge == the card's inner right edge; use
        // the no-scrollbar (widest) value so the containment band starts just
        // outside the largest card the column could occupy.
        let scroll_wide = (width as f32) - 2.0 * m.window_margin();
        let scroll_bar = scroll_wide - BAR_WIDTH;
        let card_inner_right =
            m.window_margin() + m.content_offset(scroll_wide) + m.content_width(scroll_wide)
                - m.card_padding();
        let control_x = m.control_x(scroll_bar);
        let inner = m.content_width(scroll_bar) - 2.0 * m.card_padding();
        let control_w = (inner - m.label_width(inner) - m.gutter()).max(0.0);
        let cells_per_row = ((((control_w + gap) / (cell_w + gap)).floor()) as usize).max(1);
        let expected_rows = swatches.div_ceil(cells_per_row);

        let frame = Frame {
            pixels,
            width,
            height: HEIGHT,
        };
        let scan = frame.scan(control_x, card_inner_right);
        let row_pitch = m.row_pitch();
        let extent = scan.max_y.saturating_sub(scan.min_y);

        let containment = scan.overflow == 0;
        let presence = scan.contained > 0;
        let wrap = scan.bands >= 2 && (extent as f32) > 3.0 * row_pitch;
        let pass = containment && presence && wrap;
        all_pass &= pass;

        println!("--- width {width} (x{HEIGHT}) -> {} ---", path.display());
        println!(
            "  control_w = {control_w:.1}  cells_per_row = {cells_per_row}  \
             expected_swatch_rows = {expected_rows}"
        );
        println!(
            "  control_x = {control_x:.1}  card_inner_right = {card_inner_right:.1}  \
             row_pitch = {row_pitch:.1}"
        );
        println!(
            "  containment: {} (overflow px past card_inner_right = {})",
            verdict(containment),
            scan.overflow
        );
        println!(
            "  presence:    {} (distinctive swatch px in column = {})",
            verdict(presence),
            scan.contained
        );
        println!(
            "  wrap:        {} (row bands = {}, vertical extent = {extent}px > {:.0}px)",
            verdict(wrap),
            scan.bands,
            3.0 * row_pitch
        );
        println!("  => {}", verdict(pass));
    }

    let wide = cells_per_row_at(1280.0, &m, cell_w, gap);
    let narrow = cells_per_row_at(460.0, &m, cell_w, gap);
    let shrink = narrow < wide;
    all_pass &= shrink;
    println!("--- cells_per_row shrink ---");
    println!(
        "  1280 -> {wide} up,  460 -> {narrow} up  shrink: {}",
        verdict(shrink)
    );

    println!("=== {} ===", if all_pass { "ALL PASS" } else { "FAILURES" });
    if all_pass {
        Ok(())
    } else {
        Err("one or more palette layout assertions failed".to_owned())
    }
}

fn verdict(pass: bool) -> &'static str {
    if pass { "PASS" } else { "FAIL" }
}

/// The palette cell geometry, measured headlessly the same way
/// `palette_editor` measures it live: the swatch is egui's color button
/// (exactly `interact_size.x`), the remove/add buttons are galley +
/// 2*`button_padding` (egui's button frame inner margin).
fn measure_metrics(tokens: &DesignTokens, m: &FormMetrics) -> (f32, f32, f32) {
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts());
    let style = settings_style(tokens, &UiConfig::default(), ThemeMode::Dark);
    let (mut gap, mut swatch_w, mut padding) = (0.0_f32, 0.0_f32, 0.0_f32);
    ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.set_style(style.clone());
        gap = ui.spacing().item_spacing.x;
        swatch_w = ui.spacing().interact_size.x;
        padding = ui.spacing().button_padding.x;
    })
    .drop_without_applying_deltas();
    let font = egui::FontId::proportional(m.base_size());
    let galley_w = |text: &str| {
        ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(text.into(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        })
    };
    let cell_w = swatch_w + gap + galley_w(REMOVE_LABEL) + 2.0 * padding;
    let add_w = galley_w(ADD_LABEL) + 2.0 * padding;
    (cell_w, gap, add_w)
}

fn cells_per_row_at(win: f32, m: &FormMetrics, cell_w: f32, gap: f32) -> usize {
    let scroll = win - 2.0 * m.window_margin() - BAR_WIDTH;
    let inner = m.content_width(scroll) - 2.0 * m.card_padding();
    let control_w = (inner - m.label_width(inner) - m.gutter()).max(0.0);
    (((control_w + gap) / (cell_w + gap)).floor() as usize).max(1)
}

struct Scan {
    /// Distinctive-color pixels to the RIGHT of the card's inner right edge
    /// (the overflow the fix must eliminate).
    overflow: usize,
    /// Distinctive-color pixels inside the control column (the swatches).
    contained: usize,
    /// Distinct vertical runs of swatch pixels in the column (one per row).
    bands: usize,
    min_y: u32,
    max_y: u32,
}

struct Frame {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl Frame {
    fn scan(&self, control_x: f32, card_inner_right: f32) -> Scan {
        let overflow_start = card_inner_right.ceil() as u32 + 1;
        let mut overflow = 0;
        for y in 0..self.height {
            for x in overflow_start..self.width {
                if self.is_swatch(x, y) {
                    overflow += 1;
                }
            }
        }

        let x0 = control_x.floor().max(0.0) as u32;
        let x1 = (card_inner_right.ceil() as u32).min(self.width.saturating_sub(1));
        let mut contained = 0;
        let mut min_y = u32::MAX;
        let mut max_y = 0;
        let mut row_has = vec![false; self.height as usize];
        for y in 0..self.height {
            for x in x0..=x1 {
                if self.is_swatch(x, y) {
                    contained += 1;
                    row_has[y as usize] = true;
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        let mut bands = 0;
        let mut in_band = false;
        for hit in &row_has {
            if *hit {
                if !in_band {
                    bands += 1;
                    in_band = true;
                }
            } else {
                in_band = false;
            }
        }
        Scan {
            overflow,
            contained,
            bands,
            min_y,
            max_y,
        }
    }

    fn is_swatch(&self, x: u32, y: u32) -> bool {
        let i = ((y * self.width + x) * 4) as usize;
        let p = [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]];
        DISTINCTIVE.iter().any(|d| {
            p[0].abs_diff(d[0]) <= 2 && p[1].abs_diff(d[1]) <= 2 && p[2].abs_diff(d[2]) <= 2
        })
    }
}

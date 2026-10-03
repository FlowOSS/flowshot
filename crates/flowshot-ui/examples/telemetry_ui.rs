//! Offscreen telemetry-UI evidence dump (no-visible-windows QA policy):
//! renders the first-launch consent dialog in each checkbox state and the
//! settings General tab's Telemetry card headlessly through the shared
//! offscreen paths, and writes PNGs.
//!
//! Usage (from the workspace root):
//!
//! ```text
//! cargo run -p flowshot-ui --example telemetry_ui -- .omo/evidence/telemetry-ui
//! ```
//!
//! Writes `consent-{default,off,both}.png` (the dialog at its fixed
//! 500x370 logical size; "default" = the shipped pre-check: telemetry
//! checked, details unchecked) and `settings-general-telemetry{,-on}.png`
//! (the full General tab at 1280x2800 so the bottom Telemetry card is
//! visible without scrolling). Exits non-zero without a GPU adapter.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::consent::render_offscreen as consent_offscreen;
use flowshot_ui::consent::{ConsentModel, ConsentWindowOptions};
use flowshot_ui::gpu::{GpuContext, new_instance};
use flowshot_ui::settings::render_offscreen as settings_offscreen;
use flowshot_ui::settings::{SettingsModel, Tab, ThemeMode};

const DIALOG_WIDTH: u32 = 500;
const DIALOG_HEIGHT: u32 = 370;
const SETTINGS_WIDTH: u32 = 1280;
const SETTINGS_HEIGHT: u32 = 2800;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowshot: telemetry-ui offscreen dump failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let dir = std::env::args().nth(1).map_or_else(
        || PathBuf::from("/tmp/flowshot-telemetry-ui"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;

    let instance = new_instance();
    let gpu = GpuContext::new_headless(&instance).map_err(|error| error.to_string())?;

    let options = ConsentWindowOptions::default();
    for (name, send, details) in [
        ("default", true, false),
        ("off", false, false),
        ("both", true, true),
    ] {
        let mut model = ConsentModel::default();
        *model.send_mut() = send;
        *model.details_mut() = details;
        let pixels =
            consent_offscreen(&gpu, &options, &mut model, DIALOG_WIDTH, DIALOG_HEIGHT, 1.0)
                .map_err(|error| error.to_string())?;
        write_png(
            &dir,
            &format!("consent-{name}.png"),
            DIALOG_WIDTH,
            DIALOG_HEIGHT,
            pixels,
        )?;
    }

    for (name, opted_in) in [("telemetry", false), ("telemetry-on", true)] {
        let mut model = SettingsModel::default();
        model.set_active_tab(Tab::General);
        if opted_in {
            model.config_mut().telemetry.enabled = true;
            model.config_mut().telemetry.include_technical_details = true;
            model.mark_dirty();
        }
        let pixels = settings_offscreen(
            &gpu,
            &DesignTokens::default(),
            &mut model,
            ThemeMode::Dark,
            SETTINGS_WIDTH,
            SETTINGS_HEIGHT,
            1.0,
        )
        .map_err(|error| error.to_string())?;
        write_png(
            &dir,
            &format!("settings-general-{name}.png"),
            SETTINGS_WIDTH,
            SETTINGS_HEIGHT,
            pixels,
        )?;
    }
    Ok(())
}

fn write_png(
    dir: &Path,
    name: &str,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
) -> Result<(), String> {
    let image = image::RgbaImage::from_vec(width, height, pixels)
        .ok_or_else(|| "readback size mismatch".to_owned())?;
    let path = dir.join(name);
    image
        .save(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    println!("wrote {} ({width}x{height})", path.display());
    Ok(())
}

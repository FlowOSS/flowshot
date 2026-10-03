//! Offscreen settings-render evidence dump (plan todo 36 QA, no-visible-
//! windows policy): renders each of the four tabs headlessly through
//! [`flowshot_ui::settings::render_offscreen`] and writes PNGs.
//!
//! Usage (from the workspace root):
//!
//! ```text
//! cargo run -p flowshot-ui --example settings_offscreen -- /tmp/shots
//! ```
//!
//! Writes `<dir>/settings-{general,interface,filename,shortcuts}.png` and
//! prints per-tab pixel stats. Exits non-zero without a GPU adapter.

use std::path::PathBuf;
use std::process::ExitCode;

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::gpu::{GpuContext, new_instance};
use flowshot_ui::settings::{SettingsModel, Tab, ThemeMode, render_offscreen};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

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
            eprintln!("flowshot: settings offscreen dump failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let dir = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("/tmp/flowshot-settings"), PathBuf::from);
    std::fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;

    let instance = new_instance();
    let gpu = GpuContext::new_headless(&instance).map_err(|error| error.to_string())?;

    for tab in Tab::ALL {
        let mut model = SettingsModel::default();
        model.set_active_tab(tab);
        // Seed a couple of realistic edits so the dump shows populated state.
        model.config_mut().upload.client_id = String::from("flowshot-demo-client");
        model.config_mut().daemon.tray = true;
        model.mark_dirty();
        let pixels = render_offscreen(
            &gpu,
            &DesignTokens::default(),
            &mut model,
            ThemeMode::Dark,
            WIDTH,
            HEIGHT,
            1.0,
        )
        .map_err(|error| error.to_string())?;
        let image = image::RgbaImage::from_vec(WIDTH, HEIGHT, pixels)
            .ok_or_else(|| "readback size mismatch".to_owned())?;
        let name = format!("settings-{}.png", tab_name(tab));
        let path = dir.join(&name);
        image
            .save(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        println!("wrote {} ({WIDTH}x{HEIGHT})", path.display());
    }
    Ok(())
}

fn tab_name(tab: Tab) -> &'static str {
    match tab {
        Tab::General => "general",
        Tab::Interface => "interface",
        Tab::Filename => "filename",
        Tab::Shortcuts => "shortcuts",
    }
}

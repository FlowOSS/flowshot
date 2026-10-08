//! Prints the layered cursor-position resolution as JSON.
//!
//! Walks [`resolve_cursor_pos`]: the ICC pointer-cursor session one-shot
//! (layer 1), the raw Hyprland IPC socket (layer 2 - no external process is
//! spawned, `hyprctl` is never called), and the overlay-first-motion handoff
//! (layer 3). The tracing log on stderr names the layer that answered
//! (`RUST_LOG=flowshot_capture_wayland=debug`); stdout is a single JSON
//! object for oracle comparison against `hyprctl cursorpos`.
//!
//! Usage:
//! - `resolve_cursor` - the full ladder (layer 1 answers on an ICC-capable
//!   compositor such as this machine's Hyprland).
//! - `resolve_cursor --no-icc` - masks layer 1, exercising the Hyprland IPC
//!   socket layer on this session.
//! - `resolve_cursor --pretty` - indents the JSON.
//!
//! Degradation, never failure: with every layer masked (e.g.
//! `env -u HYPRLAND_INSTANCE_SIGNATURE resolve_cursor --no-icc`) this prints
//! the `overlay-first-motion` layer with null coordinates and STILL exits 0 -
//! no panic, the contract todos 16/18 rely on.

use std::process::ExitCode;

use flowshot_capture_wayland::desktop::detect_desktop_env;
use flowshot_capture_wayland::{IccBackend, resolve_cursor_pos};
use serde_json::json;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pretty = args.iter().any(|arg| arg == "--pretty");
    let icc = (!args.iter().any(|arg| arg == "--no-icc")).then(IccBackend::new);
    let desktop = detect_desktop_env();
    let source = futures::executor::block_on(resolve_cursor_pos(icc));
    let value = json!({
        "desktop": desktop,
        "layer": source.layer_name(),
        "x": source.position().map(|(x, _)| x),
        "y": source.position().map(|(_, y)| y),
    });
    let rendered = if pretty {
        serde_json::to_string_pretty(&value)
    } else {
        serde_json::to_string(&value)
    };
    match rendered {
        Ok(text) => println!("{text}"),
        Err(err) => eprintln!("warning: could not render JSON: {err}"),
    }
    ExitCode::SUCCESS
}

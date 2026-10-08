//! Live QA harness for the capture launcher dialog (plan todo 37).
//!
//! Spawns the real [`flowshot_ui::launcher::LauncherWindow`] on the live
//! session with the production seams wired the way the todo-38 binary layer
//! will wire them:
//!
//! - [`MonitorProbe`] -> the live todo-6 output probe (`CaptureThread`),
//! - [`LaunchCallback`] -> executes the typed request against the session
//!   (ICC `capture_region` / `capture_output_named`), honors the delay, and
//!   writes the PNG for the acceptance asserts (`file` dims == grim oracle),
//! - `WindowCustomizer` -> `app_id=flowshot-launcher` (`hyprctl clients`).
//!
//! Usage (self-terminating via the stdin injector, user-away QA rules):
//!
//! ```text
//! cargo run -p flowshot-ui --features test-drive --example launcher_dialog -- \
//!     --save /tmp/flowshot-launcher-capture.png
//! ```
//!
//! With `--features test-drive`, stdin lines inject synthetic events through
//! the production input path (the pins/overlay injector protocol):
//! `text <string>`, `key <Tab|Enter|Escape|Space|ArrowDown|ArrowUp>`,
//! `click <x> <y>` (window-local physical px, press+release), `exit`.
//!
//! [`MonitorProbe`]: flowshot_ui::launcher::MonitorProbe
//! [`LaunchCallback`]: flowshot_ui::launcher::LaunchCallback

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use flowshot_capture::{CaptureBackend, CaptureOpts, Frame};
use flowshot_capture_wayland::stitch::to_rgba;
use flowshot_capture_wayland::{CaptureThread, IccBackend, resolve_cursor_pos};
use flowshot_core::geometry::LogicalRect;
use flowshot_ui::launcher::{
    LaunchCallback, LauncherRequest, LauncherWindow, LauncherWindowOptions, MonitorProbe,
    RegionGeometry,
};

mod common;

use common::session_window_customizer;

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
            eprintln!("flowshot: launcher dialog harness failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let save = parse_save_path();
    let thread = CaptureThread::spawn().map_err(|error| format!("output probe spawn: {error}"))?;
    let save_target = save.clone();
    let options = LauncherWindowOptions {
        monitor_probe: Some(MonitorProbe::new(move || {
            thread.outputs().unwrap_or_else(|error| {
                eprintln!("probe failed: {error}");
                Vec::new()
            })
        })),
        on_capture: Some(LaunchCallback::new(move |request| {
            println!("LAUNCHER REQUEST: {request:?}");
            match execute(request, &save_target) {
                Ok(()) => println!("CAPTURE EXECUTED"),
                Err(error) => eprintln!("CAPTURE FAILED: {error}"),
            }
        })),
        window_customizer: Some(session_window_customizer(
            "flowshot-launcher",
            "Capture Launcher",
        )),
        ..LauncherWindowOptions::default()
    };
    let window =
        LauncherWindow::new(options).map_err(|error| format!("launcher window: {error}"))?;
    println!("LAUNCHER WINDOW CREATED");
    #[cfg(feature = "test-drive")]
    spawn_stdin_injector(window.handle().clone());
    window
        .run()
        .map_err(|error| format!("launcher event loop: {error}"))?;
    println!("LAUNCHER WINDOW EXITED save={}", save.display());
    Ok(())
}

fn parse_save_path() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--save"
            && let Some(path) = args.next()
        {
            return PathBuf::from(path);
        }
    }
    PathBuf::from("/tmp/flowshot-launcher-capture.png")
}

/// Executes one typed request against the live session (the todo-38 binary
/// layer's job, done here so the acceptance chain is observable today).
fn execute(request: &LauncherRequest, save: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let delay_ms = match request {
        LauncherRequest::Region { delay_ms, .. } | LauncherRequest::Screen { delay_ms, .. } => {
            *delay_ms
        }
    };
    if delay_ms > 0 {
        println!("DELAY {delay_ms} ms");
        std::thread::sleep(std::time::Duration::from_millis(u64::from(delay_ms)));
    }
    let backend = IccBackend::new();
    let frame = futures::executor::block_on(capture(request, &backend))?;
    write_png(save, &frame)?;
    println!(
        "SAVED {} {}x{}",
        save.display(),
        frame.buffer.width,
        frame.buffer.height
    );
    Ok(())
}

async fn capture(
    request: &LauncherRequest,
    backend: &IccBackend,
) -> Result<Frame, Box<dyn std::error::Error>> {
    match request {
        LauncherRequest::Region { geometry, .. } => {
            let region = region_rect(geometry).await?;
            println!("CAPTURE REGION {region:?}");
            let frame = backend.capture_region(region).await?;
            Ok(frame)
        }
        LauncherRequest::Screen { screen, .. } => {
            let outputs = backend.outputs().await?;
            let index = usize::try_from(*screen)?;
            let output = outputs
                .get(index)
                .ok_or("screen index outside the live probe")?;
            println!("CAPTURE SCREEN {screen} ({})", output.connector);
            let frame = backend
                .capture_output_named(&output.connector, CaptureOpts::new(false))
                .await?;
            Ok(frame)
        }
    }
}

/// The typed geometry as a capture rect; offset-less `WxH` centers at the
/// cursor (the todo-18 executor semantics, resolved through the F13 ladder).
async fn region_rect(geometry: &RegionGeometry) -> Result<LogicalRect, Box<dyn std::error::Error>> {
    let (width, height) = (f64::from(geometry.width), f64::from(geometry.height));
    let (x, y) = if let (Some(offset_x), Some(offset_y)) = (geometry.x, geometry.y) {
        (f64::from(offset_x), f64::from(offset_y))
    } else {
        let source = resolve_cursor_pos(Some(IccBackend::new())).await;
        let (cursor_x, cursor_y) = source
            .position()
            .ok_or("offset-less geometry needs a cursor position; the ladder resolved none")?;
        println!(
            "CURSOR-CENTERED via {} at {cursor_x},{cursor_y}",
            source.layer_name()
        );
        (
            f64::from(cursor_x) - width / 2.0,
            f64::from(cursor_y) - height / 2.0,
        )
    };
    Ok(LogicalRect::from_raw(x, y, width, height))
}

fn write_png(path: &Path, frame: &Frame) -> Result<(), Box<dyn std::error::Error>> {
    let mut rgba = Vec::with_capacity(frame.buffer.data.len());
    for y in 0..frame.buffer.height {
        for x in 0..frame.buffer.width {
            let pixel = frame
                .buffer
                .pixel(x, y)
                .ok_or("frame buffer is shorter than its geometry")?;
            rgba.extend_from_slice(&to_rgba(frame.buffer.format, pixel));
        }
    }
    let image = image::RgbaImage::from_raw(frame.buffer.width, frame.buffer.height, rgba)
        .ok_or("frame dimensions disagree with its pixel data")?;
    image.save(path)?;
    Ok(())
}

#[cfg(feature = "test-drive")]
fn spawn_stdin_injector(handle: flowshot_ui::launcher::LauncherHandle) {
    use flowshot_ui::launcher::LauncherInput;
    use std::io::BufRead;
    use winit::keyboard::NamedKey;

    std::thread::spawn(move || {
        eprintln!("INJECTOR ONLINE");
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            eprintln!("INJECT LINE: {line}");
            let Some((command, rest)) = line.split_once(' ') else {
                if line == "exit" {
                    let _ = handle.request_exit();
                    break;
                }
                continue;
            };
            let events: Vec<LauncherInput> = match command {
                "text" => vec![LauncherInput::Text(rest.to_owned())],
                "key" => match rest.trim() {
                    "Tab" => vec![LauncherInput::Key(NamedKey::Tab)],
                    "Enter" => vec![LauncherInput::Key(NamedKey::Enter)],
                    "Escape" => vec![LauncherInput::Key(NamedKey::Escape)],
                    "Space" => vec![LauncherInput::Key(NamedKey::Space)],
                    "ArrowDown" => vec![LauncherInput::Key(NamedKey::ArrowDown)],
                    "ArrowUp" => vec![LauncherInput::Key(NamedKey::ArrowUp)],
                    other => {
                        eprintln!("unknown key {other}");
                        continue;
                    }
                },
                "click" => {
                    let Some((x, y)) = rest.split_once(' ') else {
                        continue;
                    };
                    let (Ok(x), Ok(y)) = (x.trim().parse::<f64>(), y.trim().parse::<f64>()) else {
                        continue;
                    };
                    vec![
                        LauncherInput::Pointer {
                            x,
                            y,
                            pressed: true,
                        },
                        LauncherInput::Pointer {
                            x,
                            y,
                            pressed: false,
                        },
                    ]
                }
                "exit" => {
                    let _ = handle.request_exit();
                    break;
                }
                other => {
                    eprintln!("unknown command {other}");
                    continue;
                }
            };
            for event in events {
                if handle.inject_event(event).is_err() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
        }
    });
}

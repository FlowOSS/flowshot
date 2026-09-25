//! Captures the live session through the `org.freedesktop.portal` backends
//! and writes the full layout to `/tmp/flowshot-portal-<mode>.png`.
//!
//! This is the plan todo 10 QA harness: it drives [`PortalScreenshotBackend`]
//! (`--mode screenshot`) and [`PortalScreenCastBackend`] (`--mode screencast`)
//! directly. The stitched full-layout PNG is the artifact compared against
//! the `grim` oracle (`XDPH` itself uses `grim` for screenshots and
//! `wlr-screencopy` for screencast frames, so symmetry is expected).
//!
//! Usage:
//! - `portal_capture --mode screenshot` / `--mode screencast` - captures
//!   every output, prints per-frame buffer geometry/scale/transform plus
//!   mean and pixel-variance (the non-black assertion), stitches the layout
//!   bounding box, writes the PNG, prints the stitched dimensions, exits 0.
//! - `--output NAME` - captures the single output with that connector name
//!   (an unknown name produces the typed `OutputNotFound` and exit 1).
//! - `--png PATH` - overrides the output path.
//! - `--interactive` - screenshot mode only: sends `interactive: true`
//!   (ladder rung 5; the compositor picker decides the region, so the
//!   result is a single composite frame, not a layout stitch).
//! - `--permission` - runs `request_permission` and prints the result
//!   (dismiss simulation: the denied run must print `denied` and exit 0).
//!
//! [`PortalScreenshotBackend`]: flowshot_capture_wayland::PortalScreenshotBackend
//! [`PortalScreenCastBackend`]: flowshot_capture_wayland::PortalScreenCastBackend

use std::process::ExitCode;

#[path = "common/mod.rs"]
mod common;

use flowshot_capture::{
    BackendKind, CaptureBackend, CaptureOpts, Frame, OutputRef, PermissionResult,
};
use flowshot_capture_wayland::{PortalScreenCastBackend, PortalScreenshotBackend};
use flowshot_core::geometry::OutputLayout;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Screenshot,
    Screencast,
}

struct Args {
    mode: Mode,
    output: Option<String>,
    png: String,
    interactive: bool,
    permission: bool,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("flowshot portal capture failed: {err}");
            let mut source = std::error::Error::source(&*err);
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    futures::executor::block_on(async {
        if args.permission {
            let result = match args.mode {
                Mode::Screenshot => PortalScreenshotBackend::new().request_permission().await,
                Mode::Screencast => PortalScreenCastBackend::new().request_permission().await,
            };
            println!(
                "permission: {}",
                match result {
                    PermissionResult::Granted => "granted",
                    PermissionResult::Denied => "denied",
                    PermissionResult::NotRequired => "not-required",
                }
            );
            return Ok(());
        }
        match args.mode {
            Mode::Screenshot => run_screenshot(&args).await,
            Mode::Screencast => run_screencast(&args).await,
        }
    })
}

async fn run_screenshot(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let backend = if args.interactive {
        PortalScreenshotBackend::new_interactive()
    } else {
        PortalScreenshotBackend::new()
    };
    if let Some(connector) = &args.output {
        let frame = backend.capture_output_named(connector).await?;
        finish_single(&args.png, connector, &frame)
    } else if args.interactive {
        let frames = backend.capture_outputs(CaptureOpts::new(false)).await?;
        let frame = frames
            .into_iter()
            .next()
            .ok_or("the interactive picker returned no frame")?;
        common::print_frame_stats("picked", &frame);
        common::write_png(&args.png, &frame)?;
        println!(
            "wrote {} ({}x{})",
            args.png, frame.buffer.width, frame.buffer.height
        );
        Ok(())
    } else {
        finish_layout(
            &args.png,
            BackendKind::PortalScreenshot,
            &backend,
            CaptureOpts::new(false),
        )
        .await
    }
}

async fn run_screencast(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let backend = PortalScreenCastBackend::new();
    if let Some(connector) = &args.output {
        let frame = backend
            .capture_output_named(connector, CaptureOpts::new(false))
            .await?;
        finish_single(&args.png, connector, &frame)
    } else {
        finish_layout(
            &args.png,
            BackendKind::PortalScreenCast,
            &backend,
            CaptureOpts::new(false),
        )
        .await
    }
}

async fn finish_layout(
    png: &str,
    kind: BackendKind,
    backend: &dyn CaptureBackend,
    opts: CaptureOpts,
) -> Result<(), Box<dyn std::error::Error>> {
    let outputs = backend.outputs().await?;
    let frames = backend.capture_outputs(opts).await?;
    for frame in &frames {
        let name = match &frame.output {
            OutputRef::Connector(connector) => connector.clone(),
            OutputRef::Composite => "composite".to_owned(),
        };
        common::print_frame_stats(&name, frame);
    }
    let layout = OutputLayout::new(outputs.clone());
    let region = layout
        .union_bounds()
        .ok_or("the session has no outputs to stitch")?;
    let captured = flowshot_capture_wayland::CapturedOutputs { outputs, frames };
    let stitched = captured.stitch(kind, region)?;
    let (mean, variance) = common::buffer_stats(&stitched.buffer);
    println!(
        "stitched: {}x{} format={:?} mean={mean:.3} variance={variance:.3}",
        stitched.buffer.width, stitched.buffer.height, stitched.buffer.format
    );
    common::write_png(png, &stitched)?;
    println!(
        "wrote {png} ({}x{})",
        stitched.buffer.width, stitched.buffer.height
    );
    Ok(())
}

fn finish_single(
    png: &str,
    connector: &str,
    frame: &Frame,
) -> Result<(), Box<dyn std::error::Error>> {
    common::print_frame_stats(connector, frame);
    common::write_png(png, frame)?;
    println!(
        "wrote {png} ({}x{})",
        frame.buffer.width, frame.buffer.height
    );
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut args = Args {
        mode: Mode::Screenshot,
        output: None,
        png: String::new(),
        interactive: false,
        permission: false,
    };
    let mut flags = std::env::args().skip(1);
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--mode" => {
                let mode = flags
                    .next()
                    .ok_or("--mode requires screenshot|screencast")?;
                args.mode = match mode.as_str() {
                    "screenshot" => Mode::Screenshot,
                    "screencast" => Mode::Screencast,
                    other => return Err(format!("unknown mode {other}").into()),
                };
            }
            "--output" => {
                args.output = Some(
                    flags
                        .next()
                        .ok_or("--output requires a connector name argument")?,
                );
            }
            "--png" => {
                args.png = flags.next().ok_or("--png requires a path argument")?;
            }
            "--interactive" => args.interactive = true,
            "--permission" => args.permission = true,
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    if args.png.is_empty() {
        args.png = match args.mode {
            Mode::Screenshot => "/tmp/flowshot-portal-screenshot.png".to_owned(),
            Mode::Screencast => "/tmp/flowshot-portal-screencast.png".to_owned(),
        };
    }
    Ok(args)
}

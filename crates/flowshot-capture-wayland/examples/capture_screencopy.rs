//! Captures the live Wayland session through `wlr-screencopy-unstable-v1` and
//! writes the full layout to `/tmp/flowshot-screencopy.png`.
//!
//! This is the forced-screencopy QA harness: it drives
//! [`ScreencopyBackend`] directly, so it exercises the fallback path even on a
//! compositor (like `Hyprland`) that also offers `ext-image-copy-capture-v1`.
//! The stitched full-layout PNG is the artifact compared against the `grim`
//! oracle (grim itself uses `wlr-screencopy`, so symmetry is expected).
//!
//! Usage:
//! - `capture_screencopy` - captures every output sequentially, prints
//!   per-output buffer geometry/scale/transform plus mean and pixel-variance
//!   (the non-black assertion), stitches the layout bounding box, writes the
//!   PNG, prints the stitched dimensions and variance, exits 0.
//! - `capture_screencopy --output NAME` - captures the single output with that
//!   connector name (a name the session does not advertise produces the typed
//!   [`ScreencopyError::OutputNotFound`] on stderr and exit 1).
//! - `capture_screencopy --png PATH` - overrides the output path.
//!
//! [`ScreencopyError::OutputNotFound`]: flowshot_capture_wayland::ScreencopyError::OutputNotFound

use std::process::ExitCode;

use flowshot_capture::{BackendKind, CaptureBackend, CaptureOpts, Frame, FrameBuffer, OutputRef};
use flowshot_capture_wayland::ScreencopyBackend;
use flowshot_capture_wayland::stitch::to_rgba;
use flowshot_core::geometry::OutputLayout;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("flowshot screencopy capture failed: {err}");
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
    let backend = ScreencopyBackend::new();
    futures::executor::block_on(async {
        if let Some(connector) = args.output {
            let frame = backend
                .capture_output_named(&connector, CaptureOpts::new(false))
                .await?;
            print_frame_stats(&connector, &frame);
            write_png(&args.png, &frame)?;
            println!(
                "wrote {} ({}x{})",
                args.png, frame.buffer.width, frame.buffer.height
            );
        } else {
            let outputs = backend.outputs().await?;
            let frames = backend.capture_outputs(CaptureOpts::new(false)).await?;
            for frame in &frames {
                let name = match &frame.output {
                    OutputRef::Connector(connector) => connector.clone(),
                    OutputRef::Composite => "composite".to_owned(),
                };
                print_frame_stats(&name, frame);
            }
            let layout = OutputLayout::new(outputs.clone());
            let region = layout
                .union_bounds()
                .ok_or("the session has no outputs to stitch")?;
            let captured = flowshot_capture_wayland::CapturedOutputs { outputs, frames };
            let stitched = captured.stitch(BackendKind::WlrScreencopy, region)?;
            let (mean, variance) = buffer_stats(&stitched.buffer);
            println!(
                "stitched: {}x{} format={:?} mean={mean:.3} variance={variance:.3}",
                stitched.buffer.width, stitched.buffer.height, stitched.buffer.format
            );
            write_png(&args.png, &stitched)?;
            println!(
                "wrote {} ({}x{})",
                args.png, stitched.buffer.width, stitched.buffer.height
            );
        }
        Ok(())
    })
}

struct Args {
    output: Option<String>,
    png: String,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut args = Args {
        output: None,
        png: "/tmp/flowshot-screencopy.png".to_owned(),
    };
    let mut flags = std::env::args().skip(1);
    while let Some(flag) = flags.next() {
        match flag.as_str() {
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
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(args)
}

fn print_frame_stats(name: &str, frame: &Frame) {
    let (mean, variance) = buffer_stats(&frame.buffer);
    println!(
        "output {name}: buffer={}x{} scale={} transform={:?} format={:?} mean={mean:.3} \
         variance={variance:.3}",
        frame.buffer.width, frame.buffer.height, frame.scale, frame.transform, frame.buffer.format
    );
}

/// Mean and variance over the RGBA-converted byte values (0 = uniform black;
/// any real content has variance > 0 and mean > 0).
fn buffer_stats(buffer: &FrameBuffer) -> (f64, f64) {
    let mut count = 0u64;
    let mut sum = 0f64;
    let mut sum_squares = 0f64;
    for y in 0..buffer.height {
        for x in 0..buffer.width {
            let Some(pixel) = buffer.pixel(x, y) else {
                continue;
            };
            for channel in to_rgba(buffer.format, pixel) {
                let value = f64::from(channel);
                count += 1;
                sum += value;
                sum_squares += value * value;
            }
        }
    }
    if count == 0 {
        return (0.0, 0.0);
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "channel counts stay far below 2^53 (a 16K RGBA frame is 2^30 bytes)"
    )]
    let total = count as f64;
    let mean = sum / total;
    (mean, sum_squares / total - mean * mean)
}

fn write_png(path: &str, frame: &Frame) -> Result<(), Box<dyn std::error::Error>> {
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

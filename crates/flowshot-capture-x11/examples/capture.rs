//! Live X11 capture smoke - needs a running X session (`DISPLAY`).
//!
//! Drives the real [`X11Backend`] through the async [`CaptureBackend`] trait
//! (the daemon's call path): full-output capture -> `x11-capture.png`,
//! a logical region capture -> `x11-capture-region.png` (both under
//! `$FLOWSHOT_QA_OUT`, default a per-run temp directory), per-frame
//! geometry/format/byte-size dump with raw sample pixels (the solid-color
//! pixel oracle reads these), the one-shot cursor position, and a
//! `MIT-SHM` vs plain `GetImage` timing comparison.
//!
//! Run: `cargo run -p flowshot-capture-x11 --example capture`

use std::path::{Path, PathBuf};
use std::time::Instant;

use flowshot_capture::{CaptureBackend, CaptureOpts, FrameBuffer};
use flowshot_capture_x11::{X11Backend, stitch};
use flowshot_core::geometry::{Logical, LogicalRect};

type BoxError = Box<dyn std::error::Error>;

/// The QA output directory: `$FLOWSHOT_QA_OUT` when set, else a per-run
/// temp directory. Fixed shared `/tmp` paths clobber each other when two
/// runs overlap - and the QA pixel oracle reads these files, so a stale
/// or half-written PNG can be attributed to the wrong build.
fn out_dir() -> PathBuf {
    match std::env::var_os("FLOWSHOT_QA_OUT") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => std::env::temp_dir().join(format!("flowshot-x11-capture-{}", std::process::id())),
    }
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let out = out_dir();
    std::fs::create_dir_all(&out)?;
    let full_png = out.join("x11-capture.png");
    let region_png = out.join("x11-capture-region.png");
    let backend = X11Backend::connect()?;
    println!("DISPLAY: {}", backend.display());
    println!("caps: {:?}", backend.caps());

    for output in backend.outputs().await? {
        println!(
            "output {}: logical ({}, {} {}x{}) scale {} transform {:?} physical {}x{}",
            output.connector,
            output.logical_rect.x.0,
            output.logical_rect.y.0,
            output.logical_rect.width.0,
            output.logical_rect.height.0,
            output.scale,
            output.transform,
            output.physical_size.width.0,
            output.physical_size.height.0,
        );
    }

    match backend.cursor_pos().await? {
        Some(position) => println!(
            "cursor (global logical): ({}, {})",
            position.x.0, position.y.0
        ),
        None => println!("cursor: inside no output"),
    }

    let frames = backend.capture_outputs(CaptureOpts::default()).await?;
    for frame in &frames {
        describe_frame(frame);
    }
    write_png(
        &full_png,
        &frames.first().ok_or("capture returned no frames")?.buffer,
    )?;
    if frames.len() > 1 {
        for frame in &frames {
            let name = out.join(format!("x11-capture-{:?}.png", frame.output));
            write_png(&name, &frame.buffer)?;
        }
    }

    // Transport timing on equal work (both passes without cursor painting).
    let started = Instant::now();
    let shm_frames = backend.capture_outputs(CaptureOpts::new(false)).await?;
    let shm_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let plain_frames = backend
        .clone()
        .without_shm()
        .capture_outputs(CaptureOpts::new(false))
        .await?;
    let plain_ms = started.elapsed().as_millis();
    println!(
        "timing: shm-capable pass {shm_ms} ms vs forced-plain pass {plain_ms} ms ({} vs {} frames)",
        shm_frames.len(),
        plain_frames.len()
    );
    if let (Some(painted), Some(plain)) = (frames.first(), shm_frames.first()) {
        describe_paint_diff(painted, plain);
    }

    let region = LogicalRect::new(
        Logical(100.0),
        Logical(100.0),
        Logical(800.0),
        Logical(600.0),
    );
    let stitched = backend.capture_region(region).await?;
    describe_frame(&stitched);
    write_png(&region_png, &stitched.buffer)?;
    println!("wrote {} and {}", full_png.display(), region_png.display());
    Ok(())
}

/// Diffs the cursor-painted pass against the unpainted pass: on a static
/// screen every differing pixel is painted cursor (or screen churn between
/// the two grabs), and the bounding box localizes it.
fn describe_paint_diff(painted: &flowshot_capture::Frame, plain: &flowshot_capture::Frame) {
    let buffer = &painted.buffer;
    if (buffer.width, buffer.height) != (plain.buffer.width, plain.buffer.height) {
        println!("paint-cursor diff: frames differ in size; skipping");
        return;
    }
    let mut count = 0u64;
    let (mut min_x, mut min_y) = (u32::MAX, u32::MAX);
    let (mut max_x, mut max_y) = (0u32, 0u32);
    for y in 0..buffer.height {
        for x in 0..buffer.width {
            if buffer.pixel(x, y) == plain.buffer.pixel(x, y) {
                continue;
            }
            count += 1;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if count == 0 {
        println!("paint-cursor diff: 0 pixels (cursor NOT painted, or identical screens)");
    } else {
        println!(
            "paint-cursor diff: {count} pixels, bbox ({min_x}, {min_y})-({max_x}, {max_y}) \
             (painted vs unpainted pass; screen churn also counts)"
        );
    }
}

fn describe_frame(frame: &flowshot_capture::Frame) {
    let buffer = &frame.buffer;
    println!(
        "frame {:?}: {}x{} stride {} {:?} | {} bytes | scale {} transform {:?}",
        frame.output,
        buffer.width,
        buffer.height,
        buffer.stride,
        buffer.format,
        buffer.data.len(),
        frame.scale,
        frame.transform,
    );
    let last_x = buffer.width.saturating_sub(1);
    let last_y = buffer.height.saturating_sub(1);
    for (label, x, y) in [
        ("top-left", 0, 0),
        ("center", buffer.width / 2, buffer.height / 2),
        ("bottom-right", last_x, last_y),
    ] {
        println!("  {label} ({x}, {y}) raw bytes: {:?}", buffer.pixel(x, y));
    }
}

fn write_png(path: &Path, buffer: &FrameBuffer) -> Result<(), BoxError> {
    let width = usize::try_from(buffer.width)?;
    let height = usize::try_from(buffer.height)?;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..buffer.height {
        for x in 0..buffer.width {
            let pixel = buffer.pixel(x, y).ok_or("pixel outside its own buffer")?;
            rgba.extend_from_slice(&stitch::to_rgba(buffer.format, pixel));
        }
    }
    let image = image::RgbaImage::from_raw(u32::try_from(width)?, u32::try_from(height)?, rgba)
        .ok_or("PNG geometry mismatch")?;
    image.save(path)?;
    println!("wrote {} ({width}x{height} RGBA -> PNG)", path.display());
    Ok(())
}

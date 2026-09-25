//! Shared QA-harness helpers for the capture examples (frame statistics and
//! PNG writing). Lives in a subdirectory so cargo does not treat it as an
//! example target; consumed via `#[path = "common/mod.rs"]`.

use flowshot_capture::{Frame, FrameBuffer};
use flowshot_capture_wayland::stitch::to_rgba;

pub fn print_frame_stats(name: &str, frame: &Frame) {
    let (mean, variance) = buffer_stats(&frame.buffer);
    println!(
        "output {name}: buffer={}x{} scale={} transform={:?} format={:?} mean={mean:.3} \
         variance={variance:.3}",
        frame.buffer.width, frame.buffer.height, frame.scale, frame.transform, frame.buffer.format
    );
}

/// Mean and variance over the RGBA-converted byte values (0 = uniform black;
/// any real content has variance > 0 and mean > 0).
pub fn buffer_stats(buffer: &FrameBuffer) -> (f64, f64) {
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

pub fn write_png(path: &str, frame: &Frame) -> Result<(), Box<dyn std::error::Error>> {
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

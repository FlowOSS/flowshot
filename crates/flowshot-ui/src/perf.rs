//! Live frame-timing profiler (feature `perf-trace`).
//!
//! The interactive-freeze diagnosis instrument: records the per-window frame
//! breakdown the offscreen harness cannot see - the present path
//! (`get_current_texture` FIFO blocking + compositor handoff) and the redraw
//! cadence winit actually delivers - and emits p50/p95/p99 aggregates over
//! tracing every [`REPORT_EVERY`] frames. Compiled OUT of production builds
//! (the module declaration and every `record` call site are feature-gated), so
//! a shipping overlay carries zero instrumentation overhead.
//!
//! The breakdown per window, in microseconds:
//! - `list_build`: `build_overlay_frame` (backdrop + editor + selection +
//!   chrome + magnifier display-list construction, CPU).
//! - `tess_shape`: `FrameStats::build_time` (lyon tessellation + cosmic-text
//!   shaping, CPU).
//! - `encode_submit`: `FrameStats::encode_time` (staging upload + command
//!   encode + `queue.submit`, CPU).
//! - `present`: surface total minus the renderer CPU time - the
//!   `get_current_texture` FIFO wait + crosshair encode + `present` handoff.
//! - `frame_total`: `list_build` + the full surface call.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::render::FrameStats;

/// Emit an aggregate report every this-many recorded frames.
const REPORT_EVERY: usize = 120;

thread_local! {
    static PROFILER: RefCell<FrameProfiler> = RefCell::new(FrameProfiler::new());
}

/// Records one window's frame timing (the `render_window` call site).
pub(crate) fn record(
    slot: usize,
    list_build: Duration,
    surface_total: Duration,
    stats: FrameStats,
) {
    PROFILER.with(|profiler| {
        profiler
            .borrow_mut()
            .record(slot, list_build, surface_total, stats);
    });
}

/// One window's accumulated per-phase samples (microseconds).
#[derive(Debug, Default)]
struct WindowSamples {
    list_build: Vec<f64>,
    tess_shape: Vec<f64>,
    encode_submit: Vec<f64>,
    acquire: Vec<f64>,
    present: Vec<f64>,
    frame_total: Vec<f64>,
}

/// The aggregate frame profiler: per-window samples plus the redraw cadence.
#[derive(Debug)]
struct FrameProfiler {
    windows: Vec<WindowSamples>,
    started: Instant,
    frames: usize,
}

impl FrameProfiler {
    fn new() -> Self {
        Self {
            windows: Vec::new(),
            started: Instant::now(),
            frames: 0,
        }
    }

    fn record(
        &mut self,
        slot: usize,
        list_build: Duration,
        surface_total: Duration,
        stats: FrameStats,
    ) {
        if slot >= self.windows.len() {
            self.windows.resize_with(slot + 1, WindowSamples::default);
        }
        let micros = |duration: Duration| duration.as_secs_f64() * 1e6;
        let list_build_us = micros(list_build);
        let surface_us = micros(surface_total);
        let cpu_us = micros(stats.cpu_time);
        let acquire_us = micros(stats.acquire_time);
        // The present handoff is the surface call minus the renderer CPU work
        // minus the swapchain-acquire wait; a negative delta (clock jitter)
        // clamps to zero.
        let present_us = (surface_us - cpu_us - acquire_us).max(0.0);
        let window = &mut self.windows[slot];
        window.list_build.push(list_build_us);
        window.tess_shape.push(micros(stats.build_time));
        window.encode_submit.push(micros(stats.encode_time()));
        window.acquire.push(acquire_us);
        window.present.push(present_us);
        window.frame_total.push(list_build_us + surface_us);
        self.frames += 1;
        if self.frames.is_multiple_of(REPORT_EVERY) {
            self.report();
        }
    }

    fn report(&mut self) {
        let elapsed = self.started.elapsed().as_secs_f64();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a frame count over a 120-frame window is far inside f64's exact-integer range"
        )]
        let rate = if elapsed > 0.0 {
            self.frames as f64 / elapsed
        } else {
            0.0
        };
        tracing::info!(
            target: "flowshot_ui::perf",
            frames = self.frames,
            elapsed_s = format!("{elapsed:.2}"),
            redraws_per_sec = format!("{rate:.1}"),
            "frame timing aggregate ({} frames)",
            self.frames
        );
        for (slot, window) in self.windows.iter_mut().enumerate() {
            for (phase, samples) in [
                ("list_build", &mut window.list_build),
                ("tess_shape", &mut window.tess_shape),
                ("encode_submit", &mut window.encode_submit),
                ("acquire", &mut window.acquire),
                ("present", &mut window.present),
                ("frame_total", &mut window.frame_total),
            ] {
                let (p50, p95, p99, max) = summarize(samples);
                tracing::info!(
                    target: "flowshot_ui::perf",
                    window = slot,
                    phase,
                    p50_us = format!("{p50:.1}"),
                    p95_us = format!("{p95:.1}"),
                    p99_us = format!("{p99:.1}"),
                    max_us = format!("{max:.1}"),
                    "PERF win{slot} {phase}: p50={p50:.1}us p95={p95:.1}us p99={p99:.1}us max={max:.1}us"
                );
                samples.clear();
            }
        }
        self.frames = 0;
        self.started = Instant::now();
    }
}

/// p50/p95/p99/max of `samples` (microseconds), or zeros when empty.
fn summarize(samples: &mut [f64]) -> (f64, f64, f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let max = samples[samples.len() - 1];
    (
        percentile(samples, 50.0),
        percentile(samples, 95.0),
        percentile(samples, 99.0),
        max,
    )
}

/// Linear-interpolated percentile of a SORTED slice.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the rank is clamped into [0, len-1] (a valid slice index) before the usize cast, and a sample count over one report window is far inside f64's exact-integer range"
)]
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    let Some(last) = sorted.len().checked_sub(1) else {
        return 0.0;
    };
    let rank = (pct / 100.0) * last as f64;
    let low = rank.floor().clamp(0.0, last as f64) as usize;
    let high = rank.ceil().clamp(0.0, last as f64) as usize;
    if low == high {
        return sorted[low];
    }
    let frac = rank - low as f64;
    sorted[low] * (1.0 - frac) + sorted[high] * frac
}

//! Startup milestones on the shipped overlay path (issue #16 option 4).
//!
//! The offscreen harness already reports `perf.gpu_instance` and
//! `perf.gpu_ready`, but the LIVE child emitted nothing for its GPU leg: a
//! hybrid-graphics report could only reason about the wake from a harness
//! number, and the moment the overlay becomes input-usable had no milestone at
//! all (the window leg was "the deferred GUI-QA batch"). This puts the shipped
//! child on the same vocabulary the daemon already uses: `target:
//! "flowshot_perf"` events with monotonic `elapsed_us` from ONE anchor - the
//! instant the child's overlay application is built - so each leg is a
//! subtraction between consecutive milestones instead of an estimate.
//!
//! Milestones, in order:
//! - `perf.windows_ready` - every window created (the count stays on the
//!   existing `overlay windows spawned` info line);
//! - `perf.gpu_instance` - `wgpu::Instance` built and adapters enumerated,
//!   the leg a discrete driver wakes on a hybrid (PRIME) laptop;
//! - `perf.gpu_ready` - adapter, device and queue requested;
//! - `perf.renderers_ready` - every window's renderer built, which is where the
//!   glyph atlas and the cosmic-text font DB load land;
//! - `perf.ui_ready` - the FIRST successful present, i.e. the frame that makes
//!   the overlay interactable. Reported exactly once per child.
//!
//! Always compiled in, unlike [`crate::perf`] (per-frame cadence behind
//! `perf-trace`, feature-gated out of production): the cost here is one
//! `Instant` and a handful of info events per session, which is what lets a
//! user's latency report be answered with numbers.

use std::time::Instant;

/// One overlay child's startup clock (single anchor, once-only ready report).
#[derive(Debug)]
pub(crate) struct StartupClock {
    started: Instant,
    ready_reported: bool,
}

impl StartupClock {
    /// Anchors the clock at the current instant.
    #[must_use]
    pub(crate) fn start() -> Self {
        Self {
            started: Instant::now(),
            ready_reported: false,
        }
    }

    /// Emits one milestone as `elapsed_us` from the anchor.
    pub(crate) fn record(&mut self, milestone: &str) {
        tracing::info!(
            target: "flowshot_perf",
            elapsed_us = elapsed(self.started),
            "{milestone}"
        );
    }

    /// Reports `perf.ui_ready` on the first present and never again.
    ///
    /// Returns whether this call was the one that reported it, so a caller on
    /// the frame path can tell the milestone frame from every later frame.
    pub(crate) fn report_first_present(&mut self) -> bool {
        if self.ready_reported {
            return false;
        }
        self.ready_reported = true;
        self.record("perf.ui_ready");
        true
    }
}

/// `elapsed_us` in the same shape as the daemon's perf helper.
fn elapsed(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;

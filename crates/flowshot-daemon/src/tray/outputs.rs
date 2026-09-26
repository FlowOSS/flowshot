//! Live output probing for the per-monitor submenu (plan todo 33: "live
//! probe outputs, todo 6").
//!
//! [`WaylandOutputProbe`] wraps the todo-6 [`CaptureThread`] registry
//! probe: a dedicated wayland connection, no GPU, no capture session,
//! bounded by the thread's 10 s deadlines. It is INFALLIBLE by contract -
//! headless boxes, absent sockets and frozen compositors all degrade to an
//! empty list (the menu then shows the disabled placeholder), because the
//! tray must never take the daemon down. Callers run it inside
//! `spawn_blocking` (the probe blocks on its own thread handshake).

use flowshot_core::geometry::OutputInfo;

/// Where the submenu's monitor list comes from (test seam: stubs return
/// fixed outputs without a compositor).
pub trait OutputProbe: Send + Sync + std::fmt::Debug {
    /// The currently visible outputs in registry order (empty on any
    /// failure - implementations log, never propagate).
    fn probe(&self) -> Vec<OutputInfo>;
}

/// Production probe: the todo-6 [`CaptureThread`](flowshot_capture_wayland::CaptureThread)
/// registry enumeration, spawned per refresh (connect -> probe -> close;
/// refreshes are rare - tray start and submenu opens).
#[derive(Debug, Clone, Copy, Default)]
pub struct WaylandOutputProbe;

impl OutputProbe for WaylandOutputProbe {
    fn probe(&self) -> Vec<OutputInfo> {
        let thread = match flowshot_capture_wayland::CaptureThread::spawn() {
            Ok(thread) => thread,
            Err(error) => {
                tracing::warn!(%error, "output probe could not reach a compositor; submenu stays empty");
                return Vec::new();
            }
        };
        let outputs = thread.outputs().unwrap_or_else(|error| {
            tracing::warn!(%error, "output probe failed; submenu stays empty");
            Vec::new()
        });
        thread.shutdown();
        outputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    #[derive(Debug)]
    struct StubProbe(Vec<OutputInfo>);

    impl OutputProbe for StubProbe {
        fn probe(&self) -> Vec<OutputInfo> {
            self.0.clone()
        }
    }

    #[test]
    fn stub_probe_feeds_the_menu_seam() {
        let output = OutputInfo::new(
            "DP-1",
            "Test",
            LogicalRect::new(Logical(0.0), Logical(0.0), Logical(800.0), Logical(600.0)),
            PhysicalSize::new(PhysicalPx(800), PhysicalPx(600)),
            1.0,
            Transform::Normal,
        )
        .unwrap_or_else(|error| panic!("fixture must be valid: {error}"));
        let probe = StubProbe(vec![output.clone()]);
        assert_eq!(probe.probe(), vec![output]);
        assert!(StubProbe(vec![]).probe().is_empty());
    }
}

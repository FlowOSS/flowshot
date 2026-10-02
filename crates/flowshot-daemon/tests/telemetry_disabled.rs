//! The disabled-path guarantee (CI-testable per the telemetry spec):
//! with `[telemetry].enabled = false`, `init` returns `None` and NOTHING
//! is initialized - no client, no transport, no worker thread, no DSN
//! resolution, no environment probes. The capture wrappers stay inert
//! no-ops gated on the `OnceLock` state.
//!
//! This file is a DEDICATED test binary with exactly ONE test: sentry's
//! hub is process-global, so the enabled-init test lives in its own
//! binary (`telemetry_mock_transport.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use flowshot_core::config::TelemetryConfig;
use flowshot_daemon::telemetry::{self, Surface};

#[test]
fn disabled_telemetry_initializes_nothing_and_captures_are_inert() {
    // Given: the default consent (every flag false - nothing was asked or
    // granted).
    let config = TelemetryConfig::default();
    assert!(!config.enabled);

    // When: init runs for any surface.
    let guard = telemetry::init(&config, Surface::Daemon);

    // Then: no client guard exists (the None path returns BEFORE the DSN
    // parse, the probes, and sentry::init - there is no transport that
    // could receive anything and no thread that could send it), the state
    // gate stays closed, and every capture wrapper is an inert no-op.
    assert!(guard.is_none());
    assert!(!telemetry::is_enabled());

    telemetry::capture_error(&std::io::Error::other("must never be sent"));
    telemetry::capture_error_tagged(
        &std::io::Error::other("must never be sent"),
        &[("backend", "none")],
    );
    telemetry::capture_message("must never be sent", sentry::Level::Error);
    telemetry::note_gpu_adapter("must never be attached");

    assert!(!telemetry::is_enabled());

    // And: a second init (even with consent flipped on) changes nothing -
    // the first init wins, so a stray later call cannot re-arm capture in
    // a process that booted with telemetry off.
    let enabled = TelemetryConfig {
        enabled: true,
        include_technical_details: false,
        asked_on_first_launch: true,
    };
    assert!(telemetry::init(&enabled, Surface::Cli).is_none());
    assert!(!telemetry::is_enabled());
}

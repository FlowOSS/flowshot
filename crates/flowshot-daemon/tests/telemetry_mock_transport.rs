//! The mock-transport end-to-end proof: an ENABLED tier-1 init plus one
//! `capture_error` produces EXACTLY ONE envelope at the transport, whose
//! event payload carries the tier-1 taxonomy tags, the scrubbed path, and
//! NONE of the tier-2 fields, no `server_name`, and no real network
//! (the injected transport is the only send path).
//!
//! DEDICATED test binary with exactly ONE sentry-initializing test: the
//! hub is process-global (the disabled-path assert lives in
//! `telemetry_disabled.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex, PoisonError};

use flowshot_core::config::TelemetryConfig;
use flowshot_daemon::telemetry::{self, Surface};
use sentry::protocol::EnvelopeItem;
use sentry::{Envelope, Transport};

#[derive(Debug, Default)]
struct MockTransport {
    envelopes: Mutex<Vec<Envelope>>,
}

impl Transport for MockTransport {
    fn send_envelope(&self, envelope: Envelope) {
        self.envelopes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(envelope);
    }
}

#[test]
fn enabled_tier1_sends_exactly_one_sanitized_envelope() {
    // Given: enabled tier-1 consent and the mock transport (sentry's
    // blanket `TransportFactory for Arc<T: Transport>` impl injects it;
    // the production reqwest transport is never constructed).
    let transport = Arc::new(MockTransport::default());
    let config = TelemetryConfig {
        enabled: true,
        include_technical_details: false,
        asked_on_first_launch: true,
    };

    // When: init runs and one typed error with an embedded home path is
    // captured.
    // The blanket `TransportFactory for Arc<T: Transport>` impl makes the
    // shared Arc itself the factory; the outer Arc carries it as the
    // trait object `init_with_transport` takes.
    let factory: Arc<dyn sentry::TransportFactory> = Arc::new(Arc::clone(&transport));
    let guard = telemetry::init_with_transport(&config, Surface::Cli, Some(factory));
    assert!(guard.is_some(), "enabled init must return the client guard");
    assert!(telemetry::is_enabled());
    let error = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "config missing at /home/alice/.config/flowshot/flowshot.toml",
    );
    telemetry::capture_error(&error);

    // Then: exactly one envelope arrived, synchronously, carrying exactly
    // one event item.
    let envelopes = transport
        .envelopes
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(envelopes.len(), 1, "exactly one envelope expected");
    let mut events = Vec::new();
    for item in envelopes[0].items() {
        if let EnvelopeItem::Event(event) = item {
            events.push((**event).clone());
        }
    }
    assert_eq!(events.len(), 1, "the envelope must carry exactly one event");
    let json = serde_json::to_string(&events[0]).unwrap();
    drop(envelopes);
    drop(guard);

    // The evidence sample (pretty-printed for .omo/evidence via
    // --nocapture; the assertions below are the contract).
    let pretty =
        serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&json).unwrap())
            .unwrap();
    println!("SCRUBBED-TIER1-EVENT-BEGIN\n{pretty}\nSCRUBBED-TIER1-EVENT-END");

    let value: serde_json::Value = serde_json::from_str(&json).unwrap();

    // Tier-1 taxonomy tags are present, with the surface tag from init.
    let tags = value.get("tags").expect("the event must carry tags");
    assert_eq!(tags.get("surface").and_then(|v| v.as_str()), Some("cli"));
    for key in [
        "distro",
        "arch",
        "package_manager",
        "session_type",
        "desktop",
        "gpu_family",
    ] {
        assert!(
            tags.get(key).is_some(),
            "missing tier-1 tag {key} in {json}"
        );
    }

    // No tier-2 field anywhere in the payload.
    for forbidden in ["install_id", "monitors", "kernel_release", "gpu_adapter"] {
        assert!(
            !json.contains(forbidden),
            "tier-2 field {forbidden} leaked into tier-1 payload"
        );
    }
    assert!(
        value
            .get("contexts")
            .and_then(|contexts| contexts.get("flowshot"))
            .is_none(),
        "the technical context must not exist in tier 1"
    );

    // The identifier strip held: no server_name, no user, no request.
    assert!(value.get("server_name").is_none());
    assert!(value.get("user").is_none());
    assert!(value.get("request").is_none());

    // The path scrubber ran on the exception message.
    assert!(
        !json.contains("/home/alice"),
        "the home path was not scrubbed: {json}"
    );
    assert!(
        json.contains("~/.config/flowshot/flowshot.toml"),
        "the scrubbed ~/ path is missing: {json}"
    );

    // The release is the baked package version, never a hostname or path.
    assert_eq!(
        value.get("release").and_then(|v| v.as_str()),
        Some(concat!("flowshot@", env!("CARGO_PKG_VERSION")))
    );
}

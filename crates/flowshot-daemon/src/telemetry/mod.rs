//! Opt-in error telemetry for the self-hosted `FlowShot` Sentry instance.
//!
//! # Consent model (user-locked)
//!
//! Two tiers, both OFF by default (`[telemetry]` in `flowshot-core`):
//!
//! - `enabled` - the master switch. While it is false, [`init`] does
//!   NOTHING: no client, no transport, no worker thread, no DSN
//!   resolution, no environment probes - and every capture wrapper is a
//!   no-op gated on the process-global [`OnceLock`] state below (NOT on
//!   `sentry::is_enabled`, so a stray second init cannot re-arm capture).
//! - `include_technical_details` - tier 2 (GDPR-relevant): adds the full
//!   GPU adapter string, the exact kernel release, the monitor layout
//!   (connector + size + scale), and a per-install random UUID (persisted
//!   at `<xdg-data-home>/flowshot/telemetry-id`; deleting the file
//!   regenerates it). Tier 1 - always attached when enabled - is the
//!   coarse taxonomy only: distro, arch, package manager, session type,
//!   desktop family, compositor version (Hyprland only), GPU FAMILY, and
//!   the `surface` tag, on top of path-scrubbed, identifier-stripped error
//!   reports (the crate-private `sanitize` module).
//!
//! # Process model
//!
//! Every `FlowShot` process initializes independently (the CLI, the daemon,
//! and each window-session child - [`crate::execute::session::run_child`]
//! calls [`init`] with the surface its spec names). [`init`] must run on
//! the process's main flow BEFORE any worker thread is spawned and its
//! [`sentry::ClientInitGuard`] must be held until exit: dropping the guard
//! flushes and shuts the transport down, so captures after the drop go
//! nowhere. The first [`init`] call wins; later calls return `None`.
//!
//! # Panics
//!
//! The sentry `panic` integration (a default integration) installs a
//! process-global panic hook at init, so panics on ANY thread are captured
//! automatically once telemetry is enabled - no per-site wiring needed.
//!
//! # Endpoint policy
//!
//! The DSN below is the ONLY place the endpoint exists: a build-time
//! constant, never a config field, never an env override (`SENTRY_DSN` is
//! shadowed because the DSN is always set explicitly).

mod environment;
mod payload;
mod sanitize;
mod scrub;

use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use flowshot_core::config::TelemetryConfig;

/// The self-hosted Sentry DSN: a PUBLIC client key (Sentry DSNs are public
/// by design - the key authorizes event ingestion only, never reads),
/// user-locked to `sentry.flowhost.io`, and NEVER configurable.
const DSN: &str = "https://b2cdd62bba0719283896c98aa2438024@sentry.flowhost.io/5";

/// The release string: the sentry `name@version` convention from the
/// workspace version.
const RELEASE: &str = concat!("flowshot@", env!("CARGO_PKG_VERSION"));

/// Crates whose stack frames are marked in-app.
const IN_APP_CRATES: &[&str] = &[
    "flowshot_core",
    "flowshot_capture",
    "flowshot_capture_wayland",
    "flowshot_capture_x11",
    "flowshot_ui",
    "flowshot_actions",
    "flowshot_daemon",
    "flowshot_cli",
];

/// The process surface an event was captured on (the `surface` tag on
/// every event).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The `flowshot` CLI process (non-daemon invocations).
    Cli,
    /// The resident daemon (`flowshot daemon` / the helper binary).
    Daemon,
    /// A capture-overlay (or color-picker) session child.
    Overlay,
    /// A pin window session child.
    Pin,
    /// The launcher-dialog session child.
    Launcher,
    /// The settings-window session child.
    Settings,
    /// The first-launch consent-dialog session child.
    Consent,
}

impl Surface {
    /// The tag value for this surface.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Surface::Cli => "cli",
            Surface::Daemon => "daemon",
            Surface::Overlay => "overlay",
            Surface::Pin => "pin",
            Surface::Launcher => "launcher",
            Surface::Settings => "settings",
            Surface::Consent => "consent",
        }
    }
}

impl From<crate::execute::session::SessionKind> for Surface {
    fn from(kind: crate::execute::session::SessionKind) -> Self {
        use crate::execute::session::SessionKind;
        match kind {
            SessionKind::Overlay => Surface::Overlay,
            SessionKind::Launcher => Surface::Launcher,
            SessionKind::Settings => Surface::Settings,
            SessionKind::Pin => Surface::Pin,
            SessionKind::Consent => Surface::Consent,
        }
    }
}

/// The init-time facts shared by the sanitizer and the capture gate.
pub(crate) struct Facts {
    /// The tier-1 taxonomy (probed once at init).
    pub(crate) environment: Arc<environment::Environment>,
    /// The tier-2 payload; `Some` exactly when `include_technical_details`
    /// consented AND telemetry is enabled (the sanitizer's tier flag).
    pub(crate) payload: Option<Arc<payload::Tier2Payload>>,
    /// The wgpu adapter pass-through slot
    /// ([`note_gpu_adapter`]); wins over the sysfs fallback at event time.
    pub(crate) gpu_slot: Arc<RwLock<Option<String>>>,
}

impl Facts {
    /// The slot value; a poisoned lock is read through (a plain
    /// `Option<String>` holds no invariant to protect).
    fn gpu_adapter(&self) -> Option<String> {
        self.gpu_slot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// `Some(facts)` = telemetry enabled; `None` = disabled (or never
/// initialized - both no-op identically).
static STATE: OnceLock<Option<Arc<Facts>>> = OnceLock::new();

/// Initializes telemetry for this process.
///
/// Returns the client guard (hold it until exit - dropping it flushes and
/// closes the transport) when `config.enabled`, and `None` when disabled:
/// in that case NOTHING is initialized - no probes, no DSN resolution, no
/// sentry client, transport, or thread - and the capture wrappers stay
/// no-ops. The first call in a process wins; later calls return `None`.
#[must_use = "the guard flushes on drop; holding it keeps the transport alive"]
pub fn init(config: &TelemetryConfig, surface: Surface) -> Option<sentry::ClientInitGuard> {
    init_with_transport(config, surface, None)
}

/// [`init`] with an injectable transport factory (the mock-transport test
/// seam; `None` = the production reqwest/rustls transport).
///
/// # Errors
///
/// Never fails: an unparsable DSN (impossible for the constant above)
/// degrades to disabled with an error log instead of panicking.
pub fn init_with_transport(
    config: &TelemetryConfig,
    surface: Surface,
    transport: Option<Arc<dyn sentry::TransportFactory>>,
) -> Option<sentry::ClientInitGuard> {
    if STATE.get().is_some() {
        tracing::debug!("telemetry already initialized; first init wins");
        return None;
    }
    if !config.enabled {
        record_state(None);
        return None;
    }
    let Ok(dsn) = DSN.parse::<sentry::types::Dsn>() else {
        tracing::error!("telemetry DSN failed to parse; telemetry stays disabled");
        record_state(None);
        return None;
    };

    let facts = Arc::new(probe_facts(*config));
    let sanitizer = Arc::new(sanitize::Sanitizer::new(surface.tag(), Arc::clone(&facts)));
    let mut options = sentry::ClientOptions::new()
        .release(RELEASE)
        .send_default_pii(false)
        .before_send(move |event| Some(sanitizer.sanitize(event)));
    options.dsn = Some(dsn);
    options.in_app_include = IN_APP_CRATES.to_vec();
    options.transport = transport;

    let guard = sentry::init(options);
    record_state(Some(facts));
    Some(guard)
}

/// The init-time probe run (enabled path only - a disabled config does
/// zero work): the tier-1 taxonomy always, the tier-2 payload only with
/// the `include_technical_details` consent.
fn probe_facts(config: TelemetryConfig) -> Facts {
    let environment = Arc::new(environment::Environment::probe());
    let payload = config.include_technical_details.then(|| {
        Arc::new(payload::Tier2Payload::probe(
            crate::paths::default_telemetry_id_path().ok().as_deref(),
        ))
    });
    Facts {
        environment,
        payload,
        gpu_slot: Arc::new(RwLock::new(None)),
    }
}

fn record_state(state: Option<Arc<Facts>>) {
    if STATE.set(state).is_err() {
        tracing::debug!("telemetry state already recorded; first init wins");
    }
}

/// Whether telemetry is enabled in this process (the capture gate - the
/// [`OnceLock`] state, not `sentry::is_enabled`).
#[must_use]
pub fn is_enabled() -> bool {
    enabled_facts().is_some()
}

fn enabled_facts() -> Option<&'static Arc<Facts>> {
    STATE.get().and_then(Option::as_ref)
}

/// Reports a typed error (its full source chain becomes the exception
/// list). No-op when telemetry is disabled or uninitialized.
pub fn capture_error(error: &(dyn std::error::Error + 'static)) {
    if enabled_facts().is_none() {
        return;
    }
    sentry::capture_error(error);
}

/// [`capture_error`] plus per-event tags (e.g. the capture `backend` kind
/// at the executor boundary). No-op when disabled.
pub fn capture_error_tagged(error: &(dyn std::error::Error + 'static), tags: &[(&str, &str)]) {
    if enabled_facts().is_none() {
        return;
    }
    sentry::with_scope(
        |scope| {
            for &(key, value) in tags {
                scope.set_tag(key, value);
            }
        },
        || {
            sentry::capture_error(error);
        },
    );
}

/// Reports a message event. No-op when telemetry is disabled or
/// uninitialized.
pub fn capture_message(message: &str, level: sentry::Level) {
    if enabled_facts().is_none() {
        return;
    }
    sentry::capture_message(message, level);
}

/// The tier-2 GPU pass-through: a surface that knows its wgpu adapter
/// description (flowshot-ui's `gpu.rs` selection) hands it over here, and
/// later events report it instead of the `/sys` PCI-id fallback. Inert
/// while telemetry is disabled.
pub fn note_gpu_adapter(description: &str) {
    let Some(facts) = enabled_facts() else {
        return;
    };
    let mut slot = facts
        .gpu_slot
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    *slot = Some(description.to_owned());
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn surface_tags_are_distinct_and_lowercase() {
        let tags = [
            Surface::Cli.tag(),
            Surface::Daemon.tag(),
            Surface::Overlay.tag(),
            Surface::Pin.tag(),
            Surface::Launcher.tag(),
            Surface::Settings.tag(),
            Surface::Consent.tag(),
        ];
        for (index, tag) in tags.iter().enumerate() {
            assert!(!tags[..index].contains(tag), "duplicate tag {tag}");
            assert!(
                tag.chars().all(|c| c.is_ascii_lowercase()),
                "tag {tag} must be lowercase"
            );
        }
    }

    #[test]
    fn the_dsn_constant_parses() {
        // The one place the DSN is resolved; a typo here must fail the
        // build-time-adjacent test, not a user's runtime.
        assert!(DSN.parse::<sentry::types::Dsn>().is_ok());
    }

    #[test]
    fn capture_wrappers_no_op_before_init() {
        // In the lib-test process telemetry is never initialized (the
        // init-touching tests live in dedicated integration binaries):
        // every wrapper must be an inert no-op, and is_enabled false.
        assert!(!is_enabled());
        capture_error(&std::io::Error::other("must not be captured"));
        capture_message("must not be captured", sentry::Level::Error);
        capture_error_tagged(
            &std::io::Error::other("must not be captured"),
            &[("backend", "none")],
        );
        note_gpu_adapter("inert");
        assert!(!is_enabled());
    }
}

//! The CLI -> daemon `D-Bus` mapping (the wire contract; the
//! CLI<->`D-Bus` mapping table the docs record).
//!
//! Forwarding prefers the TYPED members whenever they are lossless:
//!
//! | CLI form | Wire member |
//! |----------|-------------|
//! | `capture` (interactive/preselected/last) | `Capture(a{sv})` over [`flowshot_daemon::request::CAPTURE_OPTION_KEYS`] |
//! | `capture full` (no modifiers) | `CaptureFull` |
//! | `capture screen <n>` (no modifiers) | `CaptureScreen(u)` |
//! | `capture --dialog` | `Launcher` |
//! | `settings` | `Settings` |
//! | everything else (`full`/`screen` WITH modifiers, `screen` at-cursor or by connector, `pin`, `color`) | `Invoke(as)` - the lossless argv channel; the daemon re-parses it with this crate's clap surface |
//!
//! One-shot invocations (`--no-daemon`, `--raw`, `--print-geometry`) never
//! reach this module: stdout is never routed over `D-Bus`;
//! [`crate::dispatch`] keeps them in-process.
//!
//! [`CAPTURE_OPTION_KEYS`]: flowshot_daemon::request::CAPTURE_OPTION_KEYS

use std::collections::HashMap;

use flowshot_daemon::request::CAPTURE_OPTION_KEYS;
use flowshot_daemon::{CaptureRequest, IFACE, OBJECT_PATH, SERVICE};
use zbus::Connection;
use zbus::zvariant::{Str, Value};

use crate::invocation::{CaptureInvocation, CaptureSelection, ScreenSpec};

/// One forwarded `D-Bus` call.
#[derive(Debug, PartialEq, Eq)]
pub enum WireCall {
    /// `Capture(a{sv})` with the [`CAPTURE_OPTION_KEYS`] vocabulary.
    Capture(HashMap<String, Value<'static>>),
    /// `CaptureFull` (modifier-less).
    CaptureFull,
    /// `CaptureScreen(u)` (modifier-less).
    CaptureScreen(u32),
    /// `Launcher`.
    Launcher,
    /// `Settings`.
    Settings,
    /// `Invoke(as)`: the argv tail (minus argv\[0\]).
    Invoke(Vec<String>),
}

impl WireCall {
    /// The wire member name (stable token; logging + tests).
    #[must_use]
    pub const fn member(&self) -> &'static str {
        match self {
            Self::Capture(_) => "Capture",
            Self::CaptureFull => "CaptureFull",
            Self::CaptureScreen(_) => "CaptureScreen",
            Self::Launcher => "Launcher",
            Self::Settings => "Settings",
            Self::Invoke(_) => "Invoke",
        }
    }

    /// Performs the call against the daemon's single object path.
    ///
    /// # Errors
    ///
    /// Any `zbus` transport or method-error reply.
    pub async fn send(&self, connection: &Connection) -> Result<(), zbus::Error> {
        match self {
            Self::Capture(options) => {
                connection
                    .call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "Capture", options)
                    .await?;
            }
            Self::CaptureFull => {
                connection
                    .call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "CaptureFull", &())
                    .await?;
            }
            Self::CaptureScreen(screen) => {
                connection
                    .call_method(
                        Some(SERVICE),
                        OBJECT_PATH,
                        Some(IFACE),
                        "CaptureScreen",
                        screen,
                    )
                    .await?;
            }
            Self::Launcher => {
                connection
                    .call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "Launcher", &())
                    .await?;
            }
            Self::Settings => {
                connection
                    .call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "Settings", &())
                    .await?;
            }
            Self::Invoke(argv) => {
                connection
                    .call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "Invoke", argv)
                    .await?;
            }
        }
        Ok(())
    }
}

/// Serializes the typed modifier bag into the `Capture(a{sv})` wire
/// vocabulary. Lean encoding: default-valued keys are omitted (the daemon's
/// [`CaptureRequest::from_vardict`] defaults missing keys, so the round-trip
/// is exact - property-tested in `tests/wire_roundtrip.rs`).
#[must_use]
pub fn capture_vardict(request: &CaptureRequest) -> HashMap<String, Value<'static>> {
    let mut options = HashMap::new();
    if request.delay_ms != 0 {
        options.insert(key("delay_ms"), Value::U32(request.delay_ms));
    }
    for (name, flag) in [
        ("instant", request.instant),
        ("no_edit", request.no_edit),
        ("copy", request.copy),
        ("pin", request.pin),
        ("upload", request.upload),
        ("raw", request.raw),
        ("print_geometry", request.print_geometry),
        ("hide_cursor", request.hide_cursor),
        ("last_region", request.last_region),
    ] {
        if flag {
            options.insert(key(name), Value::Bool(true));
        }
    }
    if let Some(output) = &request.output {
        options.insert(key("output"), Value::Str(Str::from(output.clone())));
    }
    if let Some(region) = &request.region {
        options.insert(key("region"), Value::Str(Str::from(region.clone())));
    }
    options
}

/// Maps a daemon-bound capture invocation onto its wire call. `argv_tail`
/// is the lossless `Invoke` channel for the forms the typed members cannot
/// carry (see the module table).
#[must_use]
pub fn capture_call(capture: &CaptureInvocation, argv_tail: &[String]) -> WireCall {
    match &capture.selection {
        CaptureSelection::Interactive => WireCall::Capture(capture_vardict(&capture.request)),
        CaptureSelection::Full => {
            if modifier_free(&capture.request) {
                WireCall::CaptureFull
            } else {
                invoke(argv_tail)
            }
        }
        CaptureSelection::Screen(ScreenSpec::Index(screen)) => {
            if modifier_free(&capture.request) {
                WireCall::CaptureScreen(*screen)
            } else {
                invoke(argv_tail)
            }
        }
        // At-cursor and connector targets have no typed member (the wire's
        // CaptureScreen carries a plain index): the argv channel is lossless.
        CaptureSelection::Screen(ScreenSpec::Cursor | ScreenSpec::Connector(_)) => {
            invoke(argv_tail)
        }
    }
}

/// True when no modifier rides along, so the modifier-less typed members
/// (`CaptureFull`/`CaptureScreen`) are lossless.
fn modifier_free(request: &CaptureRequest) -> bool {
    request == &CaptureRequest::default()
}

fn invoke(argv_tail: &[String]) -> WireCall {
    WireCall::Invoke(argv_tail.to_vec())
}

/// Owned wire key, asserted against the frozen vocabulary in debug builds
/// (the daemon ignores unknown keys with a warning; the subset property is
/// pinned by `tests/wire_roundtrip.rs`).
fn key(name: &'static str) -> String {
    debug_assert!(
        CAPTURE_OPTION_KEYS.contains(&name),
        "`{name}` is not in CAPTURE_OPTION_KEYS"
    );
    name.to_owned()
}

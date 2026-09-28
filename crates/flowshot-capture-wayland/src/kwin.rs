//! The `org.kde.KWin.ScreenShot2` `D-Bus` fast-path backend
//! ([`KwinScreenShot2Backend`], ladder rung 3).
//!
//! # Wire contract (pinned to the fetched upstream source)
//!
//! Verified against `KDE/kwin` `src/plugins/screenshot/screenshotdbusinterface2.cpp`
//! at commit `4788c6c176bbc4a6e98c46055026ae5734252ea2` (master, interface
//! `Version` 5, fetched 2026-09-25 - identical to master HEAD at fetch
//! time), plus `org.kde.KWin.ScreenShot2.xml` at the same commit:
//!
//! - bus name `org.kde.KWin.ScreenShot2`, object path
//!   `/org/kde/KWin/ScreenShot2`, interface `org.kde.KWin.ScreenShot2`;
//! - methods `CaptureActiveScreen(options, fd)`, `CaptureActiveWindow(options, fd)`,
//!   `CaptureArea(x:i, y:i, width:u, height:u, options, fd)`, and
//!   `CaptureInteractive(kind:u, options, fd)` (kind 0 = window, 1 = point;
//!   reserved for parity delegation, not used in the default flow);
//! - the CLIENT supplies the pipe fd as an INPUT argument; `KWin` sends the
//!   metadata vardict reply FIRST, then writes the RAW `QImage` bits
//!   (`constBits()`/`sizeInBytes()` - never an encoded image) into the pipe
//!   from a worker thread with poll-based backpressure, and closes the fd;
//! - the reply vardict carries `{type:"raw", format, width, height, stride,
//!   scale}` and MAY carry additive keys (`screen`, `windowId`, future
//!   additions) - the parser in `meta` ignores unknown keys;
//! - `QImage::Format` values map onto [`FrameFormat`] through the table in
//!   `meta::qimage_layout` (enum values pinned to `qt/qtbase`
//!   `src/gui/image/qimage.h` at commit
//!   `d793ab5031c9555c514ee2ab6b4ae3c06b9e90c5`);
//! - `KWin` rejects failures with named `org.kde.KWin.ScreenShot2.Error.*`
//!   `D-Bus` errors ([`KwinError::Cancelled`], [`KwinError::KwinReply`]).
//!
//! KDE additionally gates restricted interfaces through the caller's
//! desktop entry: the shipped `packaging/flowshot.desktop.in` carries
//! `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` (installed
//! by the packaging).
//!
//! # Capture semantics
//!
//! Per-output capture calls `CaptureArea` with the output's logical
//! rectangle and `native-resolution`, so `KWin` renders the area upright at
//! physical pixels (like the `grim`-based portal composite); the frame is
//! inverse-remapped into the shared native-orientation [`Frame`] contract
//! and guarded against the output's post-transform physical size.
//! Single-image captures (`CaptureActiveScreen`, `CaptureActiveWindow`,
//! `CaptureInteractive`) are not layout-anchored and return upright
//! [`OutputRef::Composite`] frames carrying the metadata's scale. Cursor
//! inclusion follows [`CaptureOpts::paint_cursor`] through the
//! `include-cursor` option; there is no cursor observation channel
//! ([`cursor_events`](flowshot_capture::CaptureBackend::cursor_events) is
//! `None` - a documented degradation, never a failure).
//!
//! # Verification class
//!
//! NOT live-testable on the Hyprland QA machine: build + clippy +
//! private-bus stub unit tests only, with the contract pinned to the
//! fetched upstream source above. Live KDE QA is deferred and NEVER
//! claimed.

pub(crate) mod error;
pub(crate) mod meta;
pub(crate) mod probe;
pub(crate) mod run;
#[cfg(test)]
pub(crate) mod stub;
#[cfg(test)]
mod tests;
pub(crate) mod wire;

use async_trait::async_trait;
use flowshot_capture::{
    BackendKind, CaptureBackend, CaptureError, CaptureOpts, CursorStream, Frame, PermissionResult,
};
use flowshot_core::geometry::{LogicalRect, OutputInfo};

pub use error::{DecodeError, KwinError};
pub use probe::{KwinAvailability, probe_kwin, probe_kwin_blocking};
use run::Selection;
use wire::{KwinBus, Request};

/// The `ScreenShot2` well-known bus name.
pub(crate) const SERVICE: &str = "org.kde.KWin.ScreenShot2";
/// The `ScreenShot2` object path.
pub(crate) const PATH: &str = "/org/kde/KWin/ScreenShot2";
/// The `ScreenShot2` interface name.
pub(crate) const IFACE: &str = "org.kde.KWin.ScreenShot2";

/// What `CaptureInteractive` lets the user pick (wire values pinned to the
/// fetched `KWin` source: `kind == 0` starts window selection, anything
/// else position selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KwinInteractiveKind {
    /// Interactive window selection (kind 0).
    Window,
    /// Interactive position selection, capturing the screen at the picked
    /// point (kind 1).
    Point,
}

impl KwinInteractiveKind {
    /// The `kind` wire value of the `CaptureInteractive` call.
    #[must_use]
    pub const fn wire_value(self) -> u32 {
        match self {
            Self::Window => 0,
            Self::Point => 1,
        }
    }
}

/// The `org.kde.KWin.ScreenShot2` capture backend (ladder rung 3).
///
/// The KDE fast path: `KWin` renders the capture itself and hands over raw
/// pixels through a client-supplied pipe - no compositor protocol, no
/// portal round-trip, no temp files. Stateless and cheap to construct;
/// every operation opens its own one-shot connections on a worker thread
/// (the crate's shared discipline).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KwinScreenShot2Backend;

impl KwinScreenShot2Backend {
    /// Creates the backend.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Captures the single output with this connector name (QA harnesses
    /// and single-output callers).
    ///
    /// # Errors
    ///
    /// [`CaptureError::Backend`] wrapping [`KwinError`] (`OutputNotFound`
    /// listing the available connectors) plus every error of
    /// [`CaptureBackend::capture_outputs`].
    pub async fn capture_output_named(&self, connector: &str) -> Result<Frame, CaptureError> {
        let name = connector.to_owned();
        let captured = crate::worker::spawn_worker("flowshot-kwin-named", move || {
            let selection = Selection::Named(name);
            run::run_capture(KwinBus::Session, &selection, true)
        })
        .await?;
        let mut frames = captured.frames;
        if frames.len() == 1 {
            return Ok(frames.remove(0));
        }
        Err(CaptureError::Backend {
            backend: BackendKind::KwinScreenShot2,
            source: KwinError::Internal("a named capture did not return exactly one frame").into(),
        })
    }

    /// Captures the currently active screen via `CaptureActiveScreen`
    /// (upright [`OutputRef::Composite`](flowshot_capture::OutputRef::Composite) frame
    /// at the metadata's scale).
    ///
    /// # Errors
    ///
    /// Every [`KwinError`] of the chain, lifted into [`CaptureError`].
    pub async fn capture_active_screen(&self) -> Result<Frame, CaptureError> {
        single(Request::ActiveScreen).await
    }

    /// Captures the currently active window via `CaptureActiveWindow`
    /// (upright [`OutputRef::Composite`](flowshot_capture::OutputRef::Composite) frame
    /// at the metadata's scale).
    ///
    /// # Errors
    ///
    /// Every [`KwinError`] of the chain, lifted into [`CaptureError`].
    pub async fn capture_active_window(&self) -> Result<Frame, CaptureError> {
        single(Request::ActiveWindow).await
    }

    /// Captures via `CaptureInteractive` - RESERVED for parity delegation
    /// (`KWin` draws its own picker); the default flow never calls this.
    /// The human picker lives inside the `D-Bus` call, which therefore runs
    /// on the 60s handshake budget instead of the machine-speed one.
    ///
    /// # Errors
    ///
    /// Every [`KwinError`] of the chain, lifted into [`CaptureError`];
    /// a dismissed picker surfaces as [`KwinError::Cancelled`].
    pub async fn capture_interactive(
        &self,
        kind: KwinInteractiveKind,
    ) -> Result<Frame, CaptureError> {
        single(Request::Interactive(kind)).await
    }
}

/// Runs one single-image capture on a worker thread (the stateless backend's
/// shared path).
async fn single(request: Request) -> Result<Frame, CaptureError> {
    crate::worker::spawn_worker("flowshot-kwin-single", move || {
        run::run_single(KwinBus::Session, request, true)
    })
    .await
}

#[async_trait]
impl CaptureBackend for KwinScreenShot2Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::KwinScreenShot2
    }

    async fn outputs(&self) -> Result<Vec<OutputInfo>, CaptureError> {
        crate::worker::spawn_worker("flowshot-kwin-outputs", run::collect_outputs).await
    }

    async fn capture_outputs(&self, opts: CaptureOpts) -> Result<Vec<Frame>, CaptureError> {
        let paint_cursor = opts.paint_cursor;
        let captured = crate::worker::spawn_worker("flowshot-kwin-capture", move || {
            run::run_capture(KwinBus::Session, &Selection::All, paint_cursor)
        })
        .await?;
        Ok(captured.frames)
    }

    async fn capture_region(&self, region: LogicalRect) -> Result<Frame, CaptureError> {
        let captured = crate::worker::spawn_worker("flowshot-kwin-region", move || {
            run::run_capture(KwinBus::Session, &Selection::All, false)
        })
        .await?;
        captured.stitch(BackendKind::KwinScreenShot2, region)
    }

    fn cursor_events(&self) -> Option<CursorStream> {
        // ScreenShot2 has no cursor-observation channel; cursor inclusion
        // is the include-capture option. None is the contract's
        // degradation, never a capture failure.
        None
    }

    async fn request_permission(&self) -> PermissionResult {
        // KWin has no per-request permission dialog on this interface: the
        // honest probe is service availability (name owner +
        // introspection), run on a worker so the budgeted blocking probe
        // never parks the caller's executor. Unavailable reports the safe
        // side.
        let available = crate::worker::spawn_worker("flowshot-kwin-permission", || {
            Ok::<bool, KwinError>(probe::probe_kwin_blocking().screenshot2)
        })
        .await
        .unwrap_or(false);
        if available {
            PermissionResult::NotRequired
        } else {
            PermissionResult::Denied
        }
    }
}

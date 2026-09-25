//! Wayland platform capture for `FlowShot`: session connection, registry
//! capability probing, and output enumeration.
//!
//! # Architecture: a dedicated capture thread with its own connection
//!
//! This crate runs its Wayland session on a dedicated thread
//! ([`CaptureThread`]) owning a private [`wayland_client::Connection`] and a
//! `calloop` event loop - deliberately separate from the UI toolkit's
//! connection. The UI (winit) never exposes its `wl_surface` or registry
//! state for capture use, so a screenshot tool cannot ride the toolkit's
//! connection even if it wanted to; single-purpose capture clients (`grim`,
//! libwayshot) own their connections for the same reason. A private
//! connection also isolates capture protocol traffic (large buffer events,
//! blocking round-trips) from UI frame pacing, and confines every blocking
//! wayland operation to the capture thread - foreign threads only exchange
//! bounded request/reply messages with it.
//!
//! # Capability probing
//!
//! The registry listener records every advertised global. Capture-relevant
//! protocols ([`ProtocolGlobals`]) are detected by interface name and mapped
//! onto the shared [`flowshot_capture::CapabilityProbe`]:
//! `ext-image-copy-capture-v1` (manager + per-output source manager) ->
//! `ExtImageCopyCapture`, `zwlr_screencopy_manager_v1` -> `WlrScreencopy`.
//! `KWin` and portal backends are `D-Bus` services, invisible to the
//! Wayland registry; the backend todos extend the probe with those checks.
//! `zwp_linux_dmabuf_v1` and the fractional-scale manager are detect-only
//! in v1.
//!
//! # Outputs
//!
//! [`CaptureThread::outputs`] returns [`flowshot_core::geometry::OutputInfo`]
//! assembled from `wl_output` events plus `zxdg_output_manager_v1` logical
//! geometry (connector name, description, logical position and size),
//! falling back to `wl_output`-only data when `xdg-output` is absent.
//!
//! # Capture backends
//!
//! [`IccBackend`] implements the shared
//! [`CaptureBackend`](flowshot_capture::CaptureBackend) trait for
//! `ext-image-copy-capture-v1`: one-shot per-output capture chains
//! (source -> session -> constraints -> `wl_shm` buffer -> frame ->
//! `ready`/`failed`) on dedicated short-lived connections, sequential
//! multi-output capture stitched per [`OutputLayout`](flowshot_core::geometry::OutputLayout)
//! for region captures ([`stitch`]), a 10 second per-phase timeout, and the
//! `Hyprland` permission-denial black-frame mapping ([`error::IccError`]).
//! v1 captures into `wl_shm` buffers only (no dma-buf). Cursor observation
//! ([`cursor`]) bridges `ext-image-copy-capture-v1` pointer-cursor sessions
//! onto the shared [`CursorStream`](flowshot_capture::CursorStream): a one-shot
//! global-logical [`IccBackend::cursor_pos`], a one-shot [`IccBackend::cursor_image`],
//! and the long-lived event stream from
//! [`cursor_events`](flowshot_capture::CaptureBackend::cursor_events).
//!
//! [`ScreencopyBackend`] is the `wlr-screencopy-unstable-v1` fallback (rung 2
//! of the ladder, for niri and pre-0.19 wlroots sessions without ICC). It
//! reuses the same one-shot connection model, deadline-bounded dispatch, and
//! `wl_shm` buffers, but the protocol delivers buffers in the output's native
//! orientation - so there is no inverse remap, only the renderer's `y_invert`
//! correction - and cursor inclusion is the `overlay_cursor` capture flag
//! (no cursor stream).
//!
//! [`PortalScreenshotBackend`] and [`PortalScreenCastBackend`] are the
//! `org.freedesktop.portal` rungs (5 and 4): universal fallbacks driven over
//! session `D-Bus` (ashpd) instead of compositor protocols. The Screenshot
//! portal returns one full-layout composite whose pixel space is DETECTED at
//! runtime against the enumerated layout, then cropped per output; the
//! `ScreenCast` portal opens a `PipeWire` remote whose first frame per stream
//! is the capture (single-frame, one-shot session, no restore token). Portal
//! availability is a bus check the Wayland registry cannot see:
//! [`probe_portals_blocking`] feeds [`PortalAvailability`] into the shared
//! [`CapabilityProbe`](flowshot_capture::CapabilityProbe) so the negotiation
//! ladder gates the portal rungs like the native ones.
//!
//! [`KwinScreenShot2Backend`] is the `org.kde.KWin.ScreenShot2` `D-Bus`
//! fast path (rung 3, KDE Plasma): `KWin` renders the capture itself and
//! writes RAW `QImage` bits into a client-supplied pipe fd, with the frame
//! metadata arriving as the reply vardict (contract pinned to the fetched
//! `KWin` source - see the `kwin` module). Availability is the session-bus
//! name-owner check plus interface introspection
//! ([`probe_kwin_blocking`] -> [`KwinAvailability`]). Verification class:
//! private-bus stub unit tests only on this machine; live KDE QA deferred.
//!
//! # Layered cursor position
//!
//! [`resolve_cursor_pos`] walks the draft F13 ladder and traces which layer
//! answered: the ICC pointer-cursor session one-shot
//! ([`CursorSource::IccCursorSession`]), the raw Hyprland IPC socket - never
//! a spawned `hyprctl` ([`CursorSource::HyprlandIpc`]), and the universal
//! overlay-first-motion handoff ([`CursorSource::AwaitFirstMotion`]) for
//! todos 16/18. [`cursor_capabilities`] exports the per-desktop capability
//! table for the docs build. Desktop detection ([`desktop`]) is the full
//! `XDG_CURRENT_DESKTOP` + `WAYLAND_DISPLAY` + `HYPRLAND_INSTANCE_SIGNATURE`
//! sniff with the honest [`DesktopEnv::Other`](flowshot_capture::DesktopEnv)
//! fallback.
//!
//! # v1 limitations
//!
//! - **Fractional scale**: `wl_output.scale` reports integers only.
//!   Per-output fractional values (`wp_fractional_scale_v1`) require a
//!   surface round-trip this crate does not perform yet; the manager's
//!   presence is detected and recorded, and the value stays the integer
//!   scale. (Hyprland additionally exposes `wlr-output-management` as an
//!   optional cross-check; the live QA session runs scale-1 outputs.)
//! - **`linux-dmabuf`**: detected, not used - v1 captures into `wl_shm`
//!   buffers.
//!
//! # Safety policy
//!
//! This is the one `unsafe`-exempt crate in the workspace (later zero-copy
//! buffer mapping may need it). All current code is safe Rust: zero
//! `unsafe` blocks - capture buffers are anonymous files (`memfd`) read
//! back with ordinary file I/O instead of memory mapping.

#![warn(missing_docs)]

mod denial;
mod dispatch;
mod hyprland_ipc;
mod icc;
mod kwin;
mod output;
mod portal;
mod screencopy;
mod session;
mod thread;
mod transform;
mod worker;

pub mod cursor;
pub mod desktop;
pub mod error;
pub mod globals;
pub mod resolve;
pub mod stitch;

pub use cursor::CursorImage;
pub use error::{ConnectError, IccError, ProbeError, ScreencopyError};
pub use globals::{Global, ProtocolGlobals};
pub use icc::IccBackend;
pub use kwin::{
    DecodeError, KwinAvailability, KwinError, KwinInteractiveKind, KwinScreenShot2Backend,
    probe_kwin, probe_kwin_blocking,
};
pub use portal::{
    PortalAvailability, PortalDenial, PortalErrorKind, PortalScreenCastBackend,
    PortalScreenCastError, PortalScreenshotBackend, PortalScreenshotError, probe_portals,
    probe_portals_blocking,
};
pub use resolve::{
    CURSOR_CAPABILITY_DESKTOPS, CursorCapabilities, CursorSource, cursor_capabilities,
    resolve_cursor_pos,
};
pub use screencopy::ScreencopyBackend;
pub use session::SessionSnapshot;
pub use stitch::CapturedOutputs;
pub use thread::CaptureThread;

//! X11 platform capture for `FlowShot`: session connection, capability
//! probing, and output enumeration for X11-only sessions (i3, Xfce,
//! Openbox).
//!
//! Sibling of `flowshot-capture-wayland` and the platform layer of the X11
//! plan: headless capture (`flowshot capture full | screen |
//! --region WxH[+X+Y]` with `--no-edit`) driven through the shared
//! [`flowshot_capture`] contracts - a
//! [`CapabilityProbe`](flowshot_capture::CapabilityProbe) for the negotiation
//! ladder and [`OutputInfo`](flowshot_core::geometry::OutputInfo) for
//! geometry. Phase B ships the interactive surfaces on X11 too: the
//! overlay, pins, and dialogs run through the daemon's session-routed
//! window customizer (not through this crate), and `capture last` /
//! `--region at-cursor` take the headless reroute in `flowshot-daemon`'s
//! `execute/overlay/reroute.rs`.
//!
//! # Modules
//!
//! - [`error`]: the typed [`X11Error`] family, convertible into
//!   [`CaptureError`](flowshot_capture::CaptureError) tagged
//!   [`BackendKind::X11`](flowshot_capture::BackendKind::X11).
//! - [`X11Connection`]: the connection helper - `x11rb`'s `RustConnection`
//!   plus the preferred screen's number and root window, which
//!   `x11rb::connect` reports once and the connection itself does not
//!   remember.
//! - [`probe_x11`]: the session probe feeding negotiation - `Some` when
//!   `DISPLAY` is set, the server is reachable, and RANDR >= 1.3 is
//!   present, and only when `WAYLAND_DISPLAY` is NOT set (an `XWayland`
//!   session is served by the Wayland rungs).
//!   [`X11Caps`] records the RANDR/XFIXES/MIT-SHM versions the capture path
//!   branches on.
//! - [`outputs`]: RANDR output enumeration (`GetMonitors` with a lit-CRTC
//!   fallback) mapped onto [`OutputInfo`](flowshot_core::geometry::OutputInfo)
//!   with per-output scale derivation and transform mapping.
//! - [`derive_scale`] and friends: the scale-derivation rules (plan decision
//!   #4) as pure functions.
//! - [`X11Backend`]: the [`CaptureBackend`](flowshot_capture::CaptureBackend)
//!   implementation - per-output root-window `GetImage` captures (MIT-SHM
//!   fd-passing fast path, plain-socket fallback), `OutputLayout` region
//!   stitching, one-shot cursor reads (`XQueryPointer` position, XFIXES
//!   image compositing), each operation bridged from a blocking worker thread
//!   into a runtime-agnostic future.
//! - [`cursor_pos`]: the one-shot cursor position in global logical space
//!   (the cursor-aware-preselect feed; X11 offers no client-visible cursor
//!   stream - out of scope by plan, the one-shot reads are the shipped
//!   path).
//! - [`stitch::to_rgba`]: the v1 pixel-format -> `RGBA` conversion shared by
//!   the region stitcher and the live-diagnostic examples.
//!
//! # Scale derivation
//!
//! X11 has no scale concept - the screen is a pixel-exact framebuffer - so
//! the per-output scale `FlowShot`'s logical space needs is *derived* and
//! documented as approximate: the session-global `Xft.dpi` resource wins
//! (`scale = dpi / 96`); without it, a RANDR physical-size heuristic
//! (`dpi = px / (mm / 25.4)`) applies per output. Results round to the
//! nearest 0.25 and clamp to `[1.0, 4.0]`.
//!
//! Geometry stays physical-pixels-first (ADR-001): X screen coordinates are
//! framebuffer pixels (`GetImage` returns them 1:1), the logical rect is the
//! physical rect divided by the output's own scale, and no scale is ever
//! averaged across outputs.
//!
//! # Safety policy
//!
//! `#![forbid(unsafe_code)]`: this crate is safe Rust end to end. The capture
//! path uses MIT-SHM fd-passing (`attach_fd`) with `memfd` readback through
//! ordinary file I/O rather than `SysV` `shmat`, which keeps it that way.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod backend;
mod capture;
mod connect;
mod cursor;
mod output;
mod probe;
mod scale;
mod worker;

pub mod error;
// The stitch algebra lives in the contract crate (shared with
// flowshot-capture-wayland); re-exported so `crate::stitch` keeps working.
pub use flowshot_capture::stitch;

pub use backend::X11Backend;
pub use connect::X11Connection;
pub use cursor::cursor_pos;
pub use error::X11Error;
pub use output::outputs;
pub use probe::{ExtensionVersion, X11Caps, probe_x11, query_caps};
pub use scale::{derive_scale, parse_xft_dpi, quantize_scale, scale_from_mm};

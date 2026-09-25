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
//! `unsafe` blocks.

#![warn(missing_docs)]

mod dispatch;
mod output;
mod session;
mod thread;
mod transform;

pub mod desktop;
pub mod error;
pub mod globals;

pub use error::{ConnectError, ProbeError};
pub use globals::{Global, ProtocolGlobals};
pub use session::SessionSnapshot;
pub use thread::CaptureThread;

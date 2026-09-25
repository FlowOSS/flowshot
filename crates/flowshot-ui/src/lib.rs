#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! `FlowShot` user-interface runtime: the multi-monitor screenshot overlay.
//!
//! # Architecture (plan todo 13)
//!
//! Two layers, split so the interesting logic never touches a GPU or a
//! display:
//!
//! - **Headless core** ([`OverlayCore`], [`InputRouter`]): the central input
//!   router maps `(window slot, surface-local physical px)` to global logical
//!   coordinates via [`flowshot_core::geometry::OutputLayout`]. This is the
//!   cross-monitor spanning enabler: during a drag, the compositor's implicit
//!   pointer grab keeps delivering motion to the origin window even while the
//!   cursor is logically over another monitor, so mapping is linear and
//!   *unclamped* - positions beyond the surface extend across the layout,
//!   and clamping is an explicit operation. The core also owns the shared
//!   cursor track, IME plumbing (always-on model, draft D7), and the
//!   Esc-closes-all teardown flag.
//! - **Window shell** ([`OverlayRuntime`]): winit 0.30 `ApplicationHandler`
//!   spawning one borderless-fullscreen, transparent, undecorated,
//!   always-on-top (best effort - advisory on Wayland) window per monitor,
//!   with the system cursor hidden in favor of a self-drawn crosshair
//!   (#1659-class fix), wgpu surfaces (`Bgra8UnormSrgb` preferred,
//!   premultiplied alpha, `Fifo` present mode), and a `RedrawRequested`
//!   frame scheduler that keeps an idle overlay at zero CPU.
//!
//! # Purity contract
//!
//! This crate imports no platform-specific APIs - no per-OS conditional
//! compilation, no protocol crates: winit and wgpu *are* the portable layer,
//! and platform code lives in the capture crates. The single environment
//! probe (`WAYLAND_DISPLAY`, see [`UiError::NoDisplayServer`]) is a portable
//! std call mandated by the todo-13 failure-path acceptance.
//!
//! # Test seam
//!
//! With feature `test-drive`, [`OverlayCore::inject_event`] (headless) and
//! `OverlayHandle::inject_event` (live loop) feed [`SyntheticInput`] through
//! the exact production routing path, so mouse paths are QA-able without
//! external injection tools.

mod adapter;
mod app;
mod crosshair;
mod gpu;
mod handler;
mod monitor;
mod runtime;
mod state;

pub mod error;
pub mod input;
pub mod router;

pub use error::UiError;
pub use input::{Action, ImeStatus, InputEvent, RouteReport, SyntheticInput};
pub use router::{InputRouter, WindowSlot};
pub use runtime::{OverlayHandle, OverlayRuntime};
pub use state::{CursorTrack, OverlayCore};

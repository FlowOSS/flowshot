#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Platform-agnostic screen capture contracts for `FlowShot`.
//!
//! This crate owns the cross-crate capture vocabulary and the shared
//! platform-free capture algebra:
//!
//! - [`CaptureBackend`]: the async trait every platform implementation
//!   satisfies (frames, outputs, cursor stream, permission).
//! - [`Frame`], [`FrameBuffer`], [`FrameFormat`]: CPU-side pixel delivery with
//!   physical-pixels-first placement metadata (per-output scale and
//!   transform, never an averaged scale).
//! - [`CapabilityProbe`] + [`negotiate()`]: the backend ladder
//!   (`ext-image-copy-capture` -> `wlr-screencopy` -> `KWin ScreenShot2` ->
//!   portal `ScreenCast` -> portal `Screenshot` -> `X11`), filtered by what
//!   the session actually offers, with a config `force_backend` override.
//! - [`MockBackend`]: a fixture-driven implementation for downstream tests.
//! - [`stitch`]: the region-stitch algebra both platform crates consume
//!   (the executable form of the shared region-capture contract).
//!
//! Geometry types ([`flowshot_core::geometry::OutputInfo`], rectangles, transforms) are reused from
//! [`flowshot_core::geometry`], never duplicated.
//!
//! # Purity
//!
//! No display-system, windowing, or platform code lives here - this crate
//! names protocols only in diagnostics. Platform implementations live in
//! dedicated crates that depend on this one.

pub mod backend;
pub mod cursor;
pub mod error;
pub mod frame;
pub mod kind;
pub mod mock;
pub mod negotiate;
pub mod stitch;

pub use backend::{CaptureBackend, CaptureOpts, PermissionResult};
pub use cursor::{CursorEvent, CursorStream};
pub use error::CaptureError;
pub use frame::{Frame, FrameBuffer, FrameFormat, OutputRef};
pub use kind::BackendKind;
pub use mock::MockBackend;
pub use negotiate::{CapabilityProbe, DesktopEnv, NEGOTIATION_LADDER, negotiate};

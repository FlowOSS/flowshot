//! wgpu device management and per-window surface lifecycle.
//!
//! One policy module per decision class: `instance` (backends + the
//! explicit validation flags), `context` (adapter/device selection), and
//! `surface` (per-window surface configuration).

mod context;
mod instance;
mod surface;

pub use context::GpuContext;
pub use instance::{OVERLAY_BACKENDS, new_instance};
pub use surface::{configure_opaque_surface, configure_overlay_surface};

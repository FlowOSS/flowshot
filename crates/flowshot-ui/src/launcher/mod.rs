//! The capture launcher dialog: a small egui window on the
//! embedded stack (`crate::egui_host`) - the Flameshot mode-9
//! manual-coordinate capability, reached via `flowshot capture --dialog` or
//! the tray's `Capture Launcher` item (both dispatch the
//! daemon's `Launcher` method).
//!
//! # Surface (capability parity only)
//!
//! - manual geometry entry `WxH+X+Y` with inline validation (the CLI's
//!   `--region` grammar: offsets optional and independently signed),
//! - a monitor dropdown fed by the live output probe through the
//!   [`MonitorProbe`] seam,
//! - a delay spinner (ms),
//! - Capture / Cancel.
//!
//! # Dispatch seam
//!
//! Capture emits the typed [`LauncherRequest`] through the
//! [`LaunchCallback`] seam and closes; the binary layer maps it
//! onto the daemon's command vocabulary - `Region` -> `Capture` (region
//! semantics: `CaptureRequest.region` carries [`RegionGeometry::to_token`]),
//! `Screen` -> `CaptureScreen(index)`. Cancel closes without a request (the
//! CLI's user-cancelled exit class). The lib stays pure: no probe, no bus,
//! no capture backend inside this crate.

mod app;
mod frame;
mod model;
mod offscreen;
mod options;
mod request;
mod strings;
mod ui;
mod window;

#[cfg(test)]
mod tests;

pub use model::{LauncherModel, MonitorEntry, Target};
pub use offscreen::render_offscreen;
pub use options::{LaunchCallback, LauncherWindowOptions, MonitorProbe};
pub use request::{GeometryIssue, LauncherRequest, RegionGeometry};
pub use ui::LauncherAction;
pub use window::{LauncherEvent, LauncherHandle, LauncherWindow};

#[cfg(feature = "test-drive")]
pub use window::LauncherInput;

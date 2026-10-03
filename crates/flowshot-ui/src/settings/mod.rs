//! The settings surface: egui embedded in OUR wgpu renderer
//! (draft D8(b), the Ruffle pattern) - one of the recorded exceptions to
//! the winit+wgpu+cosmic-text stack (the others are the launcher dialog
//! and the first-launch consent dialog); the shared embedded plumbing
//! lives in `crate::egui_host`, and the overlay and editor never touch
//! egui.
//!
//! # Layout
//!
//! - [`SettingsModel`]: the typed edit state over the
//!   [`Config`](flowshot_core::config::Config) - resilient load (corrupt
//!   file -> defaults + banner), validation (undo limit 0..=999, JPEG
//!   quality 1..=100, `#RRGGBB` colors), reset preserving `config_version`,
//!   and the [`ToolShortcuts`](crate::editor::ToolShortcuts) rebind
//!   seams.
//! - `crate::egui_host::theme`: `egui::Style` projected FROM the design
//!   tokens + `[ui]` config (live re-themed every frame - the
//!   tokens-driven live-apply), with the vendored Inter weights as the
//!   typographic hierarchy, and `settings_style` adding the form-grid
//!   geometry (uniform control height, solid reserved scrollbar).
//! - [`FormMetrics`]: the pure two-column grid math (fixed-width
//!   right-aligned label column, gutter, control column at
//!   [`FormMetrics::control_x`]) every row/card is laid out on - the same
//!   numbers the offscreen pixel asserts consume.
//! - `tabs`: the four F12 tabs (General / Interface / Filename Editor /
//!   Shortcuts) as immediate-mode projections of the model, rendered as
//!   token-surface section cards over the grid, with a pill tab bar and a
//!   docked bottom action bar.
//! - [`render_offscreen`]: the headless settings-frame QA path (drives the
//!   shared surface into a readback-able texture; renders a warm frame
//!   first so egui's frame-lagged scroll clip/bar state is settled).
//! - [`SettingsWindow`]: the winit runtime for the normal `flowshot-settings`
//!   window; Apply = validation gate + migration-safe
//!   [`Config::save`](flowshot_core::config::Config::save) +
//!   [`AppliedCallback`] (the binary layer's `ConfigChanged` `D-Bus`
//!   emitter - daemon-owned).

pub(crate) mod fields;
pub(crate) mod form;
mod layout;
mod model;
mod strings;
mod surface;
mod tabs;
mod window;

#[cfg(test)]
mod tests;

pub use crate::egui_host::ClipboardBridge;
pub use crate::egui_host::theme::{
    Surfaces, ThemeMode, fonts, parse_hex_rgb, settings_style, style, surfaces_for,
};
pub use layout::FormMetrics;
pub use model::{
    Banner, FieldIssue, RecorderTarget, SettingsModel, Tab, ThemeChoice, UNDO_LIMIT_MAX,
};
pub use surface::{OFFSCREEN_FORMAT, render_offscreen};
pub use tabs::preview_filename;
pub use tabs::{FrameAction, TabContext};
pub use window::{
    AppliedCallback, PathPicker, SettingsEvent, SettingsHandle, SettingsWindow,
    SettingsWindowOptions,
};

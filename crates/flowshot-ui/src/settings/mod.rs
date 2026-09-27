//! The settings surface (plan todo 36): egui embedded in OUR wgpu renderer
//! (draft D8(b), the Ruffle pattern) - one of the two recorded exceptions to
//! the winit+wgpu+cosmic-text stack (the other is the todo-37 launcher
//! dialog); the shared embedded plumbing lives in [`crate::egui_host`], and
//! the overlay and editor never touch egui.
//!
//! # Layout
//!
//! - [`SettingsModel`]: the typed edit state over the todo-2
//!   [`Config`](flowshot_core::config::Config) - resilient load (corrupt
//!   file -> defaults + banner), validation (undo limit 0..=999, JPEG
//!   quality 1..=100, `#RRGGBB` colors), reset preserving `config_version`,
//!   and the todo-25 [`ToolShortcuts`](crate::editor::ToolShortcuts) rebind
//!   seams.
//! - [`crate::egui_host::theme`]: `egui::Style` projected FROM the design
//!   tokens + `[ui]` config (live re-themed every frame - the
//!   tokens-driven live-apply), with the vendored Inter as the proportional
//!   face.
//! - `tabs`: the four F12 tabs (General / Interface / Filename Editor /
//!   Shortcuts) as immediate-mode projections of the model.
//! - [`render_offscreen`]: the headless settings-frame QA path (drives the
//!   shared surface into a readback-able texture).
//! - [`SettingsWindow`]: the winit runtime for the normal `flowshot-settings`
//!   window; Apply = validation gate + migration-safe
//!   [`Config::save`](flowshot_core::config::Config::save) +
//!   [`AppliedCallback`] (the binary layer's `ConfigChanged` `D-Bus`
//!   emitter, todo 32).

mod model;
mod strings;
mod surface;
mod tabs;
mod window;

#[cfg(test)]
mod tests;

pub use crate::egui_host::ClipboardBridge;
pub use crate::egui_host::theme::{ThemeMode, fonts, parse_hex_rgb, style};
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

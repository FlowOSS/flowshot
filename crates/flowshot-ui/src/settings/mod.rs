//! The settings surface (plan todo 36): egui embedded in OUR wgpu renderer
//! (draft D8(b), the Ruffle pattern) - the single recorded exception to the
//! winit+wgpu+cosmic-text stack, confined to this window (the overlay and
//! editor never touch egui).
//!
//! # Layout
//!
//! - [`SettingsModel`]: the typed edit state over the todo-2
//!   [`Config`](flowshot_core::config::Config) - resilient load (corrupt
//!   file -> defaults + banner), validation (undo limit 0..=999, JPEG
//!   quality 1..=100, `#RRGGBB` colors), reset preserving `config_version`,
//!   and the todo-25 [`ToolShortcuts`](crate::editor::ToolShortcuts) rebind
//!   seams.
//! - [`theme`]: `egui::Style` projected FROM the design tokens + `[ui]`
//!   config (live re-themed every frame - the tokens-driven live-apply),
//!   with the vendored Inter as the proportional face.
//! - `tabs`: the four F12 tabs (General / Interface / Filename Editor /
//!   Shortcuts) as immediate-mode projections of the model.
//! - [`SettingsSurface`]: the egui+wgpu frame plumbing, window-agnostic;
//!   [`render_offscreen`] drives it headlessly into a readback-able texture
//!   (the no-visible-windows QA path).
//! - [`SettingsWindow`]: the winit runtime for the normal `flowshot-settings`
//!   window; Apply = validation gate + migration-safe
//!   [`Config::save`](flowshot_core::config::Config::save) +
//!   [`AppliedCallback`] (the binary layer's `ConfigChanged` `D-Bus`
//!   emitter, todo 32).
//!
//! # Version constraint (recorded decision, plan D8(b) fallback)
//!
//! egui 0.28.1 + egui-wgpu 0.28.1 pair with the workspace wgpu 0.20 pin
//! (egui-wgpu 0.28 requires wgpu ^0.20.0 - cached-registry-verified).
//! egui-winit is NOT used: its 0.28 release requires winit ^0.29 while the
//! workspace pins winit 0.30, and every newer egui requires wgpu 22+ (the
//! plan forbids bumping wgpu). Per the plan's recorded fallback, `input`
//! feeds egui manually from our winit 0.30 events.

mod input;
mod keymap;
mod model;
mod strings;
mod surface;
mod tabs;
mod theme;
mod window;

#[cfg(test)]
mod tests;

pub use input::ClipboardBridge;
pub use model::{
    Banner, FieldIssue, RecorderTarget, SettingsModel, Tab, ThemeChoice, UNDO_LIMIT_MAX,
};
pub use surface::{OFFSCREEN_FORMAT, SettingsSurface, render_offscreen};
pub use tabs::preview_filename;
pub use tabs::{FrameAction, TabContext};
pub use theme::{ThemeMode, fonts, parse_hex_rgb, style};
pub use window::{
    AppliedCallback, PathPicker, SettingsEvent, SettingsHandle, SettingsWindow,
    SettingsWindowOptions,
};

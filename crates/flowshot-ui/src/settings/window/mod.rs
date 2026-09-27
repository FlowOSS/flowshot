//! The settings window runtime: a NORMAL winit window (decorated, resizable,
//! `app_id=flowshot-settings` via the binary-layer customizer seam) hosting
//! the embedded egui surface on our own wgpu stack - the todo-36 shell.
//!
//! Seams (the lib stays pure; the binary layer owns platform services):
//! - [`SettingsWindowOptions::window_customizer`] applies the Wayland
//!   `app_id` (the `pins::WindowCustomizer` precedent).
//! - [`SettingsWindowOptions::on_applied`] fires after a successful Apply
//!   write - the binary layer emits the `ConfigChanged` `D-Bus` signal
//!   (todo 32) and re-projects live surfaces (`EditorState::configure`,
//!   chrome, daemon toggles).
//! - [`PathPicker`] hosts the rfd save-path dialog; [`ClipboardBridge`]
//!   hosts Ctrl+C/V.
//! - The system dark/light preference (ashpd Settings portal) arrives
//!   resolved via [`SettingsWindowOptions::system_theme`] and can be
//!   updated live through [`SettingsHandle::set_system_theme`].
//!
//! Apply = validation gate + migration-safe
//! [`Config::save`](flowshot_core::config::Config::save) + callback; a
//! failed write surfaces as a banner, never a panic.
//!
//! [`ClipboardBridge`]: super::input::ClipboardBridge

mod app;
mod frame;
mod options;
mod runtime;

pub use options::{AppliedCallback, PathPicker, SettingsWindowOptions};
pub use runtime::{SettingsEvent, SettingsHandle, SettingsWindow};

//! The embedded egui stack (draft D8(b), the Ruffle pattern): an
//! [`egui::Context`] + [`egui_wgpu::Renderer`] pair driven by OUR winit/wgpu
//! runtime - egui is a guest in this crate's renderer, never a second
//! windowing stack (a MUST-NOT).
//!
//! Shared by the sanctioned egui windows: the settings surface
//! ([`crate::settings`]), the capture launcher dialog
//! ([`crate::launcher`]), and the first-launch telemetry consent dialog
//! ([`crate::consent`]). The overlay and editor never touch egui.
//!
//! egui-winit is deliberately absent (see the crate manifest note):
//! [`input::InputState`] feeds the context from raw winit 0.30 events
//! instead ([`keymap`] mirrors egui-winit's conversion tables). The same
//! split keeps [`EguiSurface`] renderable WITHOUT any window (the offscreen
//! QA paths).
//!
//! # Version constraint (recorded decision, plan D8(b) fallback)
//!
//! egui 0.36 + egui-wgpu 0.36 pair with the workspace wgpu 30 pin and with
//! winit 0.30.13 (crates.io-verified; winit 0.31 is still beta and stays
//! deferred). egui-winit is still NOT a dependency even though its 0.36
//! release now pairs with winit 0.30: the hand-feed bridge is the tested
//! surface (recorded migration decision).

pub(crate) mod input;
pub(crate) mod keymap;
mod present;
mod surface;
pub(crate) mod theme;

pub(crate) use present::{cursor_icon, points, scale_to_ppp};
pub(crate) use surface::EguiSurface;

// The clipboard seam and the resolved dark/light preference are part of the
// egui windows' public option bags (the binary layer wires them); the rest
// of the theme vocabulary is reached through `crate::egui_host::theme`.
pub use input::ClipboardBridge;
pub use theme::ThemeMode;

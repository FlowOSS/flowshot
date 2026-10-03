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
//! egui-winit is deliberately absent (version dead end - see the crate
//! manifest note): [`input::InputState`] feeds the context from raw winit
//! 0.30 events instead ([`keymap`] mirrors egui-winit 0.28's conversion
//! tables). The same split keeps [`EguiSurface`] renderable WITHOUT any
//! window (the offscreen QA paths).
//!
//! # Version constraint (recorded decision, plan D8(b) fallback)
//!
//! egui 0.28.1 + egui-wgpu 0.28.1 pair with the workspace wgpu 0.20 pin
//! (egui-wgpu 0.28 requires wgpu ^0.20.0 - cached-registry-verified).
//! egui-winit is NOT used: its 0.28 release requires winit ^0.29 while the
//! workspace pins winit 0.30, and every newer egui requires wgpu 22+ (the
//! plan forbids bumping wgpu).

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

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
//! - **2D renderer** ([`render::Renderer`], plan todo 14): the batched
//!   draw-command consumer the shell and every later layer render through -
//!   lyon vector tessellation, cosmic-text glyph atlas, image quads, dim /
//!   shadow / rounded-clip effects, all token-driven.
//! - **Selection engine** ([`SelectionState`], plan todo 16): the full
//!   Flameshot selection behavior spec (draft F27) as a pure state machine -
//!   drag-create behind a 3px manhattan threshold, 8 token-derived handles,
//!   Shift mirror / Ctrl aspect resize, 1px keyboard nudges, the 10x10
//!   minimum, layout clamping, the geometry HUD, and the six-stage Esc
//!   cascade. The selection lives in global logical space, so ONE rect
//!   spans every monitor (#4894 restored); the shell feeds it through
//!   [`OverlayCore`] and paints it per window.
//! - **Editor tool framework** ([`EditorState`], plan todo 20): the scene
//!   bridge every annotation tool (todos 21-27) plugs into - the [`Tool`]
//!   lifecycle (drawStart/Move/End/pressed) with the F27 [`EditorContext`],
//!   the exact F27 event-routing priority (picker > right-click > active
//!   tool > edit commit > object select > selection engine), per-tool size
//!   dispatch with the digit/wheel adjusters, scene commits as single undo
//!   units, and the real producers of the Esc cascade's tool/object stages.
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
pub mod gpu;
mod handler;
mod monitor;
mod runtime;
mod state;
mod surface;

pub mod backdrop;
pub mod editor;
pub mod error;
pub mod input;
pub mod pins;
pub mod render;
pub mod router;
pub mod selection;
pub mod widgets;

pub use backdrop::{
    Backdrop, BackdropOptions, CursorSprite, FrozenCapture, MissingFrame, PlacedCursor,
    backdrop_texture_id, capture_frozen, cursor_texture_id,
};
pub use editor::{
    EditorContext, EditorEffect, EditorEnv, EditorState, EditorTools, EditorUpdate, EditorView,
    FramePixels, Tool, ToolCursor, ToolKind, ToolRegistry, ToolShortcuts, register_shape_tools,
};
pub use error::UiError;
pub use input::{Action, ImeStatus, InputEvent, RouteReport, SyntheticInput};
pub use router::{InputRouter, WindowSlot};
pub use runtime::{OverlayHandle, OverlayRuntime};
pub use selection::{
    CascadeState, Effect, EscStep, Handle, HitZone, HudPosition, HudView, SelectionConfig,
    SelectionEnv, SelectionMetrics, SelectionState, SelectionUpdate,
};
pub use state::{CursorTrack, OverlayCore};

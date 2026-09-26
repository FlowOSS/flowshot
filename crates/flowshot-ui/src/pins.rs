//! Pin-to-screen windows with zoom-to-cursor (plan todo 30, draft F27 pin
//! spec - clean-room reimplementation of the Flameshot `pinwidget` behavior
//! constants).
//!
//! # Behavior spec (F27 BORROW / MODIFIED / AVOID)
//!
//! - Frameless translucent always-on-top window per pin; drag delegates to
//!   the compositor (`drag_window()` = the `startSystemMove` BORROW,
//!   Wayland-correct). Always-on-top is ADVISORY on Wayland (xdg-shell has
//!   no stacking protocol; Hyprland `staysontop`/`float` rule snippets ship
//!   in todos 39/40).
//! - Wheel zoom: `STEP = 0.03`, accumulate-then-commit (one 120-unit notch
//!   commits one multiplicative step), ANCHORED AT CURSOR - Flameshot's
//!   zoom-from-center is the AVOID this module exists to fix. Pinch mirrors
//!   the wheel via two-finger touch tracking (winit 0.30 exposes no
//!   touchpad pinch gesture on Wayland - documented degradation).
//! - Rotate 90 degrees: BUFFER transform (core `Transform` remap), menu +
//!   bindable keys (`R` / `Shift+R` defaults; todo 36 makes them
//!   configurable).
//! - Opacity: keys `0-9` map to `1.0..0.1` ABSOLUTE (F27 table), context
//!   menu moves `+/-0.1`; stored as integer tenths (no float drift).
//! - Right-click context menu via the todo-19 widget layer: copy / save /
//!   rotate right / rotate left / increase opacity / decrease opacity /
//!   close (Flameshot item order). Copy/save flow through the
//!   [`PinActionSink`] callback trait into the todo-28/29 action modules.
//! - Pin zoom is ALWAYS antialiased (linear texture sampling; the
//!   `antialiasingPinZoom` toggle is DROPPED per Amendment #3).
//! - Geometry is PHYSICAL-FIRST (the #4920 root-cause fix, F27 AVOID of
//!   Flameshot's fractional-DPR math in pinwidget L62-84): the image
//!   buffer is physical pixels, zoom multiplies it directly, and the
//!   window's device scale factor converts exactly once - into the logical
//!   `MARGIN = 7` shadow frame (`BLUR_RADIUS = 14 = 2 * MARGIN`).
//! - Window size = `image * scale + 2 * margin`, clamped to the screen
//!   (fit wins over the `MIN_SIZE = 100` floor on degenerate screens) and
//!   floored at `MIN_SIZE` per axis (`[pin].min_size` config).
//! - NO layer-shell for pins (v1 overlay policy).
//!
//! # Crate placement (task decision, recorded in evidence)
//!
//! Pins are GPU-rendered winit windows, so the WINDOW + rendering + pure
//! behavior state machine live HERE (`flowshot-ui::pins`); the pin
//! REGISTRY (the todo-32 "pins alive" persistence reason) and the
//! copy/save action functions live in `flowshot-actions::pin` on the
//! todo-28/29 seams. The crates share NO types (ui purity gate forbids
//! depending on the Wayland-native actions crate); the binary layer (todo
//! 35/32) bridges them by implementing [`PinActionSink`] - the QA example
//! `examples/pin_window.rs` is the reference composition.
//!
//! # Wayland resize mechanics (live-probed on Hyprland 0.56.2, 2026-09-26)
//!
//! Hyprland sends all four `XDG_TOPLEVEL_STATE_TILED_*` states on EVERY
//! toplevel, so winit classifies even floating windows as compositor-sized
//! and `Window::request_inner_size` is a permanent no-op. The working
//! client-resize path pins `set_min_inner_size == set_max_inner_size ==
//! target`, which makes Hyprland reconfigure the floating window to
//! exactly that extent (growth and shrink verified). Hyprland keeps the
//! window CENTER invariant across that resize - the zoom-to-cursor math in
//! [`zoom`] compensates for it (and for the top-left policy of other
//! compositors) so the image point under the cursor stays under the
//! cursor. Floating placement itself needs a compositor window rule
//! (`float, class:flowshot-pin` - todo 39/40 snippet); without one,
//! Hyprland tiles the pin.

mod effects;
mod event;
mod image;
mod interact;
mod menu;
mod ops;
mod paint;
mod pinch;
mod runtime;
mod shell;
mod sink;
mod spawn;
mod state;
mod strings;

pub mod spec;
pub mod zoom;

#[cfg(test)]
mod tests;

use crate::render::TextureId;

pub use event::{PinEffect, PinInput};
pub use image::{PinImage, Rotation};
pub use menu::{MenuAction, PinMenu};
pub use paint::{frame_list, image_rect};
pub use runtime::{PinHandle, PinRuntime, PinSpec, WindowCustomizer};
pub use sink::{PinActionSink, PinId, PinSnapshot};
pub use state::{PinBehavior, PinState};

/// The pin image texture slot (each pin window owns its renderer, so the
/// id only needs to be stable within one pin; consumer-issued scheme of
/// todo 15: below the cursor slot `1 << 15`). Public for the offscreen
/// verify harness, which uploads through the same id [`frame_list`] draws.
pub const TEXTURE_ID: TextureId = TextureId::new(1 << 14);

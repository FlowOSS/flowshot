//! The editor chrome.
//!
//! This module provides the UI overlays for the editor:
//! - Toolbar: the tool strip anchored below the selection (flipping above
//!   near the bottom edge), button order = the `[ui].toolbar_buttons`
//!   config list, icons from the icon atlas.
//! - Color Wheel: the circular palette popover (F27 colorpicker spec) the
//!   editor's right-click effect opens.
//! - Side Panel: the Space-toggled tool-options panel and layer list
//!   (the z-order model: click = select, drag = reorder).
//! - HUD: size-notifier surface.
//!
//! [`ChromeState`] owns the four and the input contracts: the route funnel
//! consults it BEFORE the F27 editor chain (widget parity), the Esc
//! cascade's panel/picker stages (3/5) mirror its visibility flags, and the
//! draw-color sink is the F27 TOML-persistence seam the binary layer
//! installs.

#![allow(
    clippy::cast_precision_loss,
    clippy::too_many_arguments,
    clippy::match_same_arms
)]

pub mod aids;
pub mod color_wheel;
pub mod hud;
pub mod motion;
pub mod side_panel;
pub mod state;
pub mod toolbar;
pub mod tooltips;

pub use color_wheel::{BUTTON_BASE_SIZE, ColorWheel, SELECTED_RING, SWATCH_SIZE, WheelLayout};
pub use hud::SizeHud;
pub use motion::ChromeMotion;
pub use side_panel::{SidePanelLayout, size_label};
pub use state::{ChromeState, DrawColorSink};
pub use toolbar::{Toolbar, ToolbarButton, icon_for_tool};
pub use tooltips::{button_tooltip, chord, tool_blurb};

#[cfg(test)]
mod tests;

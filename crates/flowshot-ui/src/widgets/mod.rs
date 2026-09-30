//! Token-driven micro-widget layer.
//!
//! This module provides the design system primitives: buttons, sliders,
//! popovers, etc. All visual properties (colors, spacing, radii) are derived
//! from `flowshot_core::tokens::DesignTokens`. No hardcoded colors or sizes
//! are allowed.

#![allow(clippy::cast_precision_loss, clippy::match_same_arms)]

pub mod button;
pub mod chip;
pub mod context_menu;
pub mod focus;
pub mod icon_button;
pub mod popover;
pub mod scroll_area;
pub mod separator;
pub mod slider;
pub mod toggle;
pub mod tooltip;

pub use button::{Button, ButtonState};
pub use chip::AidChip;
pub use context_menu::{ContextMenu, ContextMenuEntry};
pub use focus::{FocusRing, FocusTraversal};
pub use icon_button::IconButton;
pub use popover::Popover;
pub use scroll_area::ScrollArea;
pub use separator::Separator;
pub use slider::Slider;
pub use toggle::Toggle;
pub use tooltip::{TOOLTIP_DELAY, Tooltip, TooltipClock};

/// Hover wash opacity (0-255): one ink (the contrast token) at a low alpha
/// ramp - the design system's state language (hover < press < solid).
pub const HOVER_WASH_ALPHA: u8 = 20;
/// Press wash opacity (0-255), the next ramp step after hover.
pub const PRESS_WASH_ALPHA: u8 = 40;
/// Disabled ink opacity (0-255).
pub const DISABLED_INK_ALPHA: u8 = 100;
/// Idle button ink opacity (0-255).
pub const IDLE_INK_ALPHA: u8 = 200;
/// Keyboard focus-ring stroke width in logical px.
pub const FOCUS_RING_WIDTH: f32 = 2.0;

/// The animated wash level mapped to its alpha byte: level 1 =
/// hover, level 2 = press; fractional levels fade continuously between the
/// ramp steps.
#[must_use]
pub fn wash_alpha(level: f64) -> u8 {
    let alpha = f64::from(HOVER_WASH_ALPHA) * level.max(0.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped into [0, 255] before the cast"
    )]
    let byte = alpha.min(255.0).round() as u8;
    byte
}

/// The texture ID for the icon atlas.
pub const ICON_ATLAS_ID: crate::render::TextureId = crate::render::TextureId::new(1);

// Re-export the generated icons
#[allow(missing_docs)]
pub mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}
pub use icons::*;

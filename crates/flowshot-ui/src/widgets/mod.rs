//! Token-driven micro-widget layer (plan todo 19).
//!
//! This module provides the design system primitives: buttons, sliders,
//! popovers, etc. All visual properties (colors, spacing, radii) are derived
//! from `flowshot_core::tokens::DesignTokens`. No hardcoded colors or sizes
//! are allowed.

#![allow(clippy::cast_precision_loss, clippy::match_same_arms)]

pub mod button;
pub mod context_menu;
pub mod focus;
pub mod icon_button;
pub mod popover;
pub mod scroll_area;
pub mod separator;
pub mod slider;
pub mod toggle;

pub use button::{Button, ButtonState};
pub use context_menu::{ContextMenu, ContextMenuEntry};
pub use focus::{FocusRing, FocusTraversal};
pub use icon_button::IconButton;
pub use popover::Popover;
pub use scroll_area::ScrollArea;
pub use separator::Separator;
pub use slider::Slider;
pub use toggle::Toggle;

/// The texture ID for the icon atlas.
pub const ICON_ATLAS_ID: crate::render::TextureId = crate::render::TextureId::new(1);

// Re-export the generated icons
#[allow(missing_docs)]
pub mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}
pub use icons::*;

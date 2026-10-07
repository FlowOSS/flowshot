//! The pure pin state machine.
//!
//! [`PinState`] holds everything a pin window knows - zoom scale, image
//! offset (the zoom-to-cursor anchor), rotation, opacity tenths, wheel
//! accumulator, menu and pinch state - and turns [`PinInput`]s into
//! [`PinEffect`]s. No windowing, no GPU: the whole F27 pin behavior spec is
//! unit-testable headlessly through [`PinState::on_input`] (the
//! inject-seam architecture), and the `test-drive` feature feeds the same
//! path on a live window.
//!
//! Geometry is PHYSICAL-FIRST (F27 AVOID of Flameshot's fractional-DPR math,
//! the #4920 root cause): the image buffer is in physical pixels, the scale
//! factor multiplies it directly, and the window's device scale factor is
//! converted exactly once - into the logical [`super::spec::MARGIN`] frame.

use std::time::Instant;

use flowshot_core::tokens::DesignTokens;
use winit::keyboard::ModifiersState;

use super::event::{PinEffect, PinInput};
use super::image::Rotation;
use super::menu::PinMenu;
use super::pinch::PinchTracker;
use super::spec::{MARGIN, MIN_SIZE};
use super::zoom::{
    clamp_scale, content_size, scale_bounds, window_size, ResizeAnchor, ScaleBounds,
};

/// Behavior configuration of one pin (the `[pin]` config group plus the
/// session-routed resize policy: anchor + client-driven flag).
#[derive(Debug, Clone, PartialEq)]
pub struct PinBehavior {
    /// Minimum content size per axis in physical px (`[pin].min_size`).
    pub min_size: f64,
    /// The compositor's floating-resize anchor policy (see
    /// [`super::zoom::ResizeAnchor`] for the live probe).
    pub anchor: ResizeAnchor,
    /// Client-driven resize (X11-only; session-routed by the daemon's
    /// `execute/window.rs`): after pinning the min/max hints the shell
    /// additionally issues `request_inner_size`, because on X11 a
    /// `WM_NORMAL_HINTS` update alone reconfigures nothing - the client's
    /// `ConfigureRequest` does. Must stay false on Wayland: winit resizes
    /// stateless (floating) windows client-side there WITHOUT emitting
    /// `Resized`, desyncing the render surface from the window geometry.
    pub client_resize: bool,
    /// Design tokens (menu layout, shadow, typography).
    pub tokens: DesignTokens,
    /// Reduced-motion switch: zoom transitions snap.
    pub reduced_motion: bool,
}

impl Default for PinBehavior {
    fn default() -> Self {
        Self {
            min_size: MIN_SIZE,
            anchor: ResizeAnchor::default(),
            client_resize: false,
            tokens: DesignTokens::default(),
            reduced_motion: false,
        }
    }
}

/// The state of one pin window.
#[derive(Debug, Clone, PartialEq)]
pub struct PinState {
    pub(super) upright_size: (u32, u32),
    pub(super) image_size: (u32, u32),
    pub(super) rotation: Rotation,
    pub(super) scale: f64,
    pub(super) opacity_tenths: u8,
    pub(super) offset: (f64, f64),
    /// The extent the state has requested (compositor round-trip pending
    /// or applied); zoom math predicts from THIS, never the stale actual.
    pub(super) target_window: (u32, u32),
    pub(super) screen: (u32, u32),
    pub(super) scale_factor: f64,
    pub(super) margin_px: f64,
    pub(super) behavior: PinBehavior,
    pub(super) wheel_accum: f64,
    pub(super) cursor: Option<(f64, f64)>,
    pub(super) hover: bool,
    pub(super) left_pressed: bool,
    pub(super) drag_started: bool,
    pub(super) last_left_press: Option<Instant>,
    pub(super) modifiers: ModifiersState,
    pub(super) menu: Option<PinMenu>,
    pub(super) pinch: PinchTracker,
    pub(super) zoom_anim: Option<super::anim::ZoomAnim>,
}

impl PinState {
    /// Builds the state for an upright image of `image` physical px on a
    /// `screen`-sized monitor at `scale_factor`, starting at 1:1 zoom
    /// clamped into the screen-fit / `MIN_SIZE` bounds (small images start
    /// magnified to the floor, oversized images start clamped to the
    /// screen - the failure-QA rule).
    #[must_use]
    pub fn new(
        image: (u32, u32),
        screen: (u32, u32),
        scale_factor: f64,
        behavior: PinBehavior,
    ) -> Self {
        let factor = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let margin_px = MARGIN * factor;
        let scale = clamp_scale(1.0, Self::bounds_for(image, screen, margin_px, &behavior));
        let target_window = window_size(content_size(image, scale), margin_px);
        Self {
            upright_size: image,
            image_size: image,
            rotation: Rotation::Up0,
            scale,
            opacity_tenths: super::spec::OPACITY_TENTHS,
            offset: (0.0, 0.0),
            target_window,
            screen,
            scale_factor: factor,
            margin_px,
            behavior,
            wheel_accum: 0.0,
            cursor: None,
            hover: false,
            left_pressed: false,
            drag_started: false,
            last_left_press: None,
            modifiers: ModifiersState::default(),
            menu: None,
            pinch: PinchTracker::default(),
            zoom_anim: None,
        }
    }

    /// Routes one input event; returns the effects for the shell to apply.
    /// `now` is injectable for deterministic double-click timing (the
    /// selection engine's `SelectionEnv` pattern). The full dispatch lives in the
    /// crate-private `pins::interact` module.
    pub fn on_input(&mut self, input: &PinInput, now: Instant) -> Vec<PinEffect> {
        super::interact::dispatch(self, input, now)
    }

    fn bounds_for(
        image: (u32, u32),
        screen: (u32, u32),
        margin_px: f64,
        behavior: &PinBehavior,
    ) -> ScaleBounds {
        scale_bounds(image, screen, margin_px, behavior.min_size)
    }

    /// The current zoom bounds.
    #[must_use]
    pub fn bounds(&self) -> ScaleBounds {
        Self::bounds_for(self.image_size, self.screen, self.margin_px, &self.behavior)
    }

    /// The requested window extent in physical px (what the shell pins
    /// min+max inner size to; the initial value seeds window creation).
    #[must_use]
    pub const fn target_window(&self) -> (u32, u32) {
        self.target_window
    }

    /// The current zoom factor (diagnostics/QA).
    #[must_use]
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    /// The current rotation.
    #[must_use]
    pub const fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// The rotated image dimensions in physical px.
    #[must_use]
    pub const fn image_size(&self) -> (u32, u32) {
        self.image_size
    }

    /// Window opacity in tenths (0..=10; keys 0-9 map per F27).
    #[must_use]
    pub const fn opacity_tenths(&self) -> u8 {
        self.opacity_tenths
    }

    /// Window opacity as a 0.0..=1.0 factor.
    #[must_use]
    pub fn opacity(&self) -> f32 {
        f32::from(self.opacity_tenths) / f32::from(super::spec::OPACITY_TENTHS)
    }

    /// Whether the pointer is inside the window (shadow hover color).
    #[must_use]
    pub const fn hovered(&self) -> bool {
        self.hover
    }

    /// The open context menu, when any.
    #[must_use]
    pub const fn menu(&self) -> Option<&PinMenu> {
        self.menu.as_ref()
    }

    /// The design tokens (menu/shadow drawing).
    #[must_use]
    pub const fn tokens(&self) -> &DesignTokens {
        &self.behavior.tokens
    }

    /// The window device scale factor (margin conversion point).
    #[must_use]
    pub const fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// The shadow frame width in physical px.
    #[must_use]
    pub const fn margin_px(&self) -> f64 {
        self.margin_px
    }

    /// The image draw offset within the content area, physical px (the
    /// zoom-to-cursor anchor; `(0, 0)` while the image fills the window).
    #[must_use]
    pub const fn offset(&self) -> (f64, f64) {
        self.offset
    }
}

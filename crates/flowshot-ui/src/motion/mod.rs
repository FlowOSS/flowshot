//! Token-driven motion (plan todo 41, draft D8(d)).
//!
//! The animation layer every animated surface shares: cubic-bezier easing
//! evaluated from the `flowshot_core::tokens` curves (NEVER hardcoded
//! control points), retargetable value [`Tween`]s, and the staggered-reveal
//! progress function behind the toolbar's appearance animation.
//!
//! # Frame-scheduling contract (the todo-13 idle rule)
//!
//! Motion is damage/timeline-driven: a tween is a PURE function of
//! `(start, now)`, so nothing burns CPU between frames. The shell asks the
//! motion owners ([`crate::chrome::ChromeMotion`], the selection engine's
//! grip motion, the pin zoom transition) whether anything is still moving
//! (`active_at`) and when to wake next (`deadline`, clamped to
//! [`FRAME_INTERVAL`]); an idle overlay with settled animations stays in
//! `ControlFlow::Wait` at zero CPU, exactly like the HUD countdown does.
//!
//! # Reduced motion
//!
//! Every spec resolves through [`MotionSpec`]; the reduced-motion switch
//! swaps in [`MotionSpec::instant`] (zero duration), which makes each tween
//! snap to its target and report `active_at == false` - transitions become
//! instant and NO animation frames are scheduled (the plan todo-41 failure
//! QA: "reduced-motion config -> transitions instant").

mod easing;
mod tween;

#[cfg(test)]
mod tests;

use std::time::Duration;

use flowshot_core::tokens::DesignTokens;

pub use easing::{cubic_bezier, curve_from_tokens};
pub use tween::{MotionSpec, StaggerSpec, Tween, stagger_progress};

/// Animation frame pacing while a transition runs: ~60 Hz. The `Fifo`
/// present mode throttles to the output refresh anyway; the interval only
/// bounds the `ControlFlow::WaitUntil` wake cadence. Settled animations
/// schedule NOTHING (the idle zero-CPU contract).
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Selection-handle hover-grow duration (D8(d) "handle hover").
pub const HANDLE_GROW_MS: u64 = 120;
/// Toolbar-button hover/press wash duration (small, quick = `sharp`).
pub const BUTTON_WASH_MS: u64 = 100;
/// Side-panel slide-in duration (arriving element = `decelerate`).
pub const PANEL_ENTER_MS: u64 = 180;
/// Side-panel slide-out duration (leaving element = `accelerate`).
pub const PANEL_EXIT_MS: u64 = 140;
/// Color-wheel popover scale-in duration (arriving = `decelerate`).
pub const WHEEL_ENTER_MS: u64 = 150;
/// Color-wheel popover close duration (leaving = `accelerate`).
pub const WHEEL_EXIT_MS: u64 = 100;
/// Pin zoom easing duration (D8(d) "pin zoom").
pub const PIN_ZOOM_MS: u64 = 150;
/// One toolbar button's fade+slide duration (plan todo 41: 120-180ms band).
pub const REVEAL_ELEMENT_MS: u64 = 120;
/// The whole staggered toolbar reveal's duration (plan todo 41 band top).
pub const REVEAL_TOTAL_MS: u64 = 180;

/// The grip radius growth at full hover (1.0 = +35% radius).
pub const HANDLE_GROW_FACTOR: f64 = 0.35;
/// The color-wheel scale-in's starting scale (grows to 1.0).
pub const WHEEL_SCALE_FROM: f32 = 0.85;

impl MotionSpec {
    /// The selection-handle hover-grow spec (standard curve).
    #[must_use]
    pub fn handle_grow(tokens: &DesignTokens) -> Self {
        Self::resolve(tokens, "standard", Duration::from_millis(HANDLE_GROW_MS))
    }

    /// The toolbar-button hover/press wash spec (sharp curve).
    #[must_use]
    pub fn button_wash(tokens: &DesignTokens) -> Self {
        Self::resolve(tokens, "sharp", Duration::from_millis(BUTTON_WASH_MS))
    }

    /// The side-panel slide-in spec (decelerate curve).
    #[must_use]
    pub fn panel_enter(tokens: &DesignTokens) -> Self {
        Self::resolve(tokens, "decelerate", Duration::from_millis(PANEL_ENTER_MS))
    }

    /// The side-panel slide-out spec (accelerate curve).
    #[must_use]
    pub fn panel_exit(tokens: &DesignTokens) -> Self {
        Self::resolve(tokens, "accelerate", Duration::from_millis(PANEL_EXIT_MS))
    }

    /// The color-wheel scale-in spec (decelerate curve).
    #[must_use]
    pub fn wheel_enter(tokens: &DesignTokens) -> Self {
        Self::resolve(tokens, "decelerate", Duration::from_millis(WHEEL_ENTER_MS))
    }

    /// The color-wheel close spec (accelerate curve).
    #[must_use]
    pub fn wheel_exit(tokens: &DesignTokens) -> Self {
        Self::resolve(tokens, "accelerate", Duration::from_millis(WHEEL_EXIT_MS))
    }

    /// The pin zoom easing spec (standard curve).
    #[must_use]
    pub fn pin_zoom(tokens: &DesignTokens) -> Self {
        Self::resolve(tokens, "standard", Duration::from_millis(PIN_ZOOM_MS))
    }
}

impl StaggerSpec {
    /// The toolbar reveal spec: per-button fade+slide over
    /// [`REVEAL_ELEMENT_MS`], staggered so the last button lands at
    /// [`REVEAL_TOTAL_MS`] (decelerate curve - the toolbar arrives).
    #[must_use]
    pub fn toolbar_reveal(tokens: &DesignTokens) -> Self {
        Self {
            element: Duration::from_millis(REVEAL_ELEMENT_MS),
            total: Duration::from_millis(REVEAL_TOTAL_MS),
            curve: curve_from_tokens(tokens, "decelerate"),
        }
    }
}

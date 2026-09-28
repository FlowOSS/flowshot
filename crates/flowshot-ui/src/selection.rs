//! The selection interaction engine (draft F27).
//!
//! The complete Flameshot selection behavior spec, clean-room reimplemented
//! on the shell's input seam and expressed in GLOBAL LOGICAL space so one
//! selection rect spans every monitor (#4894 regression restored - the
//! per-output cropping at export is the layout algebra of
//! `flowshot_core::geometry`, consumed by the backdrop and the export path):
//!
//! - drag-create behind a 3px manhattan threshold (a click is never a
//!   selection), 8 token-derived handles, hit priority corners > edges >
//!   center, inside-drag moves,
//! - Shift mirrors a resize around the start center, Ctrl constrains the
//!   aspect ratio (the exact per-handle formulas of `selectionwidget.cpp`),
//! - arrows move 1px, Shift+arrows resize one edge 1px, Ctrl+Shift+arrows
//!   resize symmetrically 1px (CODE values per F27 - the README's 2px is
//!   rejected),
//! - every rect clamps to the layout and holds the 10x10 minimum
//!   (BORROW-MODIFIED from Flameshot's 1x1),
//! - the geometry HUD (`WxH+X+Y`) shows on every change at the configured
//!   position 0-5 and hides after the configured hide-time,
//! - Esc walks the six-stage cascade in the exact spec order, Ctrl+Q exits
//!   immediately, Enter accepts, Ctrl+C copies, Ctrl+A selects the full
//!   layout, right-click is the color-wheel seam (chrome-owned), and a
//!   double-click inside the selection copies when configured.
//!
//! The engine is pure state: [`OverlayCore`](crate::OverlayCore) feeds it
//! clamped global positions, the modifier snapshot, and the layout bounds
//! through [`SelectionEnv`]; it answers with [`SelectionUpdate`]s (effects
//! for the shell + whether windows must redraw). No annotation drawing
//! lives here (that is the editor's), and no behavior constant is tunable
//! beyond the config keys [`SelectionConfig`] lists.

mod cascade;
mod drag;
mod events;
mod hit;
mod hover;
mod hud;
mod keys;
mod metrics;
mod paint;
mod resize;
mod types;

#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo};
use flowshot_core::tokens::{DesignTokens, Spacing};

use crate::render::{Color, DisplayList};

use paint::SelectionColors;

pub use cascade::{CascadeState, EscStep};
pub use hit::{Handle, HitZone};
pub use hud::{HudPosition, HudView, format_geometry};
// The launch preselect shares the engine's explicit layout clamp
// (size-preserving shift, intersection when oversized) - one math, two
// callers.
pub use metrics::{
    AVG_GLYPH_ADVANCE, BUTTON_BASE_FACTOR, DOUBLE_CLICK_INTERVAL, DRAG_THRESHOLD, GRIP_DIVISOR,
    HANDLE_AREA_FACTOR, HUD_BACKGROUND_ALPHA, LINE_SPACING_RATIO, MIN_SELECTION_SIDE,
    OUTLINE_WIDTH, SelectionMetrics,
};
pub(crate) use resize::fit_into_bounds;
pub use types::{Effect, SelectionConfig, SelectionEnv, SelectionUpdate};

/// The selection state machine: one selection rect in global logical space,
/// the active drag, the Esc-cascade seams, the HUD timer, and the grip
/// hover-grow motion.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionState {
    config: SelectionConfig,
    metrics: SelectionMetrics,
    spacing: Spacing,
    colors: SelectionColors,
    font_family: String,
    hud_radius: f64,
    rect: Option<LogicalRect>,
    active_drag: Option<drag::Drag>,
    cascade: CascadeState,
    hud_timer: hud::HudTimer,
    last_press: Option<PressRecord>,
    grips: hover::GripMotion,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PressRecord {
    at: Instant,
    position: LogicalPoint,
}

impl Default for SelectionState {
    fn default() -> Self {
        Self::new(SelectionConfig::default(), &DesignTokens::default())
    }
}

impl SelectionState {
    /// Builds the engine from the config keys and the design tokens (every
    /// size and color derives from them - zero hardcoded visual constants).
    #[must_use]
    pub fn new(config: SelectionConfig, tokens: &DesignTokens) -> Self {
        let metrics = SelectionMetrics::from_tokens(tokens);
        let accent = Color::from_hex_token(&tokens.palette.accent).unwrap_or_else(|| {
            tracing::error!("accent token malformed; selection paints magenta");
            Color::from_rgba8(255, 0, 255, 255)
        });
        Self {
            config,
            metrics,
            spacing: tokens.spacing,
            colors: SelectionColors::from_accent(accent),
            font_family: tokens.typography.family.clone(),
            hud_radius: f64::from(tokens.radii.small),
            rect: None,
            active_drag: None,
            cascade: CascadeState::empty(),
            hud_timer: hud::HudTimer::default(),
            last_press: None,
            grips: hover::GripMotion::new(tokens, Instant::now()),
        }
    }

    /// Re-derives every token-driven visual (metrics, colors, typography,
    /// hover motion) from a new token set - the theme-pass seam the QA
    /// bundle and the settings live-reload use.
    pub fn retheme(&mut self, tokens: &DesignTokens) {
        self.metrics = SelectionMetrics::from_tokens(tokens);
        self.spacing = tokens.spacing;
        if let Some(accent) = Color::from_hex_token(&tokens.palette.accent) {
            self.colors = SelectionColors::from_accent(accent);
        }
        self.font_family.clone_from(&tokens.typography.family);
        self.hud_radius = f64::from(tokens.radii.small);
        self.grips.retheme(tokens);
    }

    /// The reduced-motion switch: `true` snaps the grip hover-grow
    /// (and every later selection transition) to its target.
    pub fn set_motion_reduced(&mut self, reduced: bool) {
        self.grips.set_reduced(reduced);
    }

    /// Updates the grip hover-grow target from the pointer position (the
    /// funnel calls this on every motion, editor-consumed or not - the
    /// handles stay hoverable while a tool is active).
    pub fn update_hover(&mut self, at: LogicalPoint, now: Instant) {
        let hovered = self
            .rect
            .and_then(|rect| match hit::hit_zone(rect, at, &self.metrics) {
                HitZone::Handle(handle) => Some(handle),
                HitZone::Inside | HitZone::Outside => None,
            });
        self.grips.update(hovered, now);
    }

    /// Whether any grip hover transition is still moving (the shell's
    /// per-frame redraw test).
    #[must_use]
    pub fn motion_active(&self, now: Instant) -> bool {
        self.grips.active_at(now)
    }

    /// The earliest grip-motion settle instant after `now` (a
    /// `ControlFlow::WaitUntil` input; `None` when every grip is at rest).
    #[must_use]
    pub fn motion_wake(&self, now: Instant) -> Option<Instant> {
        self.grips.wake(now)
    }

    /// Replaces the config keys (harness/settings seam); the HUD timer is
    /// untouched.
    pub fn configure(&mut self, config: SelectionConfig) {
        self.config = config;
    }

    /// The current selection in global logical space.
    #[must_use]
    pub const fn rect(&self) -> Option<LogicalRect> {
        self.rect
    }

    /// Seeds the selection without input (the launch preselect seam).
    /// Mirrors Flameshot's `initialSelection`: no HUD show (the indicator
    /// only appears on user-driven geometry changes).
    pub fn set_rect(&mut self, rect: Option<LogicalRect>) {
        self.rect = rect;
        if rect.is_none() {
            self.hud_timer.hide();
        }
    }

    /// The Esc-cascade seam state (the editor and chrome layers set and
    /// clear the stages).
    #[must_use]
    pub const fn cascade(&self) -> &CascadeState {
        &self.cascade
    }

    /// Mutable Esc-cascade seam (the editor and chrome layers own it).
    pub fn cascade_mut(&mut self) -> &mut CascadeState {
        &mut self.cascade
    }

    /// The token-derived metrics (handle sizes, minimum, threshold).
    #[must_use]
    pub const fn metrics(&self) -> &SelectionMetrics {
        &self.metrics
    }

    /// The HUD content when visible at `now` (the render path consumes it).
    #[must_use]
    pub fn hud_view(&self, now: Instant) -> Option<HudView> {
        let rect = self.rect?;
        if !self.hud_timer.visible(now, self.hide_time()) {
            return None;
        }
        hud::hud_view(
            rect,
            HudPosition::from_config(self.config.hud_position),
            &self.metrics,
            &self.spacing,
        )
    }

    /// The instant the HUD will auto-hide, when a countdown runs (the
    /// shell's `ControlFlow::WaitUntil` wake).
    #[must_use]
    pub fn hud_wake(&self) -> Option<Instant> {
        self.hud_timer.wake(self.hide_time())
    }

    /// Evaluates the HUD deadline at `now`; `true` when this call hid the
    /// HUD (the shell redraws every window once).
    pub fn tick(&mut self, now: Instant) -> bool {
        self.hud_timer.tick(now, self.hide_time())
    }

    /// Appends this window's selection visuals (outline, grips, HUD) to
    /// `list`, derived from the global rect with `output`'s own scale.
    pub fn paint_into(&self, list: &mut DisplayList, output: &OutputInfo, now: Instant) {
        let Some(rect) = self.rect else {
            return;
        };
        let view = self.hud_view(now);
        paint::append(
            list,
            &paint::SelectionPaint {
                output,
                rect,
                hud: view.as_ref(),
                metrics: &self.metrics,
                colors: &self.colors,
                font_family: Some(self.font_family.as_str()),
                hud_radius: self.hud_radius,
                grip_scales: self.grips.scales_at(now),
            },
        );
    }

    fn commit_rect(&mut self, rect: Option<LogicalRect>, now: Instant) -> SelectionUpdate {
        let changed = rect != self.rect;
        self.rect = rect;
        if changed {
            if let Some(committed) = rect {
                tracing::debug!(
                    target: "flowshot_ui::selection",
                    geometry = %hud::format_geometry(committed),
                    "selection changed"
                );
                self.hud_timer.show(now);
            } else {
                tracing::debug!(
                    target: "flowshot_ui::selection",
                    geometry = "none",
                    "selection cleared"
                );
                self.hud_timer.hide();
            }
        }
        SelectionUpdate {
            effects: Vec::new(),
            changed,
            esc_step: None,
        }
    }

    fn consume_double_click(&mut self, now: Instant, at: LogicalPoint) -> bool {
        let fired = match (self.last_press, self.rect) {
            (Some(last), Some(rect)) => {
                self.config.double_click_copies
                    && now >= last.at
                    && now - last.at <= DOUBLE_CLICK_INTERVAL
                    && drag::manhattan(last.position, at) <= self.metrics.drag_threshold
                    && hit::hit_zone(rect, at, &self.metrics) == HitZone::Inside
            }
            _ => false,
        };
        // After a fired double-click the record clears: a third press starts
        // a fresh click pair (Qt does not re-fire on the third press).
        self.last_press = if fired {
            None
        } else {
            Some(PressRecord {
                at: now,
                position: at,
            })
        };
        fired
    }

    fn hide_time(&self) -> Duration {
        Duration::from_millis(u64::from(self.config.hud_hide_time))
    }
}

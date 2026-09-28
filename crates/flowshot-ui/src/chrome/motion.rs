//! The chrome motion owner (plan todo 41, draft D8(d)).
//!
//! One place knows when the chrome animates: the toolbar's staggered
//! fade+slide reveal (120-180ms band), the side-panel slide, the color-wheel
//! popover scale-in, and the per-button hover/press wash. Every transition is
//! a token-eased [`Tween`] evaluated at the `now` the caller was handed -
//! pure functions of time, zero background work. The shell's frame scheduler
//! consults [`ChromeMotion::active_at`] / [`ChromeMotion::wake`]: settled
//! chrome schedules NOTHING (the todo-13 idle zero-CPU contract), and the
//! reduced-motion switch ([`ChromeMotion::set_reduced`]) snaps every
//! transition to its target.
//!
//! Hit-testing deliberately uses the FINAL geometry (the transitions are
//! <=180ms; interaction leads the visual, so a panel is never unclickable
//! mid-slide) - recorded in the todo-41 motion checklist.

use std::time::{Duration, Instant};

use flowshot_core::tokens::DesignTokens;

use crate::motion::{MotionSpec, StaggerSpec, Tween, stagger_progress};

/// The resolved transition specs (curves from the token set).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Specs {
    panel_enter: MotionSpec,
    panel_exit: MotionSpec,
    wheel_enter: MotionSpec,
    wheel_exit: MotionSpec,
    wash: MotionSpec,
    reveal: StaggerSpec,
}

impl Specs {
    fn resolve(tokens: &DesignTokens) -> Self {
        Self {
            panel_enter: MotionSpec::panel_enter(tokens),
            panel_exit: MotionSpec::panel_exit(tokens),
            wheel_enter: MotionSpec::wheel_enter(tokens),
            wheel_exit: MotionSpec::wheel_exit(tokens),
            wash: MotionSpec::button_wash(tokens),
            reveal: StaggerSpec::toolbar_reveal(tokens),
        }
    }
}

/// The chrome's animation state: reveal start, panel/wheel progress, and
/// one hover/press wash tween per toolbar button.
#[derive(Debug, Clone, PartialEq)]
pub struct ChromeMotion {
    specs: Specs,
    reduced: bool,
    reveal_start: Option<Instant>,
    selection_present: bool,
    panel: Tween,
    wheel: Tween,
    wash: Vec<Tween>,
    hover: Option<usize>,
    press: Option<usize>,
}

impl ChromeMotion {
    /// Builds the motion state at rest (nothing revealed, panel/wheel
    /// hidden, no buttons).
    #[must_use]
    pub fn new(tokens: &DesignTokens, now: Instant) -> Self {
        let specs = Specs::resolve(tokens);
        Self {
            specs,
            reduced: false,
            reveal_start: None,
            selection_present: false,
            panel: Tween::settled(0.0, specs.panel_exit, now),
            wheel: Tween::settled(0.0, specs.wheel_exit, now),
            wash: Vec::new(),
            hover: None,
            press: None,
        }
    }

    /// Re-resolves the specs from new tokens and resizes the wash track to
    /// the toolbar's button count (a reorder resets the wash - hover
    /// retargets on the next motion event).
    pub fn configure(&mut self, tokens: &DesignTokens, buttons: usize, now: Instant) {
        self.specs = Specs::resolve(tokens);
        self.wash.clear();
        self.wash.resize(
            buttons,
            Tween::settled(0.0, self.wash_spec(), now),
        );
        self.hover = None;
        self.press = None;
    }

    /// The reduced-motion switch: every spec degrades to
    /// [`MotionSpec::instant`] (snap to target, schedule nothing).
    pub fn set_reduced(&mut self, reduced: bool) {
        self.reduced = reduced;
    }

    /// Whether reduced motion is on.
    #[must_use]
    pub const fn reduced(&self) -> bool {
        self.reduced
    }

    /// Advances the visibility-driven transitions. Called once per event-loop
    /// pass (the shell's `tick`): the toolbar reveal (re)starts on the
    /// selection's rising edge, and the panel/wheel tweens retarget toward
    /// their visibility flags (no-ops while unchanged).
    pub fn tick(&mut self, now: Instant, selection_present: bool, panel: bool, wheel: bool) {
        if selection_present && !self.selection_present {
            self.reveal_start = Some(now);
        } else if !selection_present {
            self.reveal_start = None;
        }
        self.selection_present = selection_present;
        let panel_spec = self.pick(panel, self.specs.panel_enter, self.specs.panel_exit);
        self.panel.retarget(f64::from(panel), panel_spec, now);
        let wheel_spec = self.pick(wheel, self.specs.wheel_enter, self.specs.wheel_exit);
        self.wheel.retarget(f64::from(wheel), wheel_spec, now);
    }

    /// Retargets the hover wash (the funnel's motion seam; `None` = the
    /// pointer is off every toolbar button).
    pub fn set_hover(&mut self, index: Option<usize>, now: Instant) {
        if self.hover == index {
            return;
        }
        let previous = self.hover;
        self.hover = index;
        if let Some(old) = previous.filter(|old| self.press != Some(*old)) {
            self.retarget_wash(old, 0.0, now);
        }
        if let Some(new) = index.filter(|new| self.press != Some(*new)) {
            self.retarget_wash(new, 1.0, now);
        }
    }

    /// Retargets the press wash (`Some(i)` = button i is down; `None` =
    /// release, falling back to the hover level).
    pub fn set_press(&mut self, index: Option<usize>, now: Instant) {
        if self.press == index {
            return;
        }
        let previous = self.press;
        self.press = index;
        if let Some(old) = previous {
            let rest = f64::from(self.hover == Some(old));
            self.retarget_wash(old, rest, now);
        }
        if let Some(new) = index {
            self.retarget_wash(new, 2.0, now);
        }
    }

    fn retarget_wash(&mut self, index: usize, target: f64, now: Instant) {
        let spec = self.wash_spec();
        if let Some(tween) = self.wash.get_mut(index) {
            tween.retarget(target, spec, now);
        }
    }

    /// The toolbar background's reveal fade at `now` (the first element's
    /// progress - the plate lands with the leading button).
    #[must_use]
    pub fn reveal_background(&self, now: Instant) -> f32 {
        self.reveal_progress(now, 0, 1)
    }

    /// Button `index` of `count`'s staggered reveal progress at `now`
    /// (1.0 when no reveal runs - a static toolbar paints settled).
    #[must_use]
    pub fn reveal_progress(&self, now: Instant, index: usize, count: usize) -> f32 {
        let Some(start) = self.reveal_start else {
            return 1.0;
        };
        let elapsed = now.saturating_duration_since(start);
        let spec = if self.reduced {
            StaggerSpec {
                element: Duration::ZERO,
                ..self.specs.reveal
            }
        } else {
            self.specs.reveal
        };
        stagger_progress(elapsed, index, count, &spec)
    }

    /// The side-panel slide progress at `now` (0 = hidden, 1 = shown).
    #[must_use]
    pub fn panel_progress(&self, now: Instant) -> f64 {
        self.panel.value_at(now)
    }

    /// The color-wheel scale-in progress at `now` (0 = closed, 1 = open).
    #[must_use]
    pub fn wheel_progress(&self, now: Instant) -> f64 {
        self.wheel.value_at(now)
    }

    /// Button `index`'s hover/press wash level at `now` (0 = idle,
    /// 1 = hover, 2 = press; out-of-range indexes read 0).
    #[must_use]
    pub fn wash(&self, now: Instant, index: usize) -> f64 {
        self.wash
            .get(index)
            .map_or(0.0, |tween| tween.value_at(now))
    }

    /// Whether any chrome transition is still moving at `now` (the shell's
    /// per-frame redraw test).
    #[must_use]
    pub fn active_at(&self, now: Instant) -> bool {
        self.reveal_active(now)
            || self.panel.active_at(now)
            || self.wheel.active_at(now)
            || self.wash.iter().any(|tween| tween.active_at(now))
    }

    /// The earliest FUTURE settle deadline while anything moves (`None`
    /// when settled). Frame pacing is the scheduler's job
    /// ([`crate::OverlayCore::wake`] clamps it to
    /// [`crate::motion::FRAME_INTERVAL`]).
    #[must_use]
    pub fn settle_at(&self, now: Instant) -> Option<Instant> {
        if !self.active_at(now) {
            return None;
        }
        self.panel
            .deadline()
            .into_iter()
            .chain(self.wheel.deadline())
            .chain(self.wash.iter().filter_map(Tween::deadline))
            .chain(self.reveal_start.and_then(|start| {
                let total = if self.reduced {
                    Duration::ZERO
                } else {
                    self.specs.reveal.total
                };
                start.checked_add(total)
            }))
            .filter(|deadline| *deadline > now)
            .min()
    }

    fn reveal_active(&self, now: Instant) -> bool {
        let Some(start) = self.reveal_start else {
            return false;
        };
        if self.reduced {
            return false;
        }
        now.saturating_duration_since(start) < self.specs.reveal.total
    }

    fn pick(&self, entering: bool, enter: MotionSpec, exit: MotionSpec) -> MotionSpec {
        if self.reduced {
            MotionSpec::instant()
        } else if entering {
            enter
        } else {
            exit
        }
    }

    fn wash_spec(&self) -> MotionSpec {
        if self.reduced {
            MotionSpec::instant()
        } else {
            self.specs.wash
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::motion::{
        BUTTON_WASH_MS, HANDLE_GROW_MS, PANEL_ENTER_MS, REVEAL_TOTAL_MS, WHEEL_ENTER_MS,
    };

    fn tokens() -> DesignTokens {
        DesignTokens::default()
    }

    fn motion(buttons: usize) -> (ChromeMotion, Instant) {
        let t0 = Instant::now();
        let mut motion = ChromeMotion::new(&tokens(), t0);
        motion.configure(&tokens(), buttons, t0);
        (motion, t0)
    }

    #[test]
    fn reveal_starts_on_the_selection_rising_edge_and_settles() {
        let (mut motion, t0) = motion(4);
        // Before any selection: static (progress 1.0 - nothing animates).
        assert_eq!(motion.reveal_progress(t0, 0, 4), 1.0);
        motion.tick(t0, true, false, false);
        assert_eq!(motion.reveal_progress(t0, 0, 4), 0.0, "reveal at start");
        assert!(motion.active_at(t0));
        let mid = t0 + Duration::from_millis(REVEAL_TOTAL_MS / 2);
        let first = motion.reveal_progress(mid, 0, 4);
        let last = motion.reveal_progress(mid, 3, 4);
        assert!(first > 0.0 && first < 1.0, "first mid {first}");
        assert!(first > last, "stagger ordering");
        // Settled after the total: exact 1.0, nothing scheduled.
        let end = t0 + Duration::from_millis(REVEAL_TOTAL_MS);
        assert_eq!(motion.reveal_progress(end, 3, 4), 1.0);
        assert!(!motion.active_at(end));
        assert!(motion.settle_at(end).is_none());
        // A repeated tick with the selection still present does NOT restart.
        motion.tick(end, true, false, false);
        assert_eq!(motion.reveal_progress(end, 0, 4), 1.0);
    }

    #[test]
    fn reveal_restarts_when_the_selection_returns() {
        let (mut motion, t0) = motion(2);
        motion.tick(t0, true, false, false);
        let gone = t0 + Duration::from_millis(REVEAL_TOTAL_MS);
        motion.tick(gone, false, false, false);
        assert!(motion.reveal_start.is_none());
        let back = gone + Duration::from_millis(10);
        motion.tick(back, true, false, false);
        assert_eq!(motion.reveal_progress(back, 0, 2), 0.0);
    }

    #[test]
    fn panel_and_wheel_tween_toward_visibility() {
        let (mut motion, t0) = motion(1);
        motion.tick(t0, true, true, true);
        let panel_end = t0 + Duration::from_millis(PANEL_ENTER_MS);
        let wheel_end = t0 + Duration::from_millis(WHEEL_ENTER_MS);
        assert!(motion.panel_progress(t0 + Duration::from_millis(60)) > 0.0);
        assert_eq!(motion.panel_progress(panel_end), 1.0);
        assert_eq!(motion.wheel_progress(wheel_end), 1.0);
        // Hiding runs the exit spec back to zero.
        motion.tick(panel_end, true, false, false);
        let hidden = panel_end + Duration::from_millis(PANEL_ENTER_MS);
        assert_eq!(motion.panel_progress(hidden), 0.0);
        assert!(!motion.active_at(hidden));
    }

    #[test]
    fn wash_tracks_hover_then_press_then_release() {
        let (mut motion, t0) = motion(3);
        motion.set_hover(Some(1), t0);
        let hovered = t0 + Duration::from_millis(BUTTON_WASH_MS);
        assert_eq!(motion.wash(hovered, 1), 1.0);
        assert_eq!(motion.wash(hovered, 0), 0.0);
        motion.set_press(Some(1), hovered);
        let pressed = hovered + Duration::from_millis(BUTTON_WASH_MS);
        assert_eq!(motion.wash(pressed, 1), 2.0);
        // Release with the pointer still on the button falls back to hover.
        motion.set_press(None, pressed);
        let released = pressed + Duration::from_millis(BUTTON_WASH_MS);
        assert_eq!(motion.wash(released, 1), 1.0);
        // Leaving drops to idle.
        motion.set_hover(None, released);
        let idle = released + Duration::from_millis(BUTTON_WASH_MS);
        assert_eq!(motion.wash(idle, 1), 0.0);
        assert!(!motion.active_at(idle));
    }

    #[test]
    fn hover_switch_crossfades_wash_between_buttons() {
        let (mut motion, t0) = motion(2);
        motion.set_hover(Some(0), t0);
        let switch = t0 + Duration::from_millis(BUTTON_WASH_MS / 2);
        motion.set_hover(Some(1), switch);
        let mid = switch + Duration::from_millis(BUTTON_WASH_MS / 4);
        assert!(motion.wash(mid, 0) < 1.0 && motion.wash(mid, 0) > 0.0);
        assert!(motion.wash(mid, 1) > 0.0 && motion.wash(mid, 1) < 1.0);
    }

    #[test]
    fn reduced_motion_snaps_every_transition() {
        let (mut motion, t0) = motion(2);
        motion.set_reduced(true);
        motion.tick(t0, true, true, true);
        motion.set_hover(Some(0), t0);
        assert_eq!(motion.reveal_progress(t0, 1, 2), 1.0);
        assert_eq!(motion.panel_progress(t0), 1.0);
        assert_eq!(motion.wheel_progress(t0), 1.0);
        assert_eq!(motion.wash(t0, 0), 1.0);
        assert!(!motion.active_at(t0), "nothing may animate");
        assert!(motion.settle_at(t0).is_none(), "nothing may be scheduled");
    }

    #[test]
    fn settle_deadline_tracks_the_running_transition() {
        let (mut motion, t0) = motion(1);
        motion.tick(t0, false, true, false);
        assert_eq!(
            motion.settle_at(t0),
            Some(t0 + Duration::from_millis(PANEL_ENTER_MS))
        );
        let end = t0 + Duration::from_millis(PANEL_ENTER_MS);
        assert!(motion.settle_at(end).is_none());
    }

    #[test]
    fn configure_resizes_the_wash_track() {
        let (mut motion, t0) = motion(2);
        motion.set_hover(Some(1), t0);
        motion.configure(&tokens(), 5, t0 + Duration::from_millis(BUTTON_WASH_MS));
        assert_eq!(motion.wash.len(), 5);
        assert_eq!(motion.wash(t0, 1), 0.0, "reconfigure resets the wash");
    }

    #[test]
    fn handle_grow_constant_stays_in_the_documented_band() {
        // The plan's 120-180ms reveal band: element <= total, both in band.
        assert!(HANDLE_GROW_MS >= 100 && HANDLE_GROW_MS <= 200);
        assert!(REVEAL_TOTAL_MS >= 120 && REVEAL_TOTAL_MS <= 180);
        assert!(WHEEL_ENTER_MS <= REVEAL_TOTAL_MS);
    }
}

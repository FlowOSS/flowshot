//! The chrome motion owner (plan todo 41, draft D8(d)).
//!
//! One place knows when the chrome animates: the toolbar's staggered
//! fade+slide reveal (120-180ms band), the side-panel slide, the color-wheel
//! popover scale-in, and the per-button hover/press wash. Every transition is
//! a token-eased [`Tween`] evaluated at the `now` the caller was handed -
//! pure functions of time, zero background work. The shell's frame scheduler
//! consults [`ChromeMotion::active_at`] / [`ChromeMotion::wake`] (the shell's
//! frame scheduler consults the motion's wake deadline to pace the frame
//! rate): settled chrome schedules NOTHING (the todo-13 idle zero-CPU
//! contract), and the reduced-motion switch ([`ChromeMotion::set_reduced`])
//! snaps every transition to its target.
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
        self.wash
            .resize(buttons, Tween::settled(0.0, self.wash_spec(), now));
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
mod tests;

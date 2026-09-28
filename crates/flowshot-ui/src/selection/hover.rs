//! Selection-handle hover motion (plan todo 41, D8(d) "handle hover-grow").
//!
//! Each of the eight grips owns one [`Tween`] (0 = resting, 1 = fully
//! grown); the paint path scales the grip radius by
//! `1 + HANDLE_GROW_FACTOR * value`. Hover retargets are continuous (a grip
//! caught mid-shrink grows from its shown size), and a settled grip
//! schedules nothing - the frame scheduler consults [`GripMotion::wake`].

use std::time::Instant;

use flowshot_core::tokens::DesignTokens;

use crate::motion::{HANDLE_GROW_FACTOR, MotionSpec, Tween};

use super::hit::Handle;

/// The eight grip hover-grow tweens, indexed by [`Handle::index`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GripMotion {
    grips: [Tween; Handle::ALL.len()],
    spec: MotionSpec,
    hovered: Option<Handle>,
}

impl GripMotion {
    /// Builds the resting motion state (every grip at 0).
    #[must_use]
    pub(super) fn new(tokens: &DesignTokens, now: Instant) -> Self {
        let spec = MotionSpec::handle_grow(tokens);
        Self {
            grips: std::array::from_fn(|_| Tween::settled(0.0, spec, now)),
            spec,
            hovered: None,
        }
    }

    /// Re-resolves the spec from new tokens (theme change).
    pub(super) fn retheme(&mut self, tokens: &DesignTokens) {
        self.spec = MotionSpec::handle_grow(tokens);
    }

    /// Swaps in the reduced-motion spec (`true` = transitions instant).
    pub(super) fn set_reduced(&mut self, reduced: bool) {
        self.spec = if reduced {
            MotionSpec::instant()
        } else {
            // Re-resolving needs tokens; the standard-curve default matches
            // every token set shipped (the curve data lives in core).
            MotionSpec::handle_grow(&DesignTokens::default())
        };
    }

    /// Retargets every grip toward `hovered` (a no-op per grip when the
    /// target is unchanged, so calling this on every motion event is safe).
    pub(super) fn update(&mut self, hovered: Option<Handle>, now: Instant) {
        if self.hovered == hovered {
            return;
        }
        self.hovered = hovered;
        for (index, grip) in self.grips.iter_mut().enumerate() {
            let target = f64::from(hovered.is_some_and(|handle| handle.index() == index));
            grip.retarget(target, self.spec, now);
        }
    }

    /// The radius multiplier per grip at `now` (`Handle::index` order).
    #[must_use]
    pub(super) fn scales_at(&self, now: Instant) -> [f64; Handle::ALL.len()] {
        let mut scales = [1.0; Handle::ALL.len()];
        for (index, grip) in self.grips.iter().enumerate() {
            scales[index] = 1.0 + HANDLE_GROW_FACTOR * grip.value_at(now);
        }
        scales
    }

    /// Whether any grip is still moving at `now`.
    #[must_use]
    pub(super) fn active_at(&self, now: Instant) -> bool {
        self.grips.iter().any(|grip| grip.active_at(now))
    }

    /// The earliest FUTURE settle instant, when any grip is still moving at
    /// `now` (the shell's `ControlFlow::WaitUntil` input).
    #[must_use]
    pub(super) fn wake(&self, now: Instant) -> Option<Instant> {
        self.grips
            .iter()
            .filter(|grip| grip.active_at(now))
            .filter_map(Tween::deadline)
            .min()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use super::*;

    const GROW_MS: u64 = crate::motion::HANDLE_GROW_MS;

    #[test]
    fn hovered_grip_grows_and_settles_exactly() {
        let t0 = Instant::now();
        let mut motion = GripMotion::new(&DesignTokens::default(), t0);
        motion.update(Some(Handle::TopLeft), t0);
        let grown = 1.0 + HANDLE_GROW_FACTOR;
        // Mid-flight: grown handle between rest and full, others at rest.
        let mid = motion.scales_at(t0 + Duration::from_millis(GROW_MS / 2));
        assert!(mid[Handle::TopLeft.index()] > 1.0);
        assert!(mid[Handle::TopLeft.index()] < grown);
        assert_eq!(mid[Handle::Bottom.index()], 1.0);
        assert!(motion.active_at(t0 + Duration::from_millis(GROW_MS / 2)));
        // Settled: exact target, no drift, nothing scheduled.
        let end = t0 + Duration::from_millis(GROW_MS);
        assert_eq!(motion.scales_at(end)[Handle::TopLeft.index()], grown);
        assert!(!motion.active_at(end));
        assert!(motion.wake(end).is_none());
    }

    #[test]
    fn hover_switch_crossfades_both_grips() {
        let t0 = Instant::now();
        let mut motion = GripMotion::new(&DesignTokens::default(), t0);
        motion.update(Some(Handle::Left), t0);
        let switch = t0 + Duration::from_millis(GROW_MS / 2);
        motion.update(Some(Handle::Right), switch);
        let mid = motion.scales_at(switch + Duration::from_millis(GROW_MS / 4));
        // The old grip shrinks from its shown value; the new one grows.
        assert!(mid[Handle::Left.index()] < 1.0 + HANDLE_GROW_FACTOR);
        assert!(mid[Handle::Left.index()] > 1.0);
        assert!(mid[Handle::Right.index()] > 1.0);
        let end = switch + Duration::from_millis(GROW_MS);
        assert_eq!(motion.scales_at(end)[Handle::Left.index()], 1.0);
        assert_eq!(
            motion.scales_at(end)[Handle::Right.index()],
            1.0 + HANDLE_GROW_FACTOR
        );
    }

    #[test]
    fn hover_clear_shrinks_back_to_rest() {
        let t0 = Instant::now();
        let mut motion = GripMotion::new(&DesignTokens::default(), t0);
        motion.update(Some(Handle::Top), t0);
        let clear = t0 + Duration::from_millis(GROW_MS);
        motion.update(None, clear);
        let end = clear + Duration::from_millis(GROW_MS);
        assert_eq!(motion.scales_at(end)[Handle::Top.index()], 1.0);
        assert!(!motion.active_at(end));
    }

    #[test]
    fn repeated_same_hover_is_a_noop() {
        let t0 = Instant::now();
        let mut motion = GripMotion::new(&DesignTokens::default(), t0);
        motion.update(Some(Handle::TopLeft), t0);
        let before = motion.clone();
        motion.update(Some(Handle::TopLeft), t0 + Duration::from_millis(30));
        assert_eq!(motion, before, "same target must not restart the tween");
    }

    #[test]
    fn reduced_motion_snaps_without_scheduling() {
        let t0 = Instant::now();
        let mut motion = GripMotion::new(&DesignTokens::default(), t0);
        motion.set_reduced(true);
        motion.update(Some(Handle::BottomRight), t0);
        assert_eq!(
            motion.scales_at(t0)[Handle::BottomRight.index()],
            1.0 + HANDLE_GROW_FACTOR
        );
        assert!(!motion.active_at(t0));
        assert!(motion.wake(t0).is_none());
    }

    #[test]
    fn wake_is_the_earliest_settle_deadline() {
        let t0 = Instant::now();
        let mut motion = GripMotion::new(&DesignTokens::default(), t0);
        motion.update(Some(Handle::TopLeft), t0);
        assert_eq!(motion.wake(t0), Some(t0 + Duration::from_millis(GROW_MS)));
        assert_eq!(motion.wake(t0 + Duration::from_millis(GROW_MS)), None);
    }
}

//! The pin zoom transition (plan todo 41, D8(d) "pin zoom easing").
//!
//! A committed zoom resizes the WINDOW immediately (the compositor-driven
//! min==max mechanism of [`super::zoom`] cannot animate), but the painted
//! content eases from the previous visual scale/offset to the committed one
//! over the token `standard` curve. Both endpoints satisfy the cursor-anchor
//! identity `margin + offset + p * scale + delta = cursor`, and the offset is
//! AFFINE in the scale, so lerping the two with one shared eased parameter
//! `e` gives the exact closed form `pos(e) = cursor - e * delta`: the
//! anchored image point converges MONOTONICALLY onto the cursor across the
//! transition, with the compositor's instant window re-placement `delta`
//! (a few px per 3% step) as the only - and shrinking - deviation.
//!
//! The pinch PREVIEW path deliberately bypasses this (direct manipulation
//! must not lag); reduced motion snaps (zero-duration spec, no frames).

use std::time::Instant;

use crate::motion::{MotionSpec, Tween};

use super::state::PinState;

/// One running zoom transition (unit tween + the endpoint pair).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ZoomAnim {
    progress: Tween,
    from_scale: f64,
    to_scale: f64,
    from_offset: (f64, f64),
    to_offset: (f64, f64),
}

impl ZoomAnim {
    fn lerp(from: f64, to: f64, t: f64) -> f64 {
        from + (to - from) * t
    }

    fn scale_at(&self, now: Instant) -> f64 {
        Self::lerp(self.from_scale, self.to_scale, self.progress.value_at(now))
    }

    fn offset_at(&self, now: Instant) -> (f64, f64) {
        (
            Self::lerp(
                self.from_offset.0,
                self.to_offset.0,
                self.progress.value_at(now),
            ),
            Self::lerp(
                self.from_offset.1,
                self.to_offset.1,
                self.progress.value_at(now),
            ),
        )
    }
}

impl PinState {
    /// Starts (or restarts) the zoom transition from the visual state
    /// captured BEFORE the commit to the committed `scale`/`offset`
    /// (consecutive wheel notches retarget without a jump).
    pub(super) fn begin_zoom_anim(
        &mut self,
        from_scale: f64,
        from_offset: (f64, f64),
        now: Instant,
    ) {
        let spec = if self.behavior.reduced_motion {
            MotionSpec::instant()
        } else {
            MotionSpec::pin_zoom(&self.behavior.tokens)
        };
        self.zoom_anim = Some(ZoomAnim {
            progress: {
                let mut tween = Tween::settled(0.0, spec, now);
                tween.retarget(1.0, spec, now);
                tween
            },
            from_scale,
            to_scale: self.scale,
            from_offset,
            to_offset: self.offset,
        });
    }

    /// The painted zoom scale at `now` (the committed scale once settled).
    #[must_use]
    pub fn visual_scale(&self, now: Instant) -> f64 {
        self.visual_scale_offset(now).0
    }

    /// The painted image offset at `now` (the committed offset once
    /// settled), window-local physical px.
    #[must_use]
    pub fn visual_offset(&self, now: Instant) -> (f64, f64) {
        self.visual_scale_offset(now).1
    }

    pub(super) fn visual_scale_offset(&self, now: Instant) -> (f64, (f64, f64)) {
        match &self.zoom_anim {
            Some(anim) if anim.progress.active_at(now) => (anim.scale_at(now), anim.offset_at(now)),
            _ => (self.scale, self.offset),
        }
    }

    /// Whether a zoom transition is still running (the shell's per-frame
    /// redraw test).
    #[must_use]
    pub fn zoom_anim_active(&self, now: Instant) -> bool {
        self.zoom_anim
            .is_some_and(|anim| anim.progress.active_at(now))
    }

    /// The zoom transition's settle deadline, when running (the shell's
    /// `ControlFlow::WaitUntil` input).
    #[must_use]
    pub fn zoom_anim_deadline(&self) -> Option<Instant> {
        self.zoom_anim.and_then(|anim| anim.progress.deadline())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use winit::event::MouseButton;

    use super::*;
    use crate::motion::PIN_ZOOM_MS;
    use crate::pins::event::{PinEffect, PinInput};
    use crate::pins::state::PinBehavior;

    fn state() -> PinState {
        PinState::new((400, 300), (1920, 1080), 1.0, PinBehavior::default())
    }

    fn zoom(state: &mut PinState, now: Instant) {
        state.on_input(
            &PinInput::Button {
                button: MouseButton::Left,
                pressed: true,
            },
            now,
        );
        let effects = state.on_input(&PinInput::Wheel { units: 120.0 }, now);
        assert!(effects.contains(&PinEffect::Redraw));
    }

    #[test]
    fn wheel_zoom_eases_from_the_old_scale_to_the_committed_one() {
        let mut state = state();
        let t0 = Instant::now();
        let before = state.scale();
        zoom(&mut state, t0);
        let committed = state.scale();
        assert!(committed > before);
        // Start: the visual state is the pre-zoom scale (no jump).
        assert_eq!(state.visual_scale(t0), before);
        assert!(state.zoom_anim_active(t0));
        // Mid-flight: strictly between, monotone.
        let mid = t0 + Duration::from_millis(PIN_ZOOM_MS / 2);
        let mid_scale = state.visual_scale(mid);
        assert!(mid_scale > before && mid_scale < committed, "{mid_scale}");
        // Settled: EXACTLY the committed state, nothing scheduled.
        let end = t0 + Duration::from_millis(PIN_ZOOM_MS);
        assert_eq!(state.visual_scale(end), committed);
        assert_eq!(state.visual_offset(end), state.offset());
        assert!(!state.zoom_anim_active(end));
        assert!(state.zoom_anim_deadline().is_none_or(|d| d <= end));
    }

    #[test]
    fn consecutive_notches_retarget_without_jumping() {
        let mut state = state();
        let t0 = Instant::now();
        zoom(&mut state, t0);
        let mid = t0 + Duration::from_millis(PIN_ZOOM_MS / 2);
        let shown = state.visual_scale(mid);
        zoom(&mut state, mid);
        // The second commit starts from the shown value, not the committed.
        assert_eq!(state.visual_scale(mid), shown);
        let end = mid + Duration::from_millis(PIN_ZOOM_MS);
        assert_eq!(state.visual_scale(end), state.scale());
    }

    #[test]
    fn anchor_point_converges_monotonically_on_the_cursor() {
        // The closed form of the shared-parameter lerp (module header):
        // pos(e) = cursor - e * delta, so the anchored image point's
        // distance from the physical cursor shrinks monotonically from
        // |delta| to 0 across the transition.
        let mut state = state();
        let t0 = Instant::now();
        let cursor = (107.0, 82.0);
        state.on_input(
            &PinInput::CursorMoved {
                x: cursor.0,
                y: cursor.1,
            },
            t0,
        );
        let (from_scale, from_offset) = (state.scale(), state.offset());
        let old_window = state.target_window();
        let p = (
            (cursor.0 - state.margin_px() - from_offset.0) / from_scale,
            (cursor.1 - state.margin_px() - from_offset.1) / from_scale,
        );
        zoom(&mut state, t0);
        let to_scale = state.scale();
        let delta = (
            (f64::from(old_window.0) - f64::from(state.target_window().0)) / 2.0,
            (f64::from(old_window.1) - f64::from(state.target_window().1)) / 2.0,
        );
        let physical_cursor = (cursor.0 - delta.0, cursor.1 - delta.1);
        let mut previous_distance = f64::MAX;
        for step in 0..=10 {
            let now = t0 + Duration::from_millis(step * PIN_ZOOM_MS / 10);
            let scale = state.visual_scale(now);
            let offset = state.visual_offset(now);
            let pos = (
                state.margin_px() + offset.0 + p.0 * scale,
                state.margin_px() + offset.1 + p.1 * scale,
            );
            // The eased parameter, recovered from the scale lerp.
            let e = (scale - from_scale) / (to_scale - from_scale);
            assert!(
                (pos.0 - (cursor.0 - e * delta.0)).abs() < 1e-6,
                "step {step}: closed form violated"
            );
            let distance =
                ((pos.0 - physical_cursor.0).powi(2) + (pos.1 - physical_cursor.1).powi(2)).sqrt();
            assert!(
                distance <= previous_distance + 1e-9,
                "step {step}: anchor drifts away from the cursor"
            );
            previous_distance = distance;
        }
        assert!(previous_distance < 1e-6, "settled exactly on the cursor");
    }

    #[test]
    fn reduced_motion_snaps_the_zoom() {
        let mut state = PinState::new(
            (400, 300),
            (1920, 1080),
            1.0,
            PinBehavior {
                reduced_motion: true,
                ..PinBehavior::default()
            },
        );
        let t0 = Instant::now();
        zoom(&mut state, t0);
        assert_eq!(state.visual_scale(t0), state.scale());
        assert!(!state.zoom_anim_active(t0));
        assert!(state.zoom_anim_deadline().is_none());
    }
}

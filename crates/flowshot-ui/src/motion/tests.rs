//! Motion unit tests: bezier solver, tween settle/retarget/no-drift, and the
//! staggered reveal schedule (the acceptance bar: "animation timeline
//! unit tests - progress curves, settle behavior, no drift").

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss
)]

use std::time::{Duration, Instant};

use flowshot_core::tokens::{DesignTokens, Easing};

use super::{MotionSpec, StaggerSpec, Tween, cubic_bezier, curve_from_tokens, stagger_progress};

fn tokens() -> DesignTokens {
    DesignTokens::default()
}

fn spec(easing: &str, ms: u64) -> MotionSpec {
    MotionSpec::resolve(&tokens(), easing, Duration::from_millis(ms))
}

#[test]
fn bezier_endpoints_are_exact_for_every_token_curve() {
    for easing in &tokens().easings {
        assert_eq!(cubic_bezier(easing.curve, 0.0), 0.0, "{} at 0", easing.name);
        assert_eq!(cubic_bezier(easing.curve, 1.0), 1.0, "{} at 1", easing.name);
        assert_eq!(cubic_bezier(easing.curve, -3.0), 0.0, "clamped below");
        assert_eq!(cubic_bezier(easing.curve, 7.0), 1.0, "clamped above");
        assert_eq!(cubic_bezier(easing.curve, f32::NAN), 0.0, "non-finite");
    }
}

#[test]
fn bezier_is_monotone_for_every_token_curve() {
    for easing in &tokens().easings {
        let mut previous = 0.0;
        for step in 0..=100 {
            let value = cubic_bezier(easing.curve, step as f32 / 100.0);
            assert!(
                value >= previous - 1e-6,
                "{}: {value} after {previous} at step {step}",
                easing.name
            );
            previous = value;
        }
    }
}

#[test]
fn linear_curve_is_the_identity() {
    let linear = [0.0, 0.0, 1.0, 1.0];
    for step in 0..=20 {
        let x = step as f32 / 20.0;
        assert!((cubic_bezier(linear, x) - x).abs() < 1e-4, "linear at {x}");
    }
}

#[test]
fn decelerate_leads_and_accelerate_trails_standard() {
    let tokens = tokens();
    let decel = tokens.easing("decelerate").unwrap().curve;
    let std = tokens.easing("standard").unwrap().curve;
    let accel = tokens.easing("accelerate").unwrap().curve;
    for x in [0.2f32, 0.4, 0.6, 0.8] {
        let d = cubic_bezier(decel, x);
        let s = cubic_bezier(std, x);
        let a = cubic_bezier(accel, x);
        assert!(d > s, "decelerate {d} <= standard {s} at {x}");
        assert!(s > a, "standard {s} <= accelerate {a} at {x}");
    }
}

#[test]
fn degenerate_curves_stay_finite_and_bounded() {
    let flat = [0.0, 0.0, 0.0, 0.0];
    let ones = [1.0, 1.0, 1.0, 1.0];
    for curve in [flat, ones, [f32::NAN, 0.0, 1.0, 0.0]] {
        for step in 0..=10 {
            let value = cubic_bezier(curve, step as f32 / 10.0);
            assert!(value.is_finite() && (0.0..=1.0).contains(&value));
        }
    }
}

#[test]
fn curve_lookup_falls_back_to_standard() {
    let tokens = tokens();
    assert_eq!(curve_from_tokens(&tokens, "sharp"), Easing::sharp().curve);
    assert_eq!(
        curve_from_tokens(&tokens, "nonexistent"),
        Easing::standard().curve
    );
}

#[test]
fn settled_tween_holds_its_value_and_never_schedules() {
    let t0 = Instant::now();
    let tween = Tween::settled(0.5, spec("standard", 120), t0);
    for offset in [0, 1, 60, 500] {
        let now = t0 + Duration::from_millis(offset);
        assert_eq!(tween.value_at(now), 0.5);
        assert!(!tween.active_at(now));
    }
    assert!(tween.deadline().is_none());
}

#[test]
fn tween_settles_exactly_on_its_target_without_drift() {
    let t0 = Instant::now();
    let mut tween = Tween::settled(0.0, spec("standard", 120), t0);
    tween.retarget(1.0, spec("standard", 120), t0);
    // Mid-flight: strictly between the endpoints, still moving.
    let mid = t0 + Duration::from_millis(60);
    let value = tween.value_at(mid);
    assert!(value > 0.0 && value < 1.0, "mid value {value}");
    assert!(tween.active_at(mid));
    assert_eq!(tween.deadline(), Some(t0 + Duration::from_millis(120)));
    // At and far past the deadline: EXACTLY the target (no asymptotic drift).
    assert_eq!(tween.value_at(t0 + Duration::from_millis(120)), 1.0);
    assert_eq!(tween.value_at(t0 + Duration::from_secs(60)), 1.0);
    assert!(!tween.active_at(t0 + Duration::from_millis(120)));
    assert!(!tween.active_at(t0 + Duration::from_secs(60)));
}

#[test]
fn retarget_midflight_continues_from_the_current_value() {
    let t0 = Instant::now();
    let mut tween = Tween::settled(0.0, spec("standard", 100), t0);
    tween.retarget(1.0, spec("standard", 100), t0);
    let mid = t0 + Duration::from_millis(50);
    let shown = tween.value_at(mid);
    tween.retarget(0.0, spec("standard", 100), mid);
    // Continuity: the first sample after the retarget is the shown value.
    assert_eq!(tween.value_at(mid), shown);
    assert_eq!(tween.value_at(mid + Duration::from_millis(100)), 0.0);
}

#[test]
fn retarget_to_the_same_target_is_a_noop() {
    let t0 = Instant::now();
    let mut tween = Tween::settled(0.0, spec("sharp", 100), t0);
    tween.retarget(1.0, spec("sharp", 100), t0);
    let mid = t0 + Duration::from_millis(50);
    let before = tween;
    // A per-frame tick with an unchanged target must not restart the tween.
    tween.retarget(1.0, spec("sharp", 100), mid);
    assert_eq!(tween, before);
}

#[test]
fn instant_spec_snaps_and_schedules_nothing() {
    // The reduced-motion path: zero duration = target immediately, never
    // active, no deadline (the frame scheduler stays in ControlFlow::Wait).
    let t0 = Instant::now();
    let mut tween = Tween::settled(0.0, MotionSpec::instant(), t0);
    tween.retarget(1.0, MotionSpec::instant(), t0);
    assert_eq!(tween.value_at(t0), 1.0);
    assert!(!tween.active_at(t0));
    assert!(!tween.active_at(t0 + Duration::from_millis(1)));
    assert!(tween.deadline().is_none());
}

#[test]
fn times_before_the_start_evaluate_to_the_from_value() {
    let t0 = Instant::now();
    let mut tween = Tween::settled(0.25, spec("standard", 100), t0);
    tween.retarget(1.0, spec("standard", 100), t0);
    // Synthetic clocks (the offscreen QA harness) may sample before start.
    let before = t0.checked_sub(Duration::from_millis(50)).unwrap_or(t0);
    assert_eq!(tween.value_at(before), 0.25);
}

fn reveal_spec() -> StaggerSpec {
    StaggerSpec::toolbar_reveal(&tokens())
}

#[test]
fn stagger_first_element_starts_immediately_last_ends_at_total() {
    let s = reveal_spec();
    let count = 5;
    assert_eq!(stagger_progress(Duration::ZERO, 0, count, &s), 0.0);
    assert!(stagger_progress(Duration::from_millis(10), 0, count, &s) > 0.0);
    // The last element finishes exactly at `total`.
    assert_eq!(
        stagger_progress(s.total, count - 1, count, &s),
        1.0,
        "last element at total"
    );
    // ... and has not started at zero.
    assert_eq!(stagger_progress(Duration::ZERO, count - 1, count, &s), 0.0);
}

#[test]
fn stagger_ordering_is_monotone_in_index_and_time() {
    let s = reveal_spec();
    let count = 6;
    let at = Duration::from_millis(100);
    let mut previous = f32::MAX;
    for index in 0..count {
        let progress = stagger_progress(at, index, count, &s);
        assert!(
            progress <= previous + 1e-6,
            "index {index} ahead of its predecessor"
        );
        previous = progress;
    }
    let mut previous = 0.0;
    for step in 0..=36 {
        let progress = stagger_progress(Duration::from_millis(step * 5), 2, count, &s);
        assert!(
            progress >= previous - 1e-6,
            "time regression at step {step}"
        );
        previous = progress;
    }
}

#[test]
fn stagger_completes_whole_after_total_and_handles_single_element() {
    let s = reveal_spec();
    for index in 0..7 {
        assert_eq!(stagger_progress(s.total, index, 7, &s), 1.0);
        assert_eq!(
            stagger_progress(s.total + Duration::from_millis(1), index, 7, &s),
            1.0
        );
    }
    // One element: unstaggered, runs for `element`.
    let half = s.element / 2;
    let mid = stagger_progress(half, 0, 1, &s);
    assert!(mid > 0.0 && mid < 1.0, "single element mid {mid}");
    assert_eq!(stagger_progress(s.element, 0, 1, &s), 1.0);
    // Zero count never divides by zero.
    assert_eq!(stagger_progress(half, 0, 0, &s), 1.0);
}

#[test]
fn reduced_motion_specs_resolve_to_instant() {
    // The reduced-motion contract at the spec level: instant() has zero
    // duration, so every consumer snaps (see instant_spec_snaps above).
    let instant = MotionSpec::instant();
    assert!(instant.duration.is_zero());
    let mut tween = Tween::settled(0.0, instant, Instant::now());
    tween.retarget(2.0, instant, Instant::now());
    assert_eq!(tween.value_at(Instant::now()), 2.0);
}

//! Retargetable value tweens and the staggered-reveal progress function.
//!
//! A [`Tween`] is a PURE function of `(start, now)`: it stores its endpoints,
//! start instant, and [`MotionSpec`], and every consumer evaluates it with
//! the `now` it was handed (the shell's `Instant::now()`, the offscreen QA
//! harness's synthetic clock). Nothing ticks in the background, so a settled
//! tween costs zero CPU and zero frames (the shell's idle contract).

use std::time::{Duration, Instant};

use flowshot_core::tokens::DesignTokens;

use super::easing::{cubic_bezier, curve_from_tokens};

/// Target-equality threshold: retargeting to the (numerically) same value is
/// a no-op, so per-frame `tick` calls never restart a running transition.
const TARGET_EPSILON: f64 = 1e-9;

/// An eased transition recipe: how long, along which token bezier curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpec {
    /// Transition duration; [`Duration::ZERO`] = reduced motion (snap).
    pub duration: Duration,
    /// Resolved cubic-bezier control points `[x1, y1, x2, y2]`.
    pub curve: [f32; 4],
}

impl MotionSpec {
    /// Resolves the named token easing curve for `duration`.
    #[must_use]
    pub fn resolve(tokens: &DesignTokens, easing: &str, duration: Duration) -> Self {
        Self {
            duration,
            curve: curve_from_tokens(tokens, easing),
        }
    }

    /// The reduced-motion spec: zero duration, every tween snaps to its
    /// target and never schedules a frame.
    #[must_use]
    pub const fn instant() -> Self {
        Self {
            duration: Duration::ZERO,
            curve: [0.0, 0.0, 1.0, 1.0],
        }
    }
}

/// One eased value transition between two `f64` endpoints, retargetable
/// mid-flight without visual jumps (the new tween starts at the CURRENT
/// eased value).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween {
    from: f64,
    to: f64,
    start: Instant,
    spec: MotionSpec,
}

impl Tween {
    /// A tween at rest at `value`.
    #[must_use]
    pub fn settled(value: f64, spec: MotionSpec, now: Instant) -> Self {
        Self {
            from: value,
            to: value,
            start: now,
            spec,
        }
    }

    /// Animates toward `to` under `spec`, starting from the value the tween
    /// shows at `now` (continuity: a retarget mid-flight never jumps). A
    /// numerically identical target under the same spec is a no-op, so
    /// calling this every frame is safe.
    pub fn retarget(&mut self, to: f64, spec: MotionSpec, now: Instant) {
        if (self.to - to).abs() <= TARGET_EPSILON && self.spec == spec {
            return;
        }
        self.from = self.value_at(now);
        self.to = to;
        self.start = now;
        self.spec = spec;
    }

    /// The eased value at `now`. Times before the start evaluate to `from`
    /// (synthetic-clock safety); at or past the deadline the value is
    /// EXACTLY `to` (no asymptotic drift).
    #[must_use]
    pub fn value_at(&self, now: Instant) -> f64 {
        if self.spec.duration.is_zero() {
            return self.to;
        }
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= self.spec.duration {
            return self.to;
        }
        let progress = elapsed.as_secs_f64() / self.spec.duration.as_secs_f64();
        let eased = f64::from(cubic_bezier(
            self.spec.curve,
            crate::render::f32_from_f64(progress),
        ));
        self.from + (self.to - self.from) * eased
    }

    /// Whether the value is still moving at `now` (a settled or
    /// zero-duration tween is never active - the frame scheduler's
    /// damage test).
    #[must_use]
    pub fn active_at(&self, now: Instant) -> bool {
        self.moving() && now < self.end()
    }

    /// The instant the tween settles, when it is moving.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.moving().then(|| self.end())
    }

    fn moving(&self) -> bool {
        !self.spec.duration.is_zero() && (self.to - self.from).abs() > TARGET_EPSILON
    }

    fn end(&self) -> Instant {
        // Millisecond-scale durations never overflow `Instant`; a saturated
        // platform clock settles immediately rather than panicking.
        self.start
            .checked_add(self.spec.duration)
            .unwrap_or(self.start)
    }
}

/// A staggered multi-element reveal recipe (the toolbar's per-button
/// fade+slide): element `i` of `n` starts at `i * step` with
/// `step = (total - element) / (n - 1)`, so the FIRST element starts at zero
/// and the LAST finishes exactly at `total`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaggerSpec {
    /// One element's transition duration.
    pub element: Duration,
    /// The whole reveal's duration (>= `element`).
    pub total: Duration,
    /// Resolved cubic-bezier control points.
    pub curve: [f32; 4],
}

/// The eased progress of element `index` of `count` at `elapsed` since the
/// reveal started: `0.0` before the element's stagger delay, `1.0` at or
/// past its end, eased in between. `count <= 1` runs the single element
/// unstaggered; a zero `element` duration snaps to `1.0`.
#[must_use]
pub fn stagger_progress(elapsed: Duration, index: usize, count: usize, spec: &StaggerSpec) -> f32 {
    if spec.element.is_zero() || elapsed >= spec.total || count == 0 {
        return 1.0;
    }
    let spread = spec.total.saturating_sub(spec.element);
    let steps = u32::try_from(count.saturating_sub(1)).unwrap_or(u32::MAX);
    let delay = if steps == 0 {
        Duration::ZERO
    } else {
        spread / steps
    };
    let start = delay.saturating_mul(u32::try_from(index).unwrap_or(u32::MAX));
    if elapsed <= start {
        return 0.0;
    }
    let local = elapsed.saturating_sub(start).min(spec.element);
    let progress = local.as_secs_f64() / spec.element.as_secs_f64();
    cubic_bezier(spec.curve, crate::render::f32_from_f64(progress))
}

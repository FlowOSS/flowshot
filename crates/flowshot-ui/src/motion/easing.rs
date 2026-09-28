//! Cubic-bezier easing evaluation (draft D8(d)).
//!
//! The curves are the `flowshot_core::tokens::Easing` control points
//! (`[x1, y1, x2, y2]`, the CSS `cubic-bezier()` convention): for a linear
//! progress `x` (elapsed / duration), the solver finds the bezier parameter
//! `t` with `x(t) = x` and returns `y(t)`. Newton-Raphson with a bisection
//! fallback keeps it branch-predictable and total - every input produces a
//! finite `[0, 1]`-domain output, degenerate curves included.

use flowshot_core::tokens::{DesignTokens, Easing};

/// Solver epsilon (bezier parameter space); far below a pixel of visual
/// difference at any duration the token spec uses.
const EPSILON: f32 = 1e-5;
/// Newton-Raphson iteration cap.
const NEWTON_ITERS: u32 = 8;
/// Bisection iteration cap (fallback; 20 halvings = ~1e-6 resolution).
const BISECT_ITERS: u32 = 20;

/// Evaluates the cubic-bezier `curve` (`[x1, y1, x2, y2]`) at linear
/// progress `x`, returning the eased progress in `[0, 1]` for token curves
/// (y control points inside the unit range). Non-finite input eases to the
/// nearest endpoint; a non-finite curve degrades to linear.
#[must_use]
pub fn cubic_bezier(curve: [f32; 4], x: f32) -> f32 {
    if !x.is_finite() || x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let [x1, y1, x2, y2] = sanitize(curve);
    let t = solve_t(x1, x2, x);
    bezier_component(y1, y2, t).clamp(0.0, 1.0)
}

/// Resolves a named token easing curve; a missing or malformed name falls
/// back to the `standard` curve (the token set's own default).
#[must_use]
pub fn curve_from_tokens(tokens: &DesignTokens, name: &str) -> [f32; 4] {
    tokens
        .easing(name)
        .map_or_else(|| Easing::standard().curve, |easing| easing.curve)
}

/// Clamps the x control points into `[0, 1]` (monotonicity requirement) and
/// degrades a non-finite curve to linear.
fn sanitize(curve: [f32; 4]) -> [f32; 4] {
    let [x1, y1, x2, y2] = curve;
    if !curve.iter().all(|c| c.is_finite()) {
        return [0.0, 0.0, 1.0, 1.0];
    }
    [x1.clamp(0.0, 1.0), y1, x2.clamp(0.0, 1.0), y2]
}

/// One bezier component in Bernstein form with fixed endpoints 0 and 1:
/// `3(1-t)^2 t c1 + 3(1-t) t^2 c2 + t^3`.
fn bezier_component(c1: f32, c2: f32, t: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * t * c1 + 3.0 * inv * t * t * c2 + t * t * t
}

/// Derivative of the x component: `3(1-t)^2 x1 + 6(1-t)t (x2-x1) + 3t^2 (1-x2)`.
fn bezier_slope(x1: f32, x2: f32, t: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * x1 + 6.0 * inv * t * (x2 - x1) + 3.0 * t * t * (1.0 - x2)
}

/// Finds the bezier parameter `t` whose x component equals `progress`.
fn solve_t(x1: f32, x2: f32, progress: f32) -> f32 {
    // Newton-Raphson from the diagonal guess; falls back to bisection the
    // moment the iteration leaves the parameter range or stalls on a flat
    // derivative (deenerate straight-line curves).
    let mut t = progress;
    for _ in 0..NEWTON_ITERS {
        let error = bezier_component(x1, x2, t) - progress;
        if error.abs() < EPSILON {
            return t;
        }
        let slope = bezier_slope(x1, x2, t);
        if slope.abs() < 1e-6 {
            break;
        }
        let next = t - error / slope;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        t = next;
    }
    bisect(x1, x2, progress)
}

/// Bisection fallback: total and monotone (the x component is non-decreasing
/// for x control points in `[0, 1]`).
fn bisect(x1: f32, x2: f32, progress: f32) -> f32 {
    let mut low = 0.0;
    let mut high = 1.0;
    let mut t = progress;
    for _ in 0..BISECT_ITERS {
        let value = bezier_component(x1, x2, t);
        if (value - progress).abs() < EPSILON {
            return t;
        }
        if value < progress {
            low = t;
        } else {
            high = t;
        }
        t = f32::midpoint(low, high);
    }
    t
}

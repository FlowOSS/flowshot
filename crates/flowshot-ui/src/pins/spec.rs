//! Pin behavior constants (draft F27 pin spec, Flameshot `pinwidget.cpp`
//! anonymous namespace @2d478061 - clean-room reimplementation).
//!
//! These are the pin surface's design tokens. `flowshot-core::tokens` owns
//! the shared palette/spacing/shadow scales; the pin geometry constants live
//! here because they are behavior spec, not theme (F27 BORROW list). The
//! `[pin].min_size` config key overrides [`MIN_SIZE`] at runtime.

/// Frame margin around the image, in logical px (F27 `MARGIN = 7`).
///
/// The pin window is `content + 2 * margin` per axis so the drop shadow has
/// room to render; converted to physical px once per window scale factor
/// (physical-first rule, #4920 fix).
pub const MARGIN: f64 = 7.0;

/// Drop-shadow blur radius in logical px (F27 `BLUR_RADIUS = 2 * MARGIN`).
pub const BLUR_RADIUS: f64 = 14.0;

/// Multiplicative zoom step per committed wheel notch (F27 `STEP = 0.03`).
///
/// Flameshot accumulates `+= STEP` into a per-event step factor and never
/// commits it on Linux discrete wheels (additive drift, asymmetric in/out).
/// `FlowShot` commits multiplicatively - zoom in `scale *= 1 + STEP`, zoom out
/// `scale /= 1 + STEP` - so five notches give exactly `1.03^5` and in/out
/// round-trips are lossless (the acceptance math).
pub const ZOOM_STEP: f64 = 0.03;

/// Wheel accumulator units per committed zoom step.
///
/// One discrete wheel notch is 120 units (the `Qt`/`wl_pointer` convention);
/// high-resolution trackpad deltas accumulate until a full notch is reached
/// (F27 "accumulate-then-commit"). Flameshot's sign-only accumulation (every
/// scroll *event* zooms a full step regardless of magnitude) is AVOIDED.
pub const WHEEL_UNITS_PER_STEP: f64 = 120.0;

/// Wheel units contributed by one physical px of touchpad `PixelDelta`.
///
/// winit delivers high-resolution touchpad scrolls as pixel deltas; 60 px of
/// accumulated scroll commits one zoom step (half a discrete notch's units
/// per 30 px - tuned so a deliberate two-finger flick zooms about as far as
/// a wheel notch).
pub const WHEEL_UNITS_PER_PX: f64 = 2.0;

/// Minimum content size per axis in physical px (F27 `MIN_SIZE = 100`,
/// `[pin].min_size` config default). Small images start magnified to the
/// floor; zoom-out clamps at it (Flameshot `qBound(MIN_SIZE, ...)` parity).
pub const MIN_SIZE: f64 = 100.0;

/// Opacity granularity: keys `0-9` map to tenths `10, 1, .., 9` and the
/// context menu moves one tenth per click (F27 "opacity keys 0-9 = 1.0-0.1,
/// menu +/-0.1"). Stored as integer tenths so repeated menu steps never
/// drift (`0.7 - 0.1 != 0.6` in binary floating point).
pub const OPACITY_TENTHS: u8 = 10;

/// Double-click interval for the close-on-double-click parity behavior
/// (Flameshot `mouseDoubleClickEvent` -> `closePin`).
pub const DOUBLE_CLICK_MS: u64 = 400;

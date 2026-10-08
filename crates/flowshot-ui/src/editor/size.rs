//! Per-tool size dispatch and the two size adjusters (Flameshot).
//!
//! Flameshot size model (`confighandler.cpp` `setToolSize`/`toolSize`): text uses
//! the font size (rendered at `size + ` [`BASE_POINT_SIZE`]),
//! rectangle/marker/pixelate/counter own independent `[tools.*]` slots, and
//! every other tool shares `[editor].draw_thickness` (default 3). Digit keys
//! accumulate `10 * acc + digit` clipped to [`MAX_TOOL_SIZE`] (Flameshot
//! `m_toolSizeByKeyboard` + `qBound(1, size, maxToolSize=50)`, accumulator
//! reset on clip AND on the notifier-box timeout - [`DIGIT_RESET_DELAY`],
//! `notifierbox.cpp` 600ms). The wheel adjusts ±1 per
//! [`WHEEL_THRESHOLD`]-unit notch (Flameshot `MOUSE_WHEEL_TRESHOLD = 60`,
//! `capturewidget.cpp` L46/L1140).
//!
//! BORROW-MODIFIED (wheel): Flameshot rate-limits sub-threshold (trackpad)
//! deltas to one step per 200ms; `FlowShot` ACCUMULATES them to the 60-unit
//! threshold instead (the plan's "60-delta accumulation threshold") - a
//! standard 120-unit notch still yields exactly ±1 (Flameshot clamps
//! `120/60 = 2` to 1), and fast trackpad scrolls sum honestly instead of
//! being dropped by a wall-clock window.

use std::time::{Duration, Instant};

use super::kind::ToolKind;
use super::tool::EditorTools;

/// `maxToolSize` (Flameshot): the hard clip for every tool size.
pub const MAX_TOOL_SIZE: u32 = 50;
/// The minimum tool size (`qBound(1, ..)` parity).
pub const MIN_TOOL_SIZE: u32 = 1;
/// `MOUSE_WHEEL_TRESHOLD` (Flameshot, `capturewidget.cpp` L46): angle-delta units
/// per ±1 size step.
pub const WHEEL_THRESHOLD: i32 = 60;
/// Qt wheel-angle units per line (a standard notch = 120 units = 3 lines;
/// the winit `LineDelta` -> angle conversion of the shell).
pub const WHEEL_ANGLE_PER_LINE: f32 = 40.0;
/// The digit-accumulator reset delay: Flameshot's notifier box hides 600ms
/// after the last size change (`notifierbox.cpp` `setInterval(600)`) and its
/// `hidden` signal zeroes `m_toolSizeByKeyboard`.
pub const DIGIT_RESET_DELAY: Duration = Duration::from_millis(600);
/// Text tools render at `tool_size + BASE_POINT_SIZE` points (Flameshot text spec;
/// consumed by the text tool).
pub const BASE_POINT_SIZE: u32 = 8;

/// The per-tool size slots (Flameshot dispatch table), projected from the config
/// and adjusted at runtime by digits/wheel/panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSizes {
    /// Shared `[editor].draw_thickness` (pencil/line/arrow/circle/...).
    pub thickness: u32,
    /// `[editor].font_size` (text; rendered at `+ BASE_POINT_SIZE`).
    pub font: u32,
    /// `[tools.rectangle].corner_radius`.
    pub rect_radius: u32,
    /// `[tools.marker].size`.
    pub marker: u32,
    /// `[tools.pixelate].size`.
    pub pixelate: u32,
    /// `[tools.counter].size`.
    pub counter: u32,
}

impl ToolSizes {
    /// Projects the slots from the config groups.
    #[must_use]
    pub const fn from_config(config: &EditorTools) -> Self {
        Self {
            thickness: config.editor.draw_thickness,
            font: config.editor.font_size,
            rect_radius: config.tools.rectangle.corner_radius,
            marker: config.tools.marker.size,
            pixelate: config.tools.pixelate.size,
            counter: config.tools.counter.size,
        }
    }

    /// The dispatched size for a tool kind (`None` = no active tool = the
    /// shared-thickness slot, Flameshot's single `context.toolSize`).
    #[must_use]
    pub const fn get(&self, kind: Option<ToolKind>) -> u32 {
        match kind {
            Some(ToolKind::Text) => self.font,
            Some(ToolKind::Marker) => self.marker,
            Some(ToolKind::Pixelate | ToolKind::Blur) => self.pixelate,
            Some(ToolKind::Counter) => self.counter,
            Some(ToolKind::Rectangle | _) | None => self.thickness,
        }
    }

    /// Writes a size back to the slot the kind dispatches to (the
    /// `ConfigHandler::setToolSize` switch parity).
    pub fn set(&mut self, kind: Option<ToolKind>, value: u32) {
        let slot = match kind {
            Some(ToolKind::Text) => &mut self.font,
            Some(ToolKind::Marker) => &mut self.marker,
            Some(ToolKind::Pixelate | ToolKind::Blur) => &mut self.pixelate,
            Some(ToolKind::Counter) => &mut self.counter,
            Some(ToolKind::Rectangle | _) | None => &mut self.thickness,
        };
        *slot = value.clamp(MIN_TOOL_SIZE, MAX_TOOL_SIZE);
    }
}

/// The digit-key size accumulator (Flameshot `m_toolSizeByKeyboard`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DigitAccumulator {
    acc: u32,
    last: Option<Instant>,
}

impl DigitAccumulator {
    /// Folds one digit in: `acc = 10 * acc + digit`, size = `acc` clipped to
    /// `[1, 50]`; the accumulator resets when the value clipped (Flameshot
    /// parity: `'9','9' -> 50`, next digit starts fresh) or when more than
    /// [`DIGIT_RESET_DELAY`] passed since the previous digit (the notifier
    /// box timeout).
    pub fn digit(&mut self, digit: u32, now: Instant) -> u32 {
        let stale = self.last.is_none_or(|last| {
            now.checked_duration_since(last)
                .is_none_or(|age| age > DIGIT_RESET_DELAY)
        });
        if stale {
            self.acc = 0;
        }
        self.acc = self.acc.saturating_mul(10).saturating_add(digit);
        let size = self.acc.clamp(MIN_TOOL_SIZE, MAX_TOOL_SIZE);
        if size != self.acc {
            self.acc = 0;
        }
        self.last = Some(now);
        size
    }

    /// Drops any pending accumulation (tool switch / notifier hide).
    pub fn reset(&mut self) {
        self.acc = 0;
        self.last = None;
    }
}

/// The wheel size accumulator ([`WHEEL_THRESHOLD`] units per ±1 step).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WheelAccumulator {
    acc: i32,
}

impl WheelAccumulator {
    /// Folds one wheel angle-delta in; returns the ±1 step when the
    /// accumulation crossed the threshold (resetting the accumulator - a
    /// 120-unit notch yields exactly one step, sub-threshold trackpad
    /// deltas sum until they do).
    pub fn wheel(&mut self, delta: i32) -> Option<i32> {
        self.acc = self.acc.saturating_add(delta);
        if self.acc >= WHEEL_THRESHOLD {
            self.acc = 0;
            Some(1)
        } else if self.acc <= -WHEEL_THRESHOLD {
            self.acc = 0;
            Some(-1)
        } else {
            None
        }
    }
}

/// Applies a ±step to a size, clipped to the tool-size range.
#[must_use]
pub fn stepped(size: u32, step: i32) -> u32 {
    let next = i64::from(size) + i64::from(step);
    u32::try_from(next.clamp(i64::from(MIN_TOOL_SIZE), i64::from(MAX_TOOL_SIZE)))
        .unwrap_or(MAX_TOOL_SIZE)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp
    )]

    use flowshot_core::config::{Config, EditorConfig, ToolsConfig};

    use super::*;

    fn sizes() -> ToolSizes {
        ToolSizes::from_config(&EditorTools::from_config(&Config::default()))
    }

    #[test]
    fn dispatch_table_matches_the_config_defaults() {
        let s = sizes();
        // Flameshot: text = fontSize, rect/marker/pixelate/counter independent,
        // others shared drawThickness (default 3).
        assert_eq!(s.get(Some(ToolKind::Text)), 8);
        assert_eq!(
            s.get(Some(ToolKind::Rectangle)),
            3,
            "rectangle uses stroke thickness"
        );
        assert_eq!(s.get(Some(ToolKind::Marker)), 5);
        assert_eq!(s.get(Some(ToolKind::Pixelate)), 2);
        assert_eq!(s.get(Some(ToolKind::Counter)), 1);
        for kind in [
            ToolKind::Pencil,
            ToolKind::Line,
            ToolKind::Arrow,
            ToolKind::Circle,
            ToolKind::Invert,
            ToolKind::Selection,
            ToolKind::Move,
        ] {
            assert_eq!(s.get(Some(kind)), 3, "{kind:?} shares draw_thickness");
        }
        assert_eq!(s.get(None), 3);
    }

    #[test]
    fn set_writes_back_to_the_dispatched_slot_only() {
        let mut s = sizes();
        s.set(Some(ToolKind::Marker), 12);
        assert_eq!(s.get(Some(ToolKind::Marker)), 12);
        assert_eq!(s.get(Some(ToolKind::Pencil)), 3, "shared slot untouched");
        s.set(Some(ToolKind::Pencil), 7);
        assert_eq!(s.get(Some(ToolKind::Line)), 7, "shared slot");
        assert_eq!(s.get(Some(ToolKind::Text)), 8);
        // Clips into [1, 50].
        s.set(Some(ToolKind::Text), 99);
        assert_eq!(s.get(Some(ToolKind::Text)), MAX_TOOL_SIZE);
        s.set(Some(ToolKind::Text), 0);
        assert_eq!(s.get(Some(ToolKind::Text)), MIN_TOOL_SIZE);
    }

    #[test]
    fn from_config_tracks_non_default_groups() {
        let config = Config {
            editor: EditorConfig {
                draw_thickness: 9,
                font_size: 20,
                ..EditorConfig::default()
            },
            tools: ToolsConfig {
                marker: flowshot_core::config::MarkerToolConfig { size: 30 },
                ..ToolsConfig::default()
            },
            ..Config::default()
        };
        let s = ToolSizes::from_config(&EditorTools::from_config(&config));
        assert_eq!(s.get(Some(ToolKind::Pencil)), 9);
        assert_eq!(s.get(Some(ToolKind::Text)), 20);
        assert_eq!(s.get(Some(ToolKind::Marker)), 30);
    }

    #[test]
    fn digits_accumulate_shift_and_clip_at_fifty() {
        let mut acc = DigitAccumulator::default();
        let t0 = Instant::now();
        // Acceptance failure case: '9','9' -> 9 then 99 clipped to 50.
        assert_eq!(acc.digit(9, t0), 9);
        assert_eq!(acc.digit(9, t0 + Duration::from_millis(100)), MAX_TOOL_SIZE);
        // Clip reset the accumulator: the next digit starts fresh.
        assert_eq!(acc.digit(4, t0 + Duration::from_millis(200)), 4);
        // A fresh accumulator: '1','2' accumulates to 12 within the window.
        let mut acc = DigitAccumulator::default();
        assert_eq!(acc.digit(1, t0 + Duration::from_millis(300)), 1);
        assert_eq!(acc.digit(2, t0 + Duration::from_millis(400)), 12);
        // '0' first clips up to the minimum 1 and resets (qBound parity).
        let mut acc = DigitAccumulator::default();
        assert_eq!(acc.digit(0, t0), MIN_TOOL_SIZE);
        assert_eq!(acc.digit(5, t0 + Duration::from_millis(50)), 5);
    }

    #[test]
    fn digits_reset_after_the_notifier_delay() {
        let mut acc = DigitAccumulator::default();
        let t0 = Instant::now();
        assert_eq!(acc.digit(4, t0), 4);
        // 601ms later the notifier box hid: fresh accumulation.
        let later = t0 + DIGIT_RESET_DELAY + Duration::from_millis(1);
        assert_eq!(acc.digit(2, later), 2);
        // Just inside the window it still accumulates.
        let mut acc = DigitAccumulator::default();
        assert_eq!(acc.digit(4, t0), 4);
        assert_eq!(acc.digit(2, t0 + DIGIT_RESET_DELAY), 42);
    }

    #[test]
    fn wheel_steps_once_per_threshold() {
        let mut acc = WheelAccumulator::default();
        // A standard 120-unit notch = exactly one step (Flameshot clamps
        // 120/60=2 to 1 - parity).
        assert_eq!(acc.wheel(120), Some(1));
        assert_eq!(acc.wheel(-120), Some(-1));
        // Sub-threshold deltas accumulate (trackpad): 15 x 4 = one step.
        for _ in 0..3 {
            assert_eq!(acc.wheel(15), None);
        }
        assert_eq!(acc.wheel(15), Some(1));
        // Exactly at the threshold.
        assert_eq!(acc.wheel(60), Some(1));
        assert_eq!(acc.wheel(-60), Some(-1));
        // A step resets the accumulator (no runaway carry).
        assert_eq!(acc.wheel(59), None);
        assert_eq!(acc.wheel(240), Some(1));
    }

    #[test]
    fn stepped_clips_to_the_tool_range() {
        assert_eq!(stepped(3, 1), 4);
        assert_eq!(stepped(3, -1), 2);
        assert_eq!(stepped(1, -1), MIN_TOOL_SIZE);
        assert_eq!(stepped(MAX_TOOL_SIZE, 1), MAX_TOOL_SIZE);
        assert_eq!(stepped(3, i32::MAX), MAX_TOOL_SIZE);
        assert_eq!(stepped(3, i32::MIN), MIN_TOOL_SIZE);
    }
}

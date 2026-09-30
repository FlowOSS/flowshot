//! `ChromeMotion` timeline tests (staggered reveal, panel/wheel tweens,
//! hover/press wash, reduced motion, settle deadlines).

#![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::motion::{
    BUTTON_WASH_MS, HANDLE_GROW_MS, PANEL_ENTER_MS, REVEAL_TOTAL_MS, WHEEL_ENTER_MS,
};
use crate::widgets::TOOLTIP_DELAY;

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
fn tooltip_arms_after_the_delay_and_wakes_the_scheduler() {
    let (mut motion, t0) = motion(3);
    motion.set_hover(Some(2), t0);
    assert_eq!(motion.tooltip_button(t0), None, "dark during the delay");
    let after_wash = t0 + Duration::from_millis(BUTTON_WASH_MS);
    assert!(
        motion.active_at(after_wash),
        "the pending show keeps the frame scheduler awake after the wash settles"
    );
    assert_eq!(motion.settle_at(after_wash), Some(t0 + TOOLTIP_DELAY));
    let shown = t0 + TOOLTIP_DELAY;
    assert_eq!(motion.tooltip_button(shown), Some(2));
    assert!(!motion.active_at(shown), "shown: nothing left to schedule");
    assert!(motion.settle_at(shown).is_none());
}

#[test]
fn tooltip_delay_restarts_on_switch_and_press_consumes_it() {
    let (mut motion, t0) = motion(2);
    motion.set_hover(Some(0), t0);
    let switch = t0 + TOOLTIP_DELAY;
    motion.set_hover(Some(1), switch);
    assert_eq!(motion.tooltip_button(switch), None, "switch restarts");
    assert_eq!(motion.tooltip_button(switch + TOOLTIP_DELAY), Some(1));
    // A press consumes the tooltip; it stays away until re-entry.
    motion.set_press(Some(1), switch);
    motion.set_press(None, switch + Duration::from_millis(50));
    assert_eq!(motion.tooltip_button(switch + TOOLTIP_DELAY * 2), None);
    motion.set_hover(None, switch);
    motion.set_hover(Some(1), switch);
    assert_eq!(motion.tooltip_button(switch + TOOLTIP_DELAY), Some(1));
}

#[test]
fn reduced_motion_shows_the_tooltip_immediately() {
    let (mut motion, t0) = motion(2);
    motion.set_reduced(true);
    motion.set_hover(Some(1), t0);
    assert_eq!(motion.tooltip_button(t0), Some(1));
    assert!(!motion.active_at(t0));
    assert!(motion.settle_at(t0).is_none());
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
fn motion_durations_stay_in_the_documented_band() {
    // The plan's 120-180ms reveal band, compile-time enforced.
    const {
        assert!(HANDLE_GROW_MS >= 100 && HANDLE_GROW_MS <= 200);
        assert!(REVEAL_TOTAL_MS >= 120 && REVEAL_TOTAL_MS <= 180);
        assert!(WHEEL_ENTER_MS <= REVEAL_TOTAL_MS);
    }
}

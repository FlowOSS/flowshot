//! `OverlayCore` funnel + frame-scheduling tests (moved verbatim from the
//! facade at the 250-LOC ceiling; the `selection/tests.rs` discipline).

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use super::*;
use crate::input::Action;
use crate::render::{Color, Command, DisplayList};
use flowshot_core::geometry::{LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform};
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// Dual mixed-DPI fixture: DP-1 1920x1080 @ 1x at (0,0);
/// DP-2 3840x2160 @ 2x at logical (1920,0).
fn dual_core() -> OverlayCore {
    let make = |connector: &str, rect: LogicalRect, size: PhysicalSize, scale: f64| {
        OutputInfo::new(connector, connector, rect, size, scale, Transform::Normal)
            .expect("valid fixture output")
    };
    let layout = OutputLayout::new(vec![
        make(
            "DP-1",
            LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(1920, 1080),
            1.0,
        ),
        make(
            "DP-2",
            LogicalRect::from_raw(1920.0, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(3840, 2160),
            2.0,
        ),
    ]);
    OverlayCore::new(InputRouter::new(layout, vec![0, 1]))
}

#[test]
fn inject_motion_emits_global_coords() {
    // Acceptance (plan todo 13): inject motion -> router emits global coords.
    let mut core = dual_core();
    let report = core.inject_event(SyntheticInput::pointer_moved(
        WindowSlot::new(1),
        200.0,
        400.0,
    ));
    let global = report.global_position.expect("slot 1 is bound");
    assert_eq!((global.x.0, global.y.0), (2020.0, 200.0));
    assert_eq!(report.clamped_position, Some(global));
    assert_eq!(report.actions, vec![Action::Redraw(WindowSlot::new(1))]);
    let cursor = core.cursor().expect("motion tracked");
    assert_eq!(cursor.slot, WindowSlot::new(1));
    assert_eq!((cursor.local_x, cursor.local_y), (200.0, 400.0));
    assert_eq!((cursor.global.x.0, cursor.global.y.0), (2020.0, 200.0));
}

#[test]
fn inject_spanning_drag_motion_extends_across_window_boundary() {
    let mut core = dual_core();
    // Drag started on DP-1; implicit grab keeps delivering to slot 0 with
    // local x beyond the 1920px surface while logically over DP-2.
    let report = core.inject_event(SyntheticInput::pointer_moved(
        WindowSlot::new(0),
        2220.0,
        500.0,
    ));
    let global = report.global_position.unwrap();
    assert_eq!((global.x.0, global.y.0), (2220.0, 500.0));
    let owner = core.router().layout().output_at(global).expect("in layout");
    assert_eq!(owner.connector, "DP-2");
    // Clamped variant stays identical while inside the layout.
    assert_eq!(report.clamped_position, Some(global));
    // Beyond the far edge, clamped pulls back but global does not.
    let report = core.inject_event(SyntheticInput::pointer_moved(
        WindowSlot::new(0),
        5000.0,
        500.0,
    ));
    assert_eq!(report.global_position.unwrap().x.0, 5000.0);
    assert_eq!(report.clamped_position.unwrap().x.0, 3840.0);
}

#[test]
fn inject_escape_requests_exit_from_any_window() {
    let mut core = dual_core();
    let report = core.inject_event(SyntheticInput::key_press(
        WindowSlot::new(1),
        KeyCode::Escape,
    ));
    assert_eq!(report.actions, vec![Action::Exit]);
    assert!(core.exit_requested());
    // Release and repeat do not re-trigger or clear.
    let report = core.inject_event(SyntheticInput::key_release(
        WindowSlot::new(1),
        KeyCode::Escape,
    ));
    assert!(report.actions.is_empty());
    assert!(core.exit_requested());
}

#[test]
fn non_escape_keys_do_not_request_exit() {
    let mut core = dual_core();
    let report = core.inject_event(SyntheticInput::key_press(
        WindowSlot::new(0),
        KeyCode::Enter,
    ));
    assert!(report.actions.is_empty());
    assert!(!core.exit_requested());
}

#[test]
fn inject_pointer_button_routes_with_redraw() {
    let mut core = dual_core();
    let report = core.inject_event(SyntheticInput::pointer_button(
        WindowSlot::new(0),
        MouseButton::Left,
        true,
    ));
    assert_eq!(report.actions, vec![Action::Redraw(WindowSlot::new(0))]);
    assert_eq!(report.global_position, None);
}

#[test]
fn inject_ime_sequence_is_plumbed() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    core.inject_event(SyntheticInput::ime(slot, Ime::Enabled));
    assert_eq!(*core.ime(), ImeStatus::Active);
    core.inject_event(SyntheticInput::ime(
        slot,
        Ime::Preedit("ni".to_owned(), None),
    ));
    assert_eq!(*core.ime(), ImeStatus::Preedit("ni".to_owned()));
    core.inject_event(SyntheticInput::ime(
        slot,
        Ime::Preedit("nih".to_owned(), Some((3, 3))),
    ));
    assert_eq!(*core.ime(), ImeStatus::Preedit("nih".to_owned()));
    core.inject_event(SyntheticInput::ime(slot, Ime::Commit("日".to_owned())));
    assert_eq!(*core.ime(), ImeStatus::Active);
    assert_eq!(core.last_commit(), Some("日"));
    core.inject_event(SyntheticInput::ime(slot, Ime::Disabled));
    assert_eq!(*core.ime(), ImeStatus::Inactive);
    // Commit survives disable (todo 22 consumes it).
    assert_eq!(core.last_commit(), Some("日"));
}

#[test]
fn unknown_slot_injection_is_inert() {
    let mut core = dual_core();
    let report = core.inject_event(SyntheticInput::pointer_moved(
        WindowSlot::new(7),
        10.0,
        10.0,
    ));
    assert_eq!(report.global_position, None);
    assert_eq!(report.clamped_position, None);
    assert!(report.actions.is_empty());
    assert!(core.cursor().is_none());
    // Esc from an unknown slot still tears down (teardown must not depend
    // on slot bookkeeping).
    let report = core.inject_event(SyntheticInput::key_press(
        WindowSlot::new(7),
        KeyCode::Escape,
    ));
    assert_eq!(report.actions, vec![Action::Exit]);
}

#[test]
fn motion_frames_are_paced_and_settle_to_idle() {
    // The todo-13 idle contract under the todo-41 motion pass: a reveal
    // schedules paced frames, paints ONE settled frame, then the core
    // demands no wake at all (ControlFlow::Wait, zero CPU).
    let mut core = dual_core();
    let t0 = Instant::now();
    core.selection_mut()
        .set_rect(Some(LogicalRect::from_raw(100.0, 100.0, 200.0, 150.0)));
    // Rising edge: the tick that starts the reveal asks for a redraw.
    assert!(core.tick(t0));
    assert!(core.motion_active(t0));
    let wake = core.wake(t0).expect("running animation schedules frames");
    assert_eq!(wake, t0 + crate::motion::FRAME_INTERVAL);
    // Mid-flight keeps pacing.
    let mid = t0 + std::time::Duration::from_millis(90);
    assert!(core.tick(mid));
    assert_eq!(core.wake(mid), Some(mid + crate::motion::FRAME_INTERVAL));
    // Past the 180ms reveal total: settled, but ONE final frame paints
    // the resting state (was_active).
    let end = t0 + std::time::Duration::from_millis(400);
    assert!(core.tick(end), "the settled frame must paint");
    assert!(!core.motion_active(end));
    assert!(core.wake(end).is_none(), "settled motion schedules nothing");
    // The next pass is fully idle.
    assert!(!core.tick(end + std::time::Duration::from_millis(16)));
    assert!(
        core.wake(end + std::time::Duration::from_millis(16))
            .is_none()
    );
}

#[test]
fn reduced_motion_snaps_and_never_schedules_frames() {
    // The todo-41 failure QA (unit leg): reduced motion -> transitions
    // instant, no animation frames at all.
    let mut core = dual_core();
    core.set_motion_reduced(true);
    let t0 = Instant::now();
    core.selection_mut()
        .set_rect(Some(LogicalRect::from_raw(100.0, 100.0, 200.0, 150.0)));
    // The snapped reveal needs no animation frame: the event-driven
    // redraw (selection change) already paints the resting state.
    assert!(!core.tick(t0), "nothing may animate");
    assert!(core.wake(t0).is_none(), "nothing may be scheduled");
    assert!(!core.tick(t0 + std::time::Duration::from_millis(16)));
}

#[test]
fn grip_hover_from_injected_motion_schedules_settle_frames() {
    let mut core = dual_core();
    let t0 = Instant::now();
    core.selection_mut()
        .set_rect(Some(LogicalRect::from_raw(100.0, 100.0, 200.0, 150.0)));
    core.tick(t0);
    // Inject a motion onto the top-left handle (global 100,100 -> slot 0
    // local 100,100): the grip hover-grow starts and schedules its
    // 120ms settle deadline (paced below it).
    core.inject_event(SyntheticInput::pointer_moved(
        WindowSlot::new(0),
        100.0,
        100.0,
    ));
    let now = Instant::now();
    assert!(core.selection().motion_active(now));
    let wake = core.wake(now).expect("grip motion schedules");
    assert!(wake <= now + crate::motion::FRAME_INTERVAL);
    // Past BOTH the grip grow (120ms) and the reveal total (180ms) the
    // core is settled and idle again.
    assert!(
        !core.selection().motion_active(
            now + std::time::Duration::from_millis(crate::motion::HANDLE_GROW_MS + 1)
        )
    );
    let end = now + std::time::Duration::from_millis(crate::motion::REVEAL_TOTAL_MS + 1);
    assert!(!core.motion_active(end), "grip and reveal settled");
    core.tick(end);
    assert!(core.wake(end).is_none());
}

#[test]
fn non_finite_motion_leaves_cursor_state_untouched() {
    let mut core = dual_core();
    core.inject_event(SyntheticInput::pointer_moved(
        WindowSlot::new(0),
        10.0,
        20.0,
    ));
    let before = core.cursor().copied();
    let report = core.inject_event(SyntheticInput::pointer_moved(
        WindowSlot::new(0),
        f64::NAN,
        0.0,
    ));
    assert_eq!(report.global_position, None);
    assert!(report.actions.is_empty());
    assert_eq!(core.cursor().copied(), before);
}

/// The selection outline color the core's engine paints (first stroke).
fn outline_color(core: &OverlayCore) -> Color {
    let output = OutputInfo::new(
        "DP-1",
        "DP-1",
        LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
        PhysicalSize::from_raw(1920, 1080),
        1.0,
        Transform::Normal,
    )
    .expect("valid fixture output");
    let mut list = DisplayList::new();
    core.selection()
        .paint_into(&mut list, &output, Instant::now());
    list.iter()
        .find_map(|command| match command {
            Command::Stroke { color, .. } => Some(*color),
            _ => None,
        })
        .expect("outline stroke painted")
}

#[test]
fn configure_chrome_rethemes_the_selection_accent() {
    let mut core = dual_core();
    core.selection_mut()
        .set_rect(Some(LogicalRect::from_raw(100.0, 100.0, 200.0, 150.0)));
    assert_eq!(
        outline_color(&core),
        Color::from_rgba8(99, 102, 241, 255),
        "default token accent before any config projection"
    );
    let ui = flowshot_core::config::UiConfig {
        accent_color: "#FF0000".to_owned(),
        ..Default::default()
    };
    core.configure_chrome(&ui);
    assert_eq!(
        outline_color(&core),
        Color::from_rgba8(255, 0, 0, 255),
        "the [ui].accent_color projection must reach the selection paint"
    );
}

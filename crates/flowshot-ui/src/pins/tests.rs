//! Pin state-machine tests: the zoom-to-cursor anchor
//! invariant, the `screen`/`MIN_SIZE` clamps, the Flameshot opacity table, rotation
//! dimension swaps, the menu action map, and the close/drag behaviors -
//! all headless through [`PinState::on_input`] (no GPU, no window).

#![allow(clippy::float_cmp)]

use std::time::{Duration, Instant};

use winit::event::{MouseButton, TouchPhase};
use winit::keyboard::{KeyCode, ModifiersState};

use super::event::{PinEffect, PinInput};
use super::state::{PinBehavior, PinState};
use super::zoom::ResizeAnchor;

/// 400x300 image on a 1920x1080 screen at scale 1: margin 7, initial
/// window 414x314, scale bounds [1/3, 3.5533].
fn pin() -> PinState {
    PinState::new((400, 300), (1920, 1080), 1.0, PinBehavior::default())
}

fn route(state: &mut PinState, input: PinInput, now: Instant) -> Vec<PinEffect> {
    state.on_input(&input, now)
}

fn wheel(state: &mut PinState, notches: i32, now: Instant) -> Vec<PinEffect> {
    route(
        state,
        PinInput::Wheel {
            units: f64::from(notches) * 120.0,
        },
        now,
    )
}

fn moved(state: &mut PinState, x: f64, y: f64, now: Instant) -> Vec<PinEffect> {
    route(state, PinInput::CursorMoved { x, y }, now)
}

fn window_size_effect(effects: &[PinEffect]) -> Option<(u32, u32)> {
    effects.iter().find_map(|effect| match *effect {
        PinEffect::SetWindowSize { width, height } => Some((width, height)),
        _ => None,
    })
}

#[test]
fn initial_window_is_image_plus_frame() {
    let state = pin();
    assert_eq!(state.target_window(), (414, 314));
    assert_eq!(state.scale(), 1.0);
    assert_eq!(state.opacity_tenths(), 10);
    assert_eq!(state.image_size(), (400, 300));
}

#[test]
fn five_wheel_notches_scale_dims_by_1_03_pow_5() {
    // The acceptance math: base * 1.03^5 +/- 2 px.
    let mut state = pin();
    let t0 = Instant::now();
    moved(&mut state, 207.0, 157.0, t0);
    for _ in 0..5 {
        wheel(&mut state, 1, t0);
    }
    let expected = 1.03f64.powi(5);
    assert!((state.scale() - expected).abs() < 1e-9);
    // content = round(400 * 1.03^5) x round(300 * 1.03^5) = 464 x 348
    assert_eq!(state.target_window(), (464 + 14, 348 + 14));
}

#[test]
fn zoom_keeps_the_cursor_image_point_screen_stable() {
    // The anchor invariant, checked against an INDEPENDENT screen-space
    // model: a virtual window top-left `w` moves by the compositor's
    // center-anchor delta on every resize while the cursor screen point
    // `s` never moves; the image point under the cursor must not change.
    let mut state = pin();
    let t0 = Instant::now();
    let s = (300.0, 250.0); // screen point, off-center on purpose
    let mut w = (100.0, 100.0); // virtual window top-left
    let cursor_point = |state: &PinState, w: (f64, f64)| {
        let local = (s.0 - w.0, s.1 - w.1);
        let margin = state.margin_px();
        let (ox, oy) = state.offset();
        (
            (local.0 - margin - ox) / state.scale(),
            (local.1 - margin - oy) / state.scale(),
        )
    };
    moved(&mut state, s.0 - w.0, s.1 - w.1, t0);
    let p0 = cursor_point(&state, w);
    assert!((p0.0 - 193.0).abs() < 1e-9 && (p0.1 - 143.0).abs() < 1e-9);
    for step in 0..8 {
        let notches = if step % 2 == 0 { 1 } else { -1 };
        let old = state.target_window();
        let effects = wheel(&mut state, notches, t0);
        let Some(new) = window_size_effect(&effects) else {
            continue; // clamped: nothing moved, invariant trivially holds
        };
        // Hyprland center-anchor: the top-left shifts by (old - new) / 2.
        w = (
            w.0 + (f64::from(old.0) - f64::from(new.0)) / 2.0,
            w.1 + (f64::from(old.1) - f64::from(new.1)) / 2.0,
        );
        let p = cursor_point(&state, w);
        assert!(
            (p.0 - p0.0).abs() < 1e-6 && (p.1 - p0.1).abs() < 1e-6,
            "step {step}: anchor drifted {p:?} != {p0:?}"
        );
    }
}

#[test]
fn default_behavior_never_client_resizes() {
    // The Wayland-safety contract: `client_resize` opts the shell into the
    // X11 ConfigureRequest path; the default must stay false so a Wayland
    // session can never hit winit's stateless-window client-side resize
    // (no `Resized` event -> the render surface desyncs from the window).
    assert!(!PinBehavior::default().client_resize);
}

#[test]
fn top_left_anchor_policy_holds_too() {
    let behavior = PinBehavior {
        anchor: ResizeAnchor::TopLeft,
        ..PinBehavior::default()
    };
    let mut state = PinState::new((400, 300), (1920, 1080), 1.0, behavior);
    let t0 = Instant::now();
    let (s, w) = ((300.0, 250.0), (100.0, 100.0));
    moved(&mut state, s.0 - w.0, s.1 - w.1, t0);
    let margin = state.margin_px();
    let (ox, oy) = state.offset();
    let p0 = (
        (s.0 - w.0 - margin - ox) / state.scale(),
        (s.1 - w.1 - margin - oy) / state.scale(),
    );
    wheel(&mut state, 3, t0);
    // Top-left policy: w never moves.
    let (ox, oy) = state.offset();
    let p = (
        (s.0 - w.0 - margin - ox) / state.scale(),
        (s.1 - w.1 - margin - oy) / state.scale(),
    );
    assert!((p.0 - p0.0).abs() < 1e-6 && (p.1 - p0.1).abs() < 1e-6);
}

#[test]
fn oversized_image_starts_clamped_to_screen() {
    // Failure QA (unit half): 3000x2000 on 1920x1080.
    let mut state = PinState::new((3000, 2000), (1920, 1080), 1.0, PinBehavior::default());
    let (w, h) = state.target_window();
    assert!(w <= 1920 && h <= 1080, "window {w}x{h} exceeds the screen");
    assert_eq!(h, 1080); // height-limited fit, frame included
    // Zoom-in is already at the ceiling: inert.
    let t0 = Instant::now();
    assert_eq!(
        wheel(&mut state, 1, t0),
        [] as [crate::pins::event::PinEffect; 0]
    );
}

#[test]
fn tiny_image_starts_magnified_to_the_min_size_floor() {
    let state = PinState::new((40, 20), (1920, 1080), 1.0, PinBehavior::default());
    assert!((state.scale() - 5.0).abs() < 1e-9);
    assert_eq!(state.target_window(), (200 + 14, 100 + 14));
}

#[test]
fn zoom_out_stops_at_the_min_size_floor() {
    let mut state = PinState::new((40, 20), (1920, 1080), 1.0, PinBehavior::default());
    let t0 = Instant::now();
    assert_eq!(
        wheel(&mut state, -1, t0),
        [] as [crate::pins::event::PinEffect; 0]
    );
    // From scale 1 a 400x300 image floors at 1/3 (both axes >= 100).
    let mut state = pin();
    for _ in 0..40 {
        wheel(&mut state, -1, t0);
    }
    assert!((state.scale() - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(
        wheel(&mut state, -1, t0),
        [] as [crate::pins::event::PinEffect; 0]
    );
    // The floor binds the SMALLER axis to exactly MIN_SIZE.
    assert!((f64::from(300) * state.scale() - 100.0).abs() < 1e-9);
}

#[test]
fn opacity_keys_follow_the_f27_table() {
    let mut state = pin();
    let t0 = Instant::now();
    // Key 5 -> 0.5 (the live-QA acceptance value).
    let effects = route(
        &mut state,
        PinInput::Key {
            code: KeyCode::Digit5,
            pressed: true,
            repeat: false,
        },
        t0,
    );
    assert_eq!(state.opacity_tenths(), 5);
    assert!((state.opacity() - 0.5).abs() < 1e-6);
    assert_eq!(effects, vec![PinEffect::Reupload, PinEffect::Redraw]);
    // Key 0 -> 1.0, key 9 -> 0.9, key 1 -> 0.1 (absolute table).
    for (code, tenths) in [
        (KeyCode::Digit0, 10u8),
        (KeyCode::Digit9, 9),
        (KeyCode::Digit1, 1),
        (KeyCode::Numpad7, 7),
    ] {
        route(
            &mut state,
            PinInput::Key {
                code,
                pressed: true,
                repeat: false,
            },
            t0,
        );
        assert_eq!(state.opacity_tenths(), tenths, "key {code:?}");
    }
    // Setting the same value again is inert (no re-upload churn).
    let effects = route(
        &mut state,
        PinInput::Key {
            code: KeyCode::Numpad7,
            pressed: true,
            repeat: true,
        },
        t0,
    );
    assert_eq!(effects, [] as [crate::pins::event::PinEffect; 0]);
}

#[test]
fn rotate_swaps_dimensions_and_reuploads() {
    let mut state = pin();
    let t0 = Instant::now();
    let effects = route(
        &mut state,
        PinInput::Key {
            code: KeyCode::KeyR,
            pressed: true,
            repeat: false,
        },
        t0,
    );
    assert_eq!(state.image_size(), (300, 400));
    assert_eq!(state.target_window(), (314, 414));
    assert_eq!(effects[0], PinEffect::Reupload);
    assert_eq!(window_size_effect(&effects), Some((314, 414)));
    // Shift+R rotates back (counter-clockwise).
    route(&mut state, PinInput::Modifiers(ModifiersState::SHIFT), t0);
    route(
        &mut state,
        PinInput::Key {
            code: KeyCode::KeyR,
            pressed: true,
            repeat: false,
        },
        t0,
    );
    assert_eq!(state.image_size(), (400, 300));
    assert_eq!(state.target_window(), (414, 314));
    // Four clockwise turns are the identity.
    route(&mut state, PinInput::Modifiers(ModifiersState::empty()), t0);
    for _ in 0..4 {
        route(
            &mut state,
            PinInput::Key {
                code: KeyCode::KeyR,
                pressed: true,
                repeat: false,
            },
            t0,
        );
    }
    assert_eq!(state.image_size(), (400, 300));
}

#[test]
fn right_release_opens_menu_and_left_press_dispatches_copy() {
    let mut state = pin();
    let t0 = Instant::now();
    moved(&mut state, 100.0, 100.0, t0);
    let effects = route(
        &mut state,
        PinInput::Button {
            button: MouseButton::Right,
            pressed: false,
        },
        t0,
    );
    assert_eq!(effects, vec![PinEffect::Redraw]);
    assert!(state.menu().is_some());
    // First item row under the menu origin: Copy (padding 4 + half of 22).
    // Move onto the first item row (padding 4 + half the 22 px row).
    moved(&mut state, 110.0, 115.0, t0);
    let effects = route(
        &mut state,
        PinInput::Button {
            button: MouseButton::Left,
            pressed: true,
        },
        t0,
    );
    assert!(state.menu().is_none());
    assert!(effects.contains(&PinEffect::Copy));
}

#[test]
fn menu_rotate_item_swaps_dims_and_close_item_closes() {
    let mut state = pin();
    let t0 = Instant::now();
    moved(&mut state, 20.0, 20.0, t0);
    // Open the menu at the cursor; rows: copy(0) save(1) sep(2) rotR(3)...
    route(
        &mut state,
        PinInput::Button {
            button: MouseButton::Right,
            pressed: false,
        },
        t0,
    );
    // Rows: copy(0) save(1) sep(2) rotate-right(3) rotate-left(4)
    // inc(5) dec(6) sep(7) close(8); row center from the LIVE menu rect.
    let row_center = |state: &PinState, rows_down: f64| {
        let rect = state
            .menu()
            .map_or_else(|| panic!("menu closed"), super::menu::PinMenu::rect);
        f64::from(rect.origin.y) + 4.0 + rows_down
    };
    let x = state.menu().map_or_else(
        || panic!("menu closed"),
        |m| f64::from(m.rect().origin.x) + 10.0,
    );
    let y = row_center(&state, 2.0 * 22.0 + 8.0 + 11.0);
    moved(&mut state, x, y, t0);
    let effects = route(
        &mut state,
        PinInput::Button {
            button: MouseButton::Left,
            pressed: true,
        },
        t0,
    );
    assert_eq!(
        state.image_size(),
        (300, 400),
        "rotate-right row: {effects:?}"
    );
    // Reopen and hit the Close row (last item).
    route(
        &mut state,
        PinInput::Button {
            button: MouseButton::Right,
            pressed: false,
        },
        t0,
    );
    let x = state.menu().map_or_else(
        || panic!("menu closed"),
        |m| f64::from(m.rect().origin.x) + 10.0,
    );
    let y = row_center(&state, 6.0 * 22.0 + 2.0 * 8.0 + 11.0);
    moved(&mut state, x, y, t0);
    let effects = route(
        &mut state,
        PinInput::Button {
            button: MouseButton::Left,
            pressed: true,
        },
        t0,
    );
    assert!(effects.contains(&PinEffect::Close));
}

#[test]
fn escape_closes_menu_first_then_the_pin() {
    let mut state = pin();
    let t0 = Instant::now();
    moved(&mut state, 100.0, 100.0, t0);
    route(
        &mut state,
        PinInput::Button {
            button: MouseButton::Right,
            pressed: false,
        },
        t0,
    );
    let effects = route(
        &mut state,
        PinInput::Key {
            code: KeyCode::Escape,
            pressed: true,
            repeat: false,
        },
        t0,
    );
    assert_eq!(effects, vec![PinEffect::Redraw]);
    assert!(state.menu().is_none());
    let effects = route(
        &mut state,
        PinInput::Key {
            code: KeyCode::Escape,
            pressed: true,
            repeat: false,
        },
        t0,
    );
    assert_eq!(effects, vec![PinEffect::Close]);
}

#[test]
fn ctrl_q_and_double_click_close() {
    let mut state = pin();
    let t0 = Instant::now();
    route(&mut state, PinInput::Modifiers(ModifiersState::CONTROL), t0);
    let effects = route(
        &mut state,
        PinInput::Key {
            code: KeyCode::KeyQ,
            pressed: true,
            repeat: false,
        },
        t0,
    );
    assert_eq!(effects, vec![PinEffect::Close]);
    route(&mut state, PinInput::Modifiers(ModifiersState::empty()), t0);
    // Double-click parity: two left presses within 400 ms close the pin.
    let press = PinInput::Button {
        button: MouseButton::Left,
        pressed: true,
    };
    let release = PinInput::Button {
        button: MouseButton::Left,
        pressed: false,
    };
    let mut state = pin();
    assert_eq!(
        route(&mut state, press, t0),
        [] as [crate::pins::event::PinEffect; 0]
    );
    route(&mut state, release, t0 + Duration::from_millis(80));
    // The SECOND press inside the interval is the double-click.
    let effects = route(&mut state, press, t0 + Duration::from_millis(160));
    assert_eq!(effects, vec![PinEffect::Close]);
    // Beyond the interval: a fresh single click.
    let mut state = pin();
    route(&mut state, press, t0);
    let effects = route(&mut state, press, t0 + Duration::from_millis(500));
    assert_eq!(effects, [] as [crate::pins::event::PinEffect; 0]);
}

#[test]
fn drag_starts_once_on_first_motion_while_pressed() {
    let mut state = pin();
    let t0 = Instant::now();
    route(
        &mut state,
        PinInput::Button {
            button: MouseButton::Left,
            pressed: true,
        },
        t0,
    );
    let effects = moved(&mut state, 120.0, 120.0, t0);
    assert_eq!(effects, vec![PinEffect::StartDrag]);
    let effects = moved(&mut state, 130.0, 130.0, t0);
    assert!(!effects.contains(&PinEffect::StartDrag));
}

#[test]
fn pinch_preview_repaints_and_commit_resizes() {
    let mut state = pin();
    let t0 = Instant::now();
    let touch = |id, phase, x, y| PinInput::Touch { id, phase, x, y };
    route(&mut state, touch(1, TouchPhase::Started, 100.0, 150.0), t0);
    route(&mut state, touch(2, TouchPhase::Started, 200.0, 150.0), t0);
    // Spread fingers 100 -> 150 px: preview scale 1.5, repaint only.
    let effects = route(&mut state, touch(2, TouchPhase::Moved, 250.0, 150.0), t0);
    assert_eq!(effects, vec![PinEffect::Redraw]);
    assert!((state.scale() - 1.5).abs() < 1e-9);
    assert_eq!(state.target_window(), (414, 314), "preview must not resize");
    // Lift: commit resizes to the previewed scale.
    let effects = route(&mut state, touch(1, TouchPhase::Ended, 100.0, 150.0), t0);
    assert_eq!(window_size_effect(&effects), Some((614, 464)));
    assert_eq!(state.target_window(), (614, 464));
}

#[test]
fn resized_adopts_the_compositor_extent() {
    let mut state = pin();
    let t0 = Instant::now();
    let effects = route(
        &mut state,
        PinInput::Resized {
            width: 500,
            height: 400,
        },
        t0,
    );
    assert_eq!(effects, vec![PinEffect::Redraw]);
    assert_eq!(state.target_window(), (500, 400));
}

#[test]
fn scale_factor_change_grows_the_frame_only() {
    let mut state = pin();
    let t0 = Instant::now();
    let effects = route(
        &mut state,
        PinInput::ScaleFactorChanged { scale_factor: 2.0 },
        t0,
    );
    // Content is physical (unchanged at zoom 1); the logical margin doubles.
    assert_eq!(state.target_window(), (400 + 28, 300 + 28));
    assert_eq!(window_size_effect(&effects), Some((428, 328)));
    assert!((state.margin_px() - 14.0).abs() < 1e-9);
}

#[test]
fn close_requested_closes() {
    let mut state = pin();
    let effects = route(&mut state, PinInput::CloseRequested, Instant::now());
    assert_eq!(effects, vec![PinEffect::Close]);
}

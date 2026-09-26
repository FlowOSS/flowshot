//! The todo-16 input-state-machine table (plan acceptance: >= 25 cases via
//! the todo-13 test-drive injection seam).
//!
//! Two levels, both headless (no GPU, no window):
//! - the routing table drives [`OverlayCore::inject_event`] - the exact
//!   production path real winit events take;
//! - time-dependent behavior (double-click window, HUD hide-time) drives
//!   [`SelectionState`] directly with synthetic [`Instant`]s, because the
//!   injection seam stamps `Instant::now()` (deterministic tests never
//!   sleep).

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::too_many_arguments
)]

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::scene::{Color as SceneColor, Rect as SceneRect, RectObject};
use winit::event::MouseButton;
use winit::keyboard::{KeyCode, ModifiersState};

use crate::editor::{Tool, ToolKind};
use crate::input::{Action, RouteReport, SyntheticInput};
use crate::router::{InputRouter, WindowSlot};
use crate::selection::{Effect, SelectionConfig, SelectionEnv, SelectionState};
use crate::state::OverlayCore;

/// The minimal registered tool the cascade tests activate (the Esc-cascade
/// tool stage is editor-driven since todo 20 - it needs a real instance).
#[derive(Debug)]
struct CascadeStubTool;

impl Tool for CascadeStubTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Pencil
    }
}

/// Live-machine fixture: HDMI-A-1 1920x1080 @ 1x at (0,0) + DP-3 2560x1440
/// @ 1x at logical (1920,0) -> union bounds 4480x1440 (both scales 1, so
/// local physical == global logical on slot 0).
fn dual_core() -> OverlayCore {
    let make = |connector: &str, rect: LogicalRect, width: i32, height: i32| {
        OutputInfo::new(
            connector,
            connector,
            rect,
            PhysicalSize::from_raw(width, height),
            1.0,
            Transform::Normal,
        )
        .expect("valid fixture output")
    };
    let layout = OutputLayout::new(vec![
        make(
            "HDMI-A-1",
            LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
            1920,
            1080,
        ),
        make(
            "DP-3",
            LogicalRect::from_raw(1920.0, 0.0, 2560.0, 1440.0),
            2560,
            1440,
        ),
    ]);
    OverlayCore::new(InputRouter::new(layout, vec![0, 1]))
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> LogicalRect {
    LogicalRect::from_raw(x, y, w, h)
}

fn selection_of(core: &OverlayCore) -> LogicalRect {
    core.selection().rect().expect("selection must exist")
}

fn move_to(core: &mut OverlayCore, slot: WindowSlot, x: f64, y: f64) -> RouteReport {
    core.inject_event(SyntheticInput::pointer_moved(slot, x, y))
}

fn press_at(core: &mut OverlayCore, slot: WindowSlot, x: f64, y: f64) {
    move_to(core, slot, x, y);
    core.inject_event(SyntheticInput::pointer_button(
        slot,
        MouseButton::Left,
        true,
    ));
}

fn release_at(core: &mut OverlayCore, slot: WindowSlot, x: f64, y: f64) {
    move_to(core, slot, x, y);
    core.inject_event(SyntheticInput::pointer_button(
        slot,
        MouseButton::Left,
        false,
    ));
}

fn drag(core: &mut OverlayCore, slot: WindowSlot, from: (f64, f64), to: (f64, f64)) {
    press_at(core, slot, from.0, from.1);
    move_to(core, slot, to.0, to.1);
    release_at(core, slot, to.0, to.1);
}

fn set_mods(core: &mut OverlayCore, slot: WindowSlot, mods: ModifiersState) {
    core.inject_event(SyntheticInput::modifiers(slot, mods));
}

fn key(core: &mut OverlayCore, slot: WindowSlot, key_code: KeyCode) -> RouteReport {
    core.inject_event(SyntheticInput::key_press(slot, key_code))
}

fn seed(core: &mut OverlayCore, r: LogicalRect) {
    core.selection_mut().set_rect(Some(r));
}

// ---------------------------------------------------------------------------
// Creation: threshold, normalization, minimum, clamp, spanning
// ---------------------------------------------------------------------------

#[test]
fn case01_click_without_drag_creates_no_selection() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    press_at(&mut core, slot, 500.0, 400.0);
    release_at(&mut core, slot, 500.0, 400.0);
    assert_eq!(core.selection().rect(), None);
}

#[test]
fn case02_drag_at_threshold_stays_a_click() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    press_at(&mut core, slot, 100.0, 100.0);
    // Manhattan 3.0 is NOT > 3.0 (the CODE value: strictly greater).
    move_to(&mut core, slot, 102.0, 101.0);
    release_at(&mut core, slot, 102.0, 101.0);
    assert_eq!(core.selection().rect(), None);
}

#[test]
fn case03_tiny_committed_drag_clamps_to_min_10x10() {
    // QA failure scenario: a 1x1-class attempt lands at the 10x10 minimum,
    // anchored at the press corner.
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    drag(&mut core, slot, (100.0, 100.0), (104.0, 104.0));
    assert_eq!(selection_of(&core), rect(100.0, 100.0, 10.0, 10.0));
}

#[test]
fn case04_drag_create_exact_rect() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    drag(&mut core, slot, (100.0, 100.0), (300.0, 250.0));
    assert_eq!(selection_of(&core), rect(100.0, 100.0, 200.0, 150.0));
}

#[test]
fn case05_drag_create_up_left_normalizes() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    drag(&mut core, slot, (300.0, 250.0), (100.0, 100.0));
    assert_eq!(selection_of(&core), rect(100.0, 100.0, 200.0, 150.0));
}

#[test]
fn case06_drag_create_clamps_to_layout() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    drag(&mut core, slot, (100.0, 100.0), (9000.0, 5000.0));
    assert_eq!(selection_of(&core), rect(100.0, 100.0, 4380.0, 1340.0));
}

#[test]
fn case07_spanning_drag_slot0_into_slot1_is_one_rect() {
    // #4894 restored: the implicit grab keeps delivering to slot 0 with
    // local x beyond its 1920px surface; ONE rect crosses the boundary.
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    drag(&mut core, slot, (1800.0, 400.0), (2200.0, 700.0));
    let selection = selection_of(&core);
    assert_eq!(selection, rect(1800.0, 400.0, 400.0, 300.0));
    assert!(selection.x.0 < 1920.0 && selection.right().0 > 1920.0);
}

#[test]
fn case08_spanning_drag_from_slot1_backwards() {
    let mut core = dual_core();
    let slot = WindowSlot::new(1);
    // Slot 1 local (-200, 300) -> global (1720, 300): back over HDMI-A-1.
    drag(&mut core, slot, (100.0, 100.0), (-200.0, 300.0));
    assert_eq!(selection_of(&core), rect(1720.0, 100.0, 300.0, 200.0));
}

#[test]
fn case09_selection_change_redraws_every_window() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    press_at(&mut core, slot, 100.0, 100.0);
    let report = move_to(&mut core, slot, 300.0, 300.0);
    assert!(report.actions.contains(&Action::Redraw(WindowSlot::new(0))));
    assert!(report.actions.contains(&Action::Redraw(WindowSlot::new(1))));
}

// ---------------------------------------------------------------------------
// Moving
// ---------------------------------------------------------------------------

#[test]
fn case10_inside_drag_moves_by_the_grab_offset() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(100.0, 100.0, 200.0, 150.0));
    drag(&mut core, slot, (150.0, 120.0), (250.0, 220.0));
    assert_eq!(selection_of(&core), rect(200.0, 200.0, 200.0, 150.0));
}

#[test]
fn case11_move_slides_at_the_bounds_preserving_size() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(1700.0, 100.0, 200.0, 150.0));
    drag(&mut core, slot, (1750.0, 150.0), (4900.0, 150.0));
    // Cursor clamps at 4480; the rect slides to the bound, size intact.
    assert_eq!(selection_of(&core), rect(4280.0, 100.0, 200.0, 150.0));
}

#[test]
fn case12_click_outside_clears_click_inside_keeps() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(100.0, 100.0, 100.0, 100.0));
    // Click far outside: cleared (Flameshot hides on an outside release).
    press_at(&mut core, slot, 800.0, 800.0);
    release_at(&mut core, slot, 800.0, 800.0);
    assert_eq!(core.selection().rect(), None);
    // Re-seed; a click inside keeps it.
    seed(&mut core, rect(100.0, 100.0, 100.0, 100.0));
    press_at(&mut core, slot, 150.0, 150.0);
    release_at(&mut core, slot, 150.0, 150.0);
    assert_eq!(selection_of(&core), rect(100.0, 100.0, 100.0, 100.0));
}

#[test]
fn case13_press_outside_replaces_the_selection() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(100.0, 100.0, 50.0, 50.0));
    drag(&mut core, slot, (500.0, 500.0), (600.0, 600.0));
    assert_eq!(selection_of(&core), rect(500.0, 500.0, 100.0, 100.0));
}

// ---------------------------------------------------------------------------
// Handles: all eight, hit priority
// ---------------------------------------------------------------------------

#[test]
fn case14_every_handle_resizes_its_edges() {
    // (handle anchor, drag target, expected rect) over the base selection
    // (500, 400, 200, 150): corners (500,400) (700,400) (500,550) (700,550),
    // edge midpoints (600,400) (600,550) (500,475) (700,475).
    let cases = [
        (
            (500.0, 400.0),
            (450.0, 350.0),
            rect(450.0, 350.0, 250.0, 200.0),
        ),
        (
            (700.0, 400.0),
            (750.0, 350.0),
            rect(500.0, 350.0, 250.0, 200.0),
        ),
        (
            (500.0, 550.0),
            (450.0, 600.0),
            rect(450.0, 400.0, 250.0, 200.0),
        ),
        (
            (700.0, 550.0),
            (750.0, 600.0),
            rect(500.0, 400.0, 250.0, 200.0),
        ),
        (
            (600.0, 400.0),
            (600.0, 350.0),
            rect(500.0, 350.0, 200.0, 200.0),
        ),
        (
            (600.0, 550.0),
            (600.0, 600.0),
            rect(500.0, 400.0, 200.0, 200.0),
        ),
        (
            (500.0, 475.0),
            (450.0, 475.0),
            rect(450.0, 400.0, 250.0, 150.0),
        ),
        (
            (700.0, 475.0),
            (750.0, 475.0),
            rect(500.0, 400.0, 250.0, 150.0),
        ),
    ];
    for (anchor, target, expected) in cases {
        let mut core = dual_core();
        let slot = WindowSlot::new(0);
        seed(&mut core, rect(500.0, 400.0, 200.0, 150.0));
        drag(&mut core, slot, anchor, target);
        assert_eq!(selection_of(&core), expected, "anchor {anchor:?}");
    }
}

#[test]
fn case15_corner_priority_beats_center_on_a_tiny_selection() {
    // On a 10x10 rect the corner areas cover the interior: a press at the
    // center must start a TL-corner RESIZE, not a move.
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(100.0, 100.0, 10.0, 10.0));
    drag(&mut core, slot, (105.0, 105.0), (95.0, 95.0));
    // TL resize to (95,95) with BR fixed at (110,110). A MOVE would have
    // produced (90, 90, 10, 10).
    assert_eq!(selection_of(&core), rect(95.0, 95.0, 15.0, 15.0));
}

#[test]
fn case16_resize_shrink_stops_at_the_minimum() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 100.0, 100.0));
    // BR handle dragged onto the fixed corner: both dimensions clamp to
    // the 10x10 minimum anchored at the fixed TL corner.
    drag(&mut core, slot, (600.0, 500.0), (505.0, 405.0));
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 10.0, 10.0));
}

#[test]
fn case17_crossing_through_flips_the_active_handle() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 100.0, 100.0));
    // BR handle dragged far past the LEFT edge: the handle flips to BL and
    // the cursor side keeps tracking (final left edge = cursor x).
    press_at(&mut core, slot, 600.0, 500.0);
    move_to(&mut core, slot, 420.0, 550.0);
    move_to(&mut core, slot, 300.0, 550.0);
    release_at(&mut core, slot, 300.0, 550.0);
    assert_eq!(selection_of(&core), rect(300.0, 400.0, 300.0, 150.0));
}

// ---------------------------------------------------------------------------
// Modifiers: Shift mirror, Ctrl aspect
// ---------------------------------------------------------------------------

#[test]
fn case18_shift_mirrors_the_resize_around_the_center() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 200.0, 150.0));
    press_at(&mut core, slot, 700.0, 550.0);
    set_mods(&mut core, slot, ModifiersState::SHIFT);
    // dBR = (20, 20) -> topLeft -= (20, 20): symmetric around (600, 475).
    move_to(&mut core, slot, 720.0, 570.0);
    release_at(&mut core, slot, 720.0, 570.0);
    assert_eq!(selection_of(&core), rect(480.0, 380.0, 240.0, 190.0));
}

#[test]
fn case19_ctrl_constrains_the_aspect_on_corners() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 200.0, 100.0)); // aspect 2.0
    press_at(&mut core, slot, 700.0, 500.0);
    set_mods(&mut core, slot, ModifiersState::CONTROL);
    // (px-l)/(py-t) = 250/110 > 2 -> width dominates: h = 250/2 = 125.
    move_to(&mut core, slot, 750.0, 510.0);
    release_at(&mut core, slot, 750.0, 510.0);
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 250.0, 125.0));
}

#[test]
fn case20_ctrl_edge_drag_moves_the_companion_edge() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 200.0, 100.0)); // aspect 2.0
    press_at(&mut core, slot, 500.0, 450.0); // Left handle
    set_mods(&mut core, slot, ModifiersState::CONTROL);
    // bottom = top + (right - px)/aspect = 400 + 250/2 = 525.
    move_to(&mut core, slot, 450.0, 450.0);
    release_at(&mut core, slot, 450.0, 450.0);
    assert_eq!(selection_of(&core), rect(450.0, 400.0, 250.0, 125.0));
}

#[test]
fn case21_shift_ctrl_combine_mirror_and_aspect() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 200.0, 100.0)); // aspect 2.0
    press_at(&mut core, slot, 700.0, 500.0);
    set_mods(
        &mut core,
        slot,
        ModifiersState::SHIFT | ModifiersState::CONTROL,
    );
    // Aspect gives (500,400,250,125); the mirror then extends TL by the
    // same delta: (450, 375, 300, 150) - still aspect 2.
    move_to(&mut core, slot, 750.0, 510.0);
    release_at(&mut core, slot, 750.0, 510.0);
    assert_eq!(selection_of(&core), rect(450.0, 375.0, 300.0, 150.0));
}

#[test]
fn case22_modifier_event_is_tracked() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    assert!(!core.modifiers().shift_key());
    set_mods(&mut core, slot, ModifiersState::SHIFT);
    assert!(core.modifiers().shift_key());
    set_mods(&mut core, slot, ModifiersState::empty());
    assert!(!core.modifiers().shift_key());
}

// ---------------------------------------------------------------------------
// Keyboard: nudge / resize / symmetric (CODE values: 1px)
// ---------------------------------------------------------------------------

#[test]
fn case23_arrows_move_one_px() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 200.0, 150.0));
    key(&mut core, slot, KeyCode::ArrowLeft);
    assert_eq!(selection_of(&core), rect(499.0, 400.0, 200.0, 150.0));
    key(&mut core, slot, KeyCode::ArrowUp);
    assert_eq!(selection_of(&core), rect(499.0, 399.0, 200.0, 150.0));
    key(&mut core, slot, KeyCode::ArrowRight);
    assert_eq!(selection_of(&core), rect(500.0, 399.0, 200.0, 150.0));
    key(&mut core, slot, KeyCode::ArrowDown);
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 200.0, 150.0));
}

#[test]
fn case24_shift_arrows_resize_one_edge_one_px() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 200.0, 150.0));
    set_mods(&mut core, slot, ModifiersState::SHIFT);
    // CODE semantics: Left/Right move the RIGHT edge, Up/Down the BOTTOM.
    key(&mut core, slot, KeyCode::ArrowLeft);
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 199.0, 150.0));
    key(&mut core, slot, KeyCode::ArrowRight);
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 200.0, 150.0));
    key(&mut core, slot, KeyCode::ArrowUp);
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 200.0, 149.0));
    key(&mut core, slot, KeyCode::ArrowDown);
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 200.0, 150.0));
}

#[test]
fn case25_ctrl_shift_arrows_resize_symmetrically_one_px() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 200.0, 150.0));
    set_mods(
        &mut core,
        slot,
        ModifiersState::SHIFT | ModifiersState::CONTROL,
    );
    key(&mut core, slot, KeyCode::ArrowLeft);
    assert_eq!(selection_of(&core), rect(501.0, 400.0, 198.0, 150.0));
    key(&mut core, slot, KeyCode::ArrowRight);
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 200.0, 150.0));
    key(&mut core, slot, KeyCode::ArrowUp);
    assert_eq!(selection_of(&core), rect(500.0, 399.0, 200.0, 152.0));
    key(&mut core, slot, KeyCode::ArrowDown);
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 200.0, 150.0));
}

#[test]
fn case26_arrow_move_clamps_at_the_bounds() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(0.0, 0.0, 50.0, 50.0));
    key(&mut core, slot, KeyCode::ArrowLeft);
    key(&mut core, slot, KeyCode::ArrowUp);
    assert_eq!(selection_of(&core), rect(0.0, 0.0, 50.0, 50.0));
}

#[test]
fn case27_keyboard_resize_stops_at_the_minimum() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(500.0, 400.0, 10.0, 10.0));
    set_mods(&mut core, slot, ModifiersState::SHIFT);
    key(&mut core, slot, KeyCode::ArrowLeft); // right edge -1: refused
    key(&mut core, slot, KeyCode::ArrowUp); // bottom edge -1: refused
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 10.0, 10.0));
    set_mods(
        &mut core,
        slot,
        ModifiersState::SHIFT | ModifiersState::CONTROL,
    );
    key(&mut core, slot, KeyCode::ArrowLeft); // symmetric shrink: refused
    assert_eq!(selection_of(&core), rect(500.0, 400.0, 10.0, 10.0));
}

#[test]
fn case28_arrows_without_selection_are_inert() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    let report = key(&mut core, slot, KeyCode::ArrowLeft);
    assert_eq!(core.selection().rect(), None);
    assert!(report.actions.is_empty());
}

// ---------------------------------------------------------------------------
// Shortcuts: Ctrl+A / Enter / Ctrl+C / Ctrl+Q / right-click
// ---------------------------------------------------------------------------

#[test]
fn case29_ctrl_a_selects_the_full_layout() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(100.0, 100.0, 50.0, 50.0));
    set_mods(&mut core, slot, ModifiersState::CONTROL);
    key(&mut core, slot, KeyCode::KeyA);
    assert_eq!(selection_of(&core), rect(0.0, 0.0, 4480.0, 1440.0));
}

#[test]
fn case30_enter_accepts_only_with_a_selection() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    let report = key(&mut core, slot, KeyCode::Enter);
    assert!(!report.actions.contains(&Action::Accept));
    seed(&mut core, rect(100.0, 100.0, 50.0, 50.0));
    let report = key(&mut core, slot, KeyCode::Enter);
    assert!(report.actions.contains(&Action::Accept));
    let report = key(&mut core, slot, KeyCode::NumpadEnter);
    assert!(report.actions.contains(&Action::Accept));
}

#[test]
fn case31_ctrl_c_copies_only_with_a_selection() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    set_mods(&mut core, slot, ModifiersState::CONTROL);
    let report = key(&mut core, slot, KeyCode::KeyC);
    assert!(!report.actions.contains(&Action::Copy));
    seed(&mut core, rect(100.0, 100.0, 50.0, 50.0));
    let report = key(&mut core, slot, KeyCode::KeyC);
    assert!(report.actions.contains(&Action::Copy));
}

#[test]
fn case32_ctrl_q_exits_immediately() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    seed(&mut core, rect(100.0, 100.0, 50.0, 50.0));
    // Todo 20: the tool-checked cascade stage is editor-driven - a REAL
    // registered+activated tool occupies it (raw flag pokes are overwritten
    // by the post-event cascade sync).
    core.editor_mut()
        .registry_mut()
        .register(ToolKind::Pencil, || Box::new(CascadeStubTool));
    core.editor_mut().activate_tool(ToolKind::Pencil);
    core.sync_cascade();
    set_mods(&mut core, slot, ModifiersState::CONTROL);
    let report = key(&mut core, slot, KeyCode::KeyQ);
    // Immediate: the cascade is NOT walked.
    assert!(report.actions.contains(&Action::Exit));
    assert!(core.exit_requested());
    assert!(core.selection().cascade().tool_checked());
}

#[test]
fn case33_right_click_is_the_color_wheel_seam() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    move_to(&mut core, slot, 500.0, 400.0);
    let report = core.inject_event(SyntheticInput::pointer_button(
        slot,
        MouseButton::Right,
        true,
    ));
    assert!(report.actions.contains(&Action::ColorWheel));
    // The right press does not disturb the selection or start a drag.
    assert_eq!(core.selection().rect(), None);
}

// ---------------------------------------------------------------------------
// Esc cascade (QA failure scenario: the log order matches the spec exactly)
// ---------------------------------------------------------------------------

#[test]
fn case34_esc_cascade_walks_all_six_stages_in_order() {
    use tracing_subscriber::fmt::MakeWriter;

    struct TestWriter(Arc<Mutex<Vec<u8>>>);
    impl MakeWriter<'_> for TestWriter {
        type Writer = TestSink;
        fn make_writer(&self) -> Self::Writer {
            TestSink(self.0.clone())
        }
    }
    struct TestSink(Arc<Mutex<Vec<u8>>>);
    impl Write for TestSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|e| io::Error::other(e.to_string()))?
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    // Todo 20/26: EVERY stage is driven by real state now - 1/2/4 by the
    // editor, 3/5 by the chrome (raw cascade pokes are futile).
    core.editor_mut()
        .registry_mut()
        .register(ToolKind::Pencil, || Box::new(CascadeStubTool));
    core.editor_mut().activate_tool(ToolKind::Pencil);
    core.editor_mut().commit_object(Box::new(RectObject::new(
        SceneRect::new(100.0, 100.0, 50.0, 50.0),
        SceneColor::new(255, 0, 0, 255),
        2.0,
        false,
    )));
    core.editor_mut()
        .select_object_at(LogicalPoint::from_raw(120.0, 120.0));
    core.editor_mut().set_edit_widget_present(true);
    assert!(core.chrome_space(), "panel toggle (stage 3 producer)");
    core.chrome_mut()
        .show_color_wheel(LogicalPoint::from_raw(200.0, 200.0));
    core.sync_cascade();
    assert!(core.selection().cascade().panel_visible());
    assert!(core.selection().cascade().picker_visible());

    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(TestWriter(buffer.clone()))
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        // Stages 1-5: no exit, one flag cleared per press, in spec order.
        let report = key(&mut core, slot, KeyCode::Escape);
        assert!(!report.actions.contains(&Action::Exit));
        let cascade = core.selection().cascade();
        assert!(!cascade.tool_checked() && cascade.object_selected());
        let report = key(&mut core, slot, KeyCode::Escape);
        assert!(!report.actions.contains(&Action::Exit));
        let cascade = core.selection().cascade();
        assert!(!cascade.object_selected() && cascade.panel_visible());
        let report = key(&mut core, slot, KeyCode::Escape);
        assert!(!report.actions.contains(&Action::Exit));
        assert!(!core.selection().cascade().panel_visible());
        let report = key(&mut core, slot, KeyCode::Escape);
        assert!(!report.actions.contains(&Action::Exit));
        assert!(!core.selection().cascade().tool_widget_present());
        let report = key(&mut core, slot, KeyCode::Escape);
        assert!(!report.actions.contains(&Action::Exit));
        assert!(!core.selection().cascade().picker_visible());
        // Stage 6: close.
        let report = key(&mut core, slot, KeyCode::Escape);
        assert!(report.actions.contains(&Action::Exit));
        assert!(core.exit_requested());
    });

    let log = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    let order = [
        "deselect-tool",
        "deselect-object",
        "hide-panel",
        "delete-tool-widget",
        "hide-picker",
        "close",
    ];
    let mut last = 0;
    for token in order {
        let at = log[last..]
            .find(token)
            .unwrap_or_else(|| panic!("cascade log missing stage {token}: {log}"));
        last += at + token.len();
    }
}

#[test]
fn case35_esc_on_empty_cascade_closes() {
    let mut core = dual_core();
    let slot = WindowSlot::new(1);
    let report = key(&mut core, slot, KeyCode::Escape);
    assert_eq!(report.actions, vec![Action::Exit]);
    assert!(core.exit_requested());
}

// ---------------------------------------------------------------------------
// HUD (time-based: driven at the SelectionState level with synthetic Instants)
// ---------------------------------------------------------------------------

fn state_with(config: SelectionConfig) -> SelectionState {
    SelectionState::new(config, &flowshot_core::tokens::DesignTokens::default())
}

fn env_at(bounds: LogicalRect, now: Instant) -> SelectionEnv {
    SelectionEnv {
        bounds: Some(bounds),
        modifiers: ModifiersState::empty(),
        now,
    }
}

#[test]
fn case36_hud_shows_on_change_with_wxh_x_y_text() {
    let mut core = dual_core();
    let slot = WindowSlot::new(0);
    drag(&mut core, slot, (100.0, 100.0), (400.0, 300.0));
    let view = core
        .selection()
        .hud_view(Instant::now())
        .expect("HUD visible right after the change");
    assert_eq!(view.text, "300x200+100+100");
    // Default position 4 = bottom-right: the box's bottom-right corner is
    // the selection's bottom-right corner.
    assert_eq!(view.box_rect.right().0, 400.0);
    assert_eq!(view.box_rect.bottom().0, 300.0);
}

#[test]
fn case37_hud_hides_after_the_configured_time() {
    let mut state = state_with(SelectionConfig {
        hud_hide_time: 3000,
        ..SelectionConfig::default()
    });
    let bounds = rect(0.0, 0.0, 1920.0, 1080.0);
    let t0 = Instant::now();
    state.set_rect(Some(rect(100.0, 100.0, 200.0, 150.0)));
    let env = env_at(bounds, t0);
    let update = state.key_press(&env, KeyCode::ArrowLeft);
    assert!(update.changed);
    assert!(state.hud_view(t0 + Duration::from_millis(2999)).is_some());
    assert!(state.hud_view(t0 + Duration::from_millis(3000)).is_none());
    assert_eq!(state.hud_wake(), Some(t0 + Duration::from_millis(3000)));
    // tick flips once at the deadline (the shell's redraw signal).
    assert!(!state.tick(t0 + Duration::from_millis(2999)));
    assert!(state.tick(t0 + Duration::from_millis(3000)));
    assert!(!state.tick(t0 + Duration::from_millis(3001)));
    assert_eq!(state.hud_wake(), None);
}

#[test]
fn case38_hud_position_zero_never_shows() {
    let mut state = state_with(SelectionConfig {
        hud_position: 0,
        ..SelectionConfig::default()
    });
    let t0 = Instant::now();
    state.set_rect(Some(rect(100.0, 100.0, 200.0, 150.0)));
    let env = env_at(rect(0.0, 0.0, 1920.0, 1080.0), t0);
    state.key_press(&env, KeyCode::ArrowLeft);
    assert!(state.hud_view(t0).is_none());
}

#[test]
fn case39_zero_hide_time_keeps_the_hud() {
    let mut state = state_with(SelectionConfig {
        hud_hide_time: 0,
        ..SelectionConfig::default()
    });
    let t0 = Instant::now();
    state.set_rect(Some(rect(100.0, 100.0, 200.0, 150.0)));
    let env = env_at(rect(0.0, 0.0, 1920.0, 1080.0), t0);
    state.key_press(&env, KeyCode::ArrowLeft);
    assert!(state.hud_view(t0 + Duration::from_secs(3600)).is_some());
    assert_eq!(state.hud_wake(), None);
}

// ---------------------------------------------------------------------------
// Double-click copy (time-based, SelectionState level)
// ---------------------------------------------------------------------------

fn double_click_state(enabled: bool) -> SelectionState {
    let mut state = state_with(SelectionConfig {
        double_click_copies: enabled,
        ..SelectionConfig::default()
    });
    state.set_rect(Some(rect(100.0, 100.0, 200.0, 150.0)));
    state
}

#[test]
fn case40_double_click_inside_copies_when_configured() {
    let mut state = double_click_state(true);
    let bounds = rect(0.0, 0.0, 1920.0, 1080.0);
    let at = flowshot_core::geometry::LogicalPoint::from_raw(150.0, 150.0);
    let t0 = Instant::now();
    let env = env_at(bounds, t0);
    let first = state.pointer_press(&env, MouseButton::Left, at);
    assert!(!first.effects.contains(&Effect::Copy));
    state.pointer_release(&env, MouseButton::Left, at);
    // Second press inside the interval and distance: copy fires.
    let env = env_at(bounds, t0 + Duration::from_millis(100));
    let second = state.pointer_press(&env, MouseButton::Left, at);
    assert!(second.effects.contains(&Effect::Copy));
    state.pointer_release(&env, MouseButton::Left, at);
    // A third press starts a fresh pair (Qt does not re-fire).
    let env = env_at(bounds, t0 + Duration::from_millis(150));
    let third = state.pointer_press(&env, MouseButton::Left, at);
    assert!(!third.effects.contains(&Effect::Copy));
}

#[test]
fn case41_double_click_beyond_the_interval_does_not_copy() {
    let mut state = double_click_state(true);
    let bounds = rect(0.0, 0.0, 1920.0, 1080.0);
    let at = flowshot_core::geometry::LogicalPoint::from_raw(150.0, 150.0);
    let t0 = Instant::now();
    let env = env_at(bounds, t0);
    state.pointer_press(&env, MouseButton::Left, at);
    state.pointer_release(&env, MouseButton::Left, at);
    let env = env_at(bounds, t0 + Duration::from_millis(401));
    let second = state.pointer_press(&env, MouseButton::Left, at);
    assert!(!second.effects.contains(&Effect::Copy));
}

#[test]
fn case42_double_click_disabled_by_config_never_copies() {
    let mut state = double_click_state(false);
    let bounds = rect(0.0, 0.0, 1920.0, 1080.0);
    let at = flowshot_core::geometry::LogicalPoint::from_raw(150.0, 150.0);
    let t0 = Instant::now();
    let env = env_at(bounds, t0);
    state.pointer_press(&env, MouseButton::Left, at);
    state.pointer_release(&env, MouseButton::Left, at);
    let env = env_at(bounds, t0 + Duration::from_millis(100));
    let second = state.pointer_press(&env, MouseButton::Left, at);
    assert!(!second.effects.contains(&Effect::Copy));
}

#[test]
fn case43_double_click_outside_the_selection_does_not_copy() {
    let mut state = double_click_state(true);
    let bounds = rect(0.0, 0.0, 1920.0, 1080.0);
    let at = flowshot_core::geometry::LogicalPoint::from_raw(800.0, 800.0);
    let t0 = Instant::now();
    let env = env_at(bounds, t0);
    state.pointer_press(&env, MouseButton::Left, at);
    state.pointer_release(&env, MouseButton::Left, at);
    let env = env_at(bounds, t0 + Duration::from_millis(100));
    let second = state.pointer_press(&env, MouseButton::Left, at);
    assert!(!second.effects.contains(&Effect::Copy));
}

// ---------------------------------------------------------------------------
// Engine-level effects and seams
// ---------------------------------------------------------------------------

#[test]
fn case44_effects_map_onto_shell_actions() {
    assert_eq!(Action::from(Effect::Accept), Action::Accept);
    assert_eq!(Action::from(Effect::Copy), Action::Copy);
    assert_eq!(Action::from(Effect::Exit), Action::Exit);
    assert_eq!(Action::from(Effect::ColorWheel), Action::ColorWheel);
}

#[test]
fn case45_set_rect_seeds_without_showing_the_hud() {
    // The todo-18 preselect seam mirrors Flameshot's initialSelection: the
    // geometry indicator only appears on USER-driven changes.
    let mut state = state_with(SelectionConfig::default());
    state.set_rect(Some(rect(100.0, 100.0, 200.0, 150.0)));
    assert_eq!(state.rect(), Some(rect(100.0, 100.0, 200.0, 150.0)));
    assert!(state.hud_view(Instant::now()).is_none());
}

#[test]
fn case46_unpaired_release_is_ignored() {
    let mut state = state_with(SelectionConfig::default());
    state.set_rect(Some(rect(100.0, 100.0, 200.0, 150.0)));
    let env = env_at(rect(0.0, 0.0, 1920.0, 1080.0), Instant::now());
    let outside = flowshot_core::geometry::LogicalPoint::from_raw(900.0, 900.0);
    let update = state.pointer_release(&env, MouseButton::Left, outside);
    assert!(!update.changed);
    assert_eq!(state.rect(), Some(rect(100.0, 100.0, 200.0, 150.0)));
}

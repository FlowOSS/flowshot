//! The todo-20 editor state-machine table (plan acceptance: routing
//! priority, size dispatch, digit accumulation clip, wheel threshold,
//! stroke commit, undo integration, Esc cascade with the real tool stage).
//!
//! Two levels, both headless:
//! - [`EditorState`] driven directly with synthetic [`Instant`]s (the
//!   digit-accumulator clock never sleeps),
//! - [`OverlayCore::inject_event`] - the exact production funnel real winit
//!   events take (the todo-13 test-drive seam).
//!
//! The stub tools log through a thread-local (the registry's `fn`-pointer
//! factories cannot capture), so lifecycle order is assertable across the
//! fresh-instance-per-stroke replacement.

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use std::cell::RefCell;
use std::time::{Duration, Instant};

use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::scene::{
    ArrowObject, Color as SceneColor, CounterObject, PaintSink, Point as ScenePoint,
    Rect as SceneRect, RectObject, TextObject, ToolObject,
};
use winit::event::MouseButton;
use winit::keyboard::{KeyCode, ModifiersState};

use crate::editor::*;
use crate::input::{Action, RouteReport, SyntheticInput};
use crate::render::{Command, DisplayList, Shape};
use crate::router::{InputRouter, WindowSlot};
use crate::state::OverlayCore;

// ---------------------------------------------------------------------------
// Stub tools (the concrete tools are todos 21-27; these prove the framework)
// ---------------------------------------------------------------------------

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(event: &str) {
    LOG.with(|log| log.borrow_mut().push(event.to_owned()));
}

fn take_log() -> Vec<String> {
    LOG.with(|log| log.borrow_mut().drain(..).collect())
}

fn pt(at: LogicalPoint) -> ScenePoint {
    ScenePoint::new(at.x.0 as f32, at.y.0 as f32)
}

const RED: SceneColor = SceneColor::new(255, 0, 0, 255);

/// Pencil stand-in: a one-line stroke committed as an [`ArrowObject`] (the
/// scene's line-capable object until todo 21 lands pencil/line types).
#[derive(Debug)]
struct StubLineTool {
    from: Option<ScenePoint>,
    to: Option<ScenePoint>,
    color: SceneColor,
    size: u32,
}

impl StubLineTool {
    fn new() -> Self {
        log("new");
        Self {
            from: None,
            to: None,
            color: RED,
            size: 3,
        }
    }
}

impl Tool for StubLineTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Pencil
    }
    fn pressed(
        &mut self,
        _ctx: &EditorContext<'_>,
        _button: MouseButton,
        _at: LogicalPoint,
    ) -> bool {
        log("pressed");
        false
    }
    fn draw_start(&mut self, ctx: &EditorContext<'_>, at: LogicalPoint) {
        log(&format!(
            "draw_start frame={} selection={} color={},{},{} size={} count={} ctrl={} shift={}",
            u8::from(ctx.frame.is_some()),
            u8::from(ctx.selection.is_some()),
            ctx.color.r,
            ctx.color.g,
            ctx.color.b,
            ctx.tool_size,
            ctx.circle_count,
            u8::from(ctx.modifiers.control_key()),
            u8::from(ctx.modifiers.shift_key()),
        ));
        self.from = Some(pt(at));
        self.to = Some(pt(at));
    }
    fn draw_move(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        log("draw_move");
        self.to = Some(pt(at));
    }
    fn draw_end(
        &mut self,
        _ctx: &EditorContext<'_>,
        at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        log("draw_end");
        let from = self.from.take()?;
        let to = pt(at);
        self.to = None;
        if from == to {
            return None;
        }
        Some(Box::new(ArrowObject::new(
            from,
            to,
            self.color,
            self.size as f32,
        )))
    }
    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        if let (Some(from), Some(to)) = (self.from, self.to) {
            log("paint-stroke");
            sink.draw_line(from, to, ctx.color, self.size as f32);
        } else {
            log("paint-preview");
            let center = pt(ctx.mouse);
            let radius = self.size as f32;
            sink.fill_ellipse(
                SceneRect::new(
                    center.x - radius,
                    center.y - radius,
                    radius * 2.0,
                    radius * 2.0,
                ),
                ctx.color,
            );
        }
    }
    fn on_color_changed(&mut self, color: SceneColor) {
        log("color-changed");
        self.color = color;
    }
    fn on_size_changed(&mut self, size: u32) {
        log(&format!("size-changed {size}"));
        self.size = size;
    }
}

/// Counter stand-in: consumes the press (single-click placement, the F27
/// `pressed` slot) - no draw session opens.
#[derive(Debug)]
struct StubClickTool;
impl Tool for StubClickTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Counter
    }
    fn pressed(
        &mut self,
        _ctx: &EditorContext<'_>,
        _button: MouseButton,
        _at: LogicalPoint,
    ) -> bool {
        log("click-consumed");
        true
    }
    fn draw_start(&mut self, _ctx: &EditorContext<'_>, _at: LogicalPoint) {
        log("click-draw_start-UNEXPECTED");
    }
}

/// Text stand-in: owns an edit widget rect; commits a [`TextObject`].
#[derive(Debug)]
struct StubEditTool;
impl Tool for StubEditTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Text
    }
    fn edit_rect(&self) -> Option<LogicalRect> {
        Some(LogicalRect::from_raw(100.0, 100.0, 200.0, 100.0))
    }
    fn pressed(
        &mut self,
        _ctx: &EditorContext<'_>,
        _button: MouseButton,
        _at: LogicalPoint,
    ) -> bool {
        log("edit-press");
        false
    }
    fn commit_edit(&mut self, _ctx: &EditorContext<'_>) -> Option<Box<dyn ToolObject>> {
        log("commit-edit");
        Some(Box::new(TextObject::new(
            ScenePoint::new(100.0, 100.0),
            "hi".to_owned(),
            16.0,
            RED,
        )))
    }
    fn cancel_edit(&mut self) {
        log("cancel-edit");
    }
}

/// Marker stand-in: no mouse preview.
#[derive(Debug)]
struct StubQuietTool;
impl Tool for StubQuietTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Marker
    }
    fn show_mouse_preview(&self) -> bool {
        false
    }
    fn paint(&self, _ctx: &EditorContext<'_>, _sink: &mut dyn PaintSink) {
        log("quiet-paint-UNEXPECTED");
    }
}

/// Circle stand-in: consumes the wheel (the counter-bubble increment seam).
#[derive(Debug)]
struct StubWheelTool;
impl Tool for StubWheelTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Circle
    }
    fn wheel(&mut self, _ctx: &EditorContext<'_>, step: i32) -> bool {
        log(&format!("wheel-consumed {step}"));
        true
    }
}

fn stub_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(ToolKind::Pencil, || Box::new(StubLineTool::new()));
    registry.register(ToolKind::Counter, || Box::new(StubClickTool));
    registry.register(ToolKind::Text, || Box::new(StubEditTool));
    registry.register(ToolKind::Marker, || Box::new(StubQuietTool));
    registry.register(ToolKind::Circle, || Box::new(StubWheelTool));
    registry
}

fn editor() -> EditorState {
    take_log();
    EditorState::new(EditorTools::default(), stub_registry())
}

fn env_at(now: Instant) -> EditorEnv {
    EditorEnv {
        selection: None,
        modifiers: ModifiersState::empty(),
        now,
        picker_visible: false,
        mouse: None,
    }
}

fn at(x: f64, y: f64) -> LogicalPoint {
    LogicalPoint::from_raw(x, y)
}

fn press(
    ed: &mut EditorState,
    env: &EditorEnv,
    button: MouseButton,
    x: f64,
    y: f64,
) -> EditorUpdate {
    ed.pointer_press(env, button, at(x, y))
}

fn release(
    ed: &mut EditorState,
    env: &EditorEnv,
    button: MouseButton,
    x: f64,
    y: f64,
) -> EditorUpdate {
    ed.pointer_release(env, button, at(x, y))
}

fn stroke(ed: &mut EditorState, env: &EditorEnv, from: (f64, f64), to: (f64, f64)) {
    press(ed, env, MouseButton::Left, from.0, from.1);
    ed.pointer_move(env, at(to.0, to.1));
    release(ed, env, MouseButton::Left, to.0, to.1);
}

fn rect_object(x: f32, y: f32, w: f32, h: f32) -> Box<dyn ToolObject> {
    Box::new(RectObject::new(SceneRect::new(x, y, w, h), RED, 2.0, false))
}

fn counter_object(x: f32, y: f32) -> Box<dyn ToolObject> {
    Box::new(CounterObject::new(ScenePoint::new(x, y), 12.0, RED, 0))
}

fn output() -> OutputInfo {
    OutputInfo::new(
        "DP-T",
        "DP-T",
        LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
        PhysicalSize::from_raw(1920, 1080),
        1.0,
        Transform::Normal,
    )
    .expect("valid fixture output")
}

// ---------------------------------------------------------------------------
// A. Draw-session lifecycle and scene commit
// ---------------------------------------------------------------------------

#[test]
fn stroke_commits_one_object_as_one_undo_unit() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    take_log();
    stroke(&mut ed, &env, (100.0, 100.0), (300.0, 250.0));
    assert_eq!(ed.scene().object_count(), 1);
    assert_eq!(ed.undo_stack().undo_depth(), 1, "one stroke = one unit");
    assert_eq!(
        ed.scene().get_object(0).map(ToolObject::type_id),
        Some("arrow")
    );
    let log = take_log();
    assert!(log.contains(&"new".to_owned()), "fresh instance per stroke");
    assert!(log.contains(&"pressed".to_owned()));
    assert!(log.contains(&"draw_move".to_owned()));
    assert!(log.contains(&"draw_end".to_owned()));
    assert!(!ed.is_drawing());
}

#[test]
fn zero_length_stroke_commits_nothing() {
    // The todo-21 acceptance rule, enforced by the tool's own validity:
    // click without move -> draw_end yields None -> scene stays empty.
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    press(&mut ed, &env, MouseButton::Left, 100.0, 100.0);
    release(&mut ed, &env, MouseButton::Left, 100.0, 100.0);
    assert_eq!(ed.scene().object_count(), 0);
    assert_eq!(ed.undo_stack().undo_depth(), 0);
}

#[test]
fn press_consuming_tool_opens_no_draw_session() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Counter);
    take_log();
    let update = press(&mut ed, &env, MouseButton::Left, 100.0, 100.0);
    assert!(update.consumed);
    assert!(!ed.is_drawing(), "pressed() consumed - no session");
    release(&mut ed, &env, MouseButton::Left, 100.0, 100.0);
    let log = take_log();
    assert!(log.contains(&"click-consumed".to_owned()));
    assert!(!log.iter().any(|entry| entry.contains("UNEXPECTED")));
    assert_eq!(ed.scene().object_count(), 0);
}

#[test]
fn each_stroke_gets_a_fresh_instance() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    take_log();
    stroke(&mut ed, &env, (0.0, 0.0), (50.0, 50.0));
    stroke(&mut ed, &env, (100.0, 100.0), (150.0, 150.0));
    assert_eq!(ed.scene().object_count(), 2);
    assert_eq!(ed.undo_stack().undo_depth(), 2);
    assert_eq!(take_log().iter().filter(|e| *e == "new").count(), 2);
}

#[test]
fn unregistered_kind_never_activates() {
    let mut ed = editor();
    ed.activate_tool(ToolKind::Invert); // not in the stub registry
    assert!(!ed.tool_active());
    assert_eq!(ed.active_tool(), None);
}

#[test]
fn activation_key_toggles_like_the_flameshot_button() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    assert!(ed.key_press(&env, KeyCode::KeyP, false, None).consumed);
    assert_eq!(ed.active_tool(), Some(ToolKind::Pencil));
    assert!(ed.key_press(&env, KeyCode::KeyP, false, None).consumed);
    assert!(!ed.tool_active(), "re-press unchecks");
    // Auto-repeat never re-toggles.
    ed.key_press(&env, KeyCode::KeyP, false, None);
    assert!(!ed.key_press(&env, KeyCode::KeyP, true, None).consumed);
    assert_eq!(ed.active_tool(), Some(ToolKind::Pencil));
}

// ---------------------------------------------------------------------------
// B. Undo / redo / delete (core UndoStack integration)
// ---------------------------------------------------------------------------

#[test]
fn undo_redo_roundtrip_restores_the_scene() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    stroke(&mut ed, &env, (0.0, 0.0), (50.0, 50.0));
    assert_eq!(ed.scene().object_count(), 1);
    assert!(ed.undo());
    assert_eq!(ed.scene().object_count(), 0);
    assert!(ed.redo());
    assert_eq!(ed.scene().object_count(), 1);
    // Ends are silent no-ops.
    assert!(!ed.redo());
    assert!(ed.undo());
    assert!(!ed.undo());
}

#[test]
fn undo_redo_run_through_the_shortcut_keys() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    stroke(&mut ed, &env, (0.0, 0.0), (50.0, 50.0));
    let undo_env = EditorEnv {
        modifiers: ModifiersState::CONTROL,
        ..env
    };
    assert!(ed.key_press(&undo_env, KeyCode::KeyZ, false, None).consumed);
    assert_eq!(ed.scene().object_count(), 0);
    let redo_env = EditorEnv {
        modifiers: ModifiersState::CONTROL | ModifiersState::SHIFT,
        ..env
    };
    assert!(ed.key_press(&redo_env, KeyCode::KeyZ, false, None).consumed);
    assert_eq!(ed.scene().object_count(), 1);
    // Plain 'z' is not a binding: passes through.
    assert!(!ed.key_press(&env, KeyCode::KeyZ, false, None).consumed);
}

#[test]
fn delete_removes_selected_and_core_renumbers_counters() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    for counter in [(10.0, 10.0), (100.0, 100.0), (200.0, 200.0)] {
        ed.commit_object(counter_object(counter.0, counter.1));
    }
    assert_eq!(ed.scene().counter_counts(), vec![1, 2, 3]);
    assert_eq!(ed.select_object_at(at(100.0, 100.0)), Some(1));
    let delete = ed.key_press(&env, KeyCode::Delete, false, None);
    assert!(delete.consumed && delete.changed);
    assert_eq!(ed.scene().counter_counts(), vec![1, 2], "core renumbered");
    assert_eq!(ed.selected_object(), None);
    assert_eq!(ed.undo_stack().undo_depth(), 4, "3 commits + 1 delete");
    assert!(ed.undo());
    assert_eq!(ed.scene().counter_counts(), vec![1, 2, 3], "delete undone");
    // Delete without a selection passes through.
    assert!(!ed.key_press(&env, KeyCode::Delete, false, None).consumed);
}

// ---------------------------------------------------------------------------
// C. Object selection (F27 P5)
// ---------------------------------------------------------------------------

#[test]
fn hit_test_picks_the_topmost_object() {
    let mut ed = editor();
    ed.commit_object(rect_object(0.0, 0.0, 100.0, 100.0));
    ed.commit_object(rect_object(50.0, 50.0, 100.0, 150.0));
    // Overlap point: the top-most (last painted) wins.
    assert_eq!(ed.object_at(at(75.0, 75.0)), Some(1));
    assert_eq!(ed.object_at(at(10.0, 10.0)), Some(0));
    assert_eq!(ed.object_at(at(500.0, 500.0)), None);
}

#[test]
fn press_selects_object_and_empty_press_deselects_passing_through() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.commit_object(rect_object(100.0, 100.0, 50.0, 50.0));
    let hit = press(&mut ed, &env, MouseButton::Left, 120.0, 120.0);
    assert!(
        hit.consumed,
        "the object press never reaches the region engine"
    );
    assert_eq!(ed.selected_object(), Some(0));
    // A press on empty space deselects AND passes to the selection engine.
    let miss = press(&mut ed, &env, MouseButton::Left, 500.0, 500.0);
    assert!(!miss.consumed);
    assert!(miss.changed, "the outline vanished -> every window redraws");
    assert_eq!(ed.selected_object(), None);
}

// ---------------------------------------------------------------------------
// D. Size dispatch, digits, wheel (editor level)
// ---------------------------------------------------------------------------

#[test]
fn digits_clip_at_fifty_and_reset_the_accumulator() {
    let mut ed = editor();
    let t0 = Instant::now();
    let env = env_at(t0);
    ed.activate_tool(ToolKind::Pencil);
    // Acceptance failure case: '9','9' -> 50.
    ed.key_press(&env, KeyCode::Digit9, false, None);
    assert_eq!(ed.tool_size(), 9);
    ed.key_press(
        &EditorEnv {
            now: t0 + Duration::from_millis(100),
            ..env
        },
        KeyCode::Digit9,
        false,
        None,
    );
    assert_eq!(ed.tool_size(), MAX_TOOL_SIZE);
    // Clip reset: the next digit starts fresh.
    ed.key_press(
        &EditorEnv {
            now: t0 + Duration::from_millis(200),
            ..env
        },
        KeyCode::Digit3,
        false,
        None,
    );
    assert_eq!(ed.tool_size(), 3);
    // The tool received the size notifications.
    assert!(take_log().iter().any(|entry| entry == "size-changed 50"));
}

#[test]
fn digits_write_to_the_active_tools_dispatch_slot() {
    let mut ed = editor();
    let t0 = Instant::now();
    let env = env_at(t0);
    ed.activate_tool(ToolKind::Marker); // independent slot, default 5
    assert_eq!(ed.tool_size(), 5);
    ed.key_press(&env, KeyCode::Digit1, false, None);
    ed.key_press(
        &EditorEnv {
            now: t0 + Duration::from_millis(50),
            ..env
        },
        KeyCode::Digit2,
        false,
        None,
    );
    assert_eq!(ed.tool_size(), 12, "marker slot");
    // The shared thickness slot is untouched.
    ed.deactivate_tool();
    assert_eq!(ed.tool_size(), 3);
}

#[test]
fn digits_reset_after_the_notifier_delay() {
    let mut ed = editor();
    let t0 = Instant::now();
    let env = env_at(t0);
    ed.activate_tool(ToolKind::Pencil);
    ed.key_press(&env, KeyCode::Digit4, false, None);
    assert_eq!(ed.tool_size(), 4);
    let later = EditorEnv {
        now: t0 + DIGIT_RESET_DELAY + Duration::from_millis(1),
        ..env
    };
    ed.key_press(&later, KeyCode::Digit2, false, None);
    assert_eq!(ed.tool_size(), 2, "stale accumulator dropped");
}

#[test]
fn wheel_notch_steps_size_by_one() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    assert_eq!(ed.tool_size(), 3);
    assert!(ed.wheel(&env, 120).consumed);
    assert_eq!(ed.tool_size(), 4);
    ed.wheel(&env, -120);
    assert_eq!(ed.tool_size(), 3);
    // Sub-threshold accumulates: 4 x 15 = one step.
    for _ in 0..3 {
        let update = ed.wheel(&env, 15);
        assert!(update.consumed && !update.changed);
        assert_eq!(ed.tool_size(), 3);
    }
    ed.wheel(&env, 15);
    assert_eq!(ed.tool_size(), 4);
    // Clips at the bounds.
    for _ in 0..60 {
        ed.wheel(&env, 120);
    }
    assert_eq!(ed.tool_size(), MAX_TOOL_SIZE);
    for _ in 0..60 {
        ed.wheel(&env, -120);
    }
    assert_eq!(ed.tool_size(), MIN_TOOL_SIZE);
}

#[test]
fn wheel_is_consumed_by_the_tool_first() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Circle); // stub consumes the wheel
    let before = ed.tool_size();
    let update = ed.wheel(&env, 120);
    assert!(update.consumed && update.changed);
    assert_eq!(ed.tool_size(), before, "size untouched - the tool ate it");
    assert!(take_log().contains(&"wheel-consumed 1".to_owned()));
}

#[test]
fn panel_size_seam_and_color_reach_the_tool() {
    let mut ed = editor();
    ed.activate_tool(ToolKind::Pencil);
    take_log();
    ed.set_tool_size(17);
    assert_eq!(ed.tool_size(), 17);
    ed.set_color(SceneColor::new(0, 255, 0, 255));
    assert_eq!(ed.color(), SceneColor::new(0, 255, 0, 255));
    let log = take_log();
    assert!(log.contains(&"size-changed 17".to_owned()));
    assert!(log.contains(&"color-changed".to_owned()));
}

// ---------------------------------------------------------------------------
// E. EditorContext plumbing
// ---------------------------------------------------------------------------

#[test]
fn context_carries_frame_selection_color_size_and_counter() {
    let mut ed = editor();
    ed.install_frame(Some(FramePixels {
        rgba: vec![0u8; 4 * 2 * 2],
        width: 2,
        height: 2,
        scale: 1.0,
        origin: LogicalPoint::from_raw(0.0, 0.0),
    }));
    ed.commit_object(counter_object(10.0, 10.0)); // next counter = 2
    ed.set_color(SceneColor::new(1, 2, 3, 255));
    ed.activate_tool(ToolKind::Pencil);
    ed.set_tool_size(7);
    take_log();
    let env = EditorEnv {
        selection: Some(LogicalRect::from_raw(0.0, 0.0, 800.0, 600.0)),
        ..env_at(Instant::now())
    };
    press(&mut ed, &env, MouseButton::Left, 50.0, 50.0);
    let log = take_log();
    let start = log
        .iter()
        .find(|entry| entry.starts_with("draw_start"))
        .expect("draw_start logged the context");
    assert_eq!(
        start,
        "draw_start frame=1 selection=1 color=1,2,3 size=7 count=2 ctrl=0 shift=0"
    );
}

// ---------------------------------------------------------------------------
// F. Edit-widget routing (the todo-22 seams, framework level)
// ---------------------------------------------------------------------------

#[test]
fn modifier_snapshot_reaches_the_tool_context() {
    let mut ed = editor();
    ed.activate_tool(ToolKind::Pencil);
    take_log();
    let env = EditorEnv {
        modifiers: ModifiersState::CONTROL | ModifiersState::SHIFT,
        ..env_at(Instant::now())
    };
    press(&mut ed, &env, MouseButton::Left, 50.0, 50.0);
    let log = take_log();
    let start = log
        .iter()
        .find(|entry| entry.starts_with("draw_start"))
        .expect("draw_start logged the context");
    assert!(start.ends_with("ctrl=1 shift=1"), "{start}");
}

#[test]
fn right_click_opens_the_wheel_except_during_an_edit() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    // No tool: F27 P2 -> ColorWheel effect.
    let update = press(&mut ed, &env, MouseButton::Right, 500.0, 500.0);
    assert!(update.consumed);
    assert_eq!(update.effects, vec![EditorEffect::ColorWheel]);
    // Draw tool active, no edit: still the wheel.
    ed.activate_tool(ToolKind::Pencil);
    let update = press(&mut ed, &env, MouseButton::Right, 500.0, 500.0);
    assert_eq!(update.effects, vec![EditorEffect::ColorWheel]);
    // Text edit active: the exception - the tool consumes, NO wheel.
    ed.activate_tool(ToolKind::Text);
    take_log();
    let update = press(&mut ed, &env, MouseButton::Right, 500.0, 500.0);
    assert!(update.consumed);
    assert!(update.effects.is_empty());
    assert!(take_log().contains(&"edit-press".to_owned()));
}

#[test]
fn click_outside_the_edit_commits_it_and_consumes() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Text);
    take_log();
    let update = press(&mut ed, &env, MouseButton::Left, 500.0, 500.0);
    assert!(update.consumed && update.changed);
    assert!(take_log().contains(&"commit-edit".to_owned()));
    assert_eq!(ed.scene().object_count(), 1, "commit = one undo unit");
    assert_eq!(ed.undo_stack().undo_depth(), 1);
}

#[test]
fn click_inside_the_edit_goes_to_the_tool() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Text);
    take_log();
    let update = press(&mut ed, &env, MouseButton::Left, 150.0, 150.0);
    assert!(update.consumed);
    let log = take_log();
    assert!(log.contains(&"edit-press".to_owned()));
    assert!(!log.contains(&"commit-edit".to_owned()));
    assert_eq!(ed.scene().object_count(), 0);
}

#[test]
fn ctrl_enter_commits_the_edit() {
    let mut ed = editor();
    let env = EditorEnv {
        modifiers: ModifiersState::CONTROL,
        mouse: Some(at(150.0, 150.0)),
        ..env_at(Instant::now())
    };
    ed.activate_tool(ToolKind::Text);
    take_log();
    let update = ed.key_press(&env, KeyCode::Enter, false, None);
    assert!(update.consumed);
    assert!(take_log().contains(&"commit-edit".to_owned()));
    assert_eq!(ed.scene().object_count(), 1);
    // Plain Enter passes through (the selection engine's Accept).
    let plain = env_at(Instant::now());
    assert!(!ed.key_press(&plain, KeyCode::Enter, false, None).consumed);
}

#[test]
fn detached_widget_flag_drives_editing_until_stage_four() {
    let mut ed = editor();
    ed.activate_tool(ToolKind::Text);
    assert!(ed.editing());
    // Unchecking the tool keeps the detached widget flag (Flameshot:
    // stage 1 does not delete m_toolWidget).
    ed.set_edit_widget_present(true);
    ed.deactivate_tool();
    assert!(ed.editing(), "detached widget survives the uncheck");
    ed.delete_tool_widget();
    assert!(!ed.editing());
}

// ---------------------------------------------------------------------------
// G. Cascade sync (the Esc hooks becoming real)
// ---------------------------------------------------------------------------

#[test]
fn sync_cascade_mirrors_the_editor_occupancy() {
    use crate::selection::CascadeState;
    let mut ed = editor();
    let mut cascade = CascadeState::empty();
    ed.sync_cascade(&mut cascade);
    assert_eq!(cascade, CascadeState::empty());
    ed.activate_tool(ToolKind::Text);
    ed.commit_object(rect_object(0.0, 0.0, 50.0, 50.0));
    ed.select_object_at(at(10.0, 10.0));
    ed.sync_cascade(&mut cascade);
    assert!(cascade.tool_checked() && cascade.object_selected() && cascade.tool_widget_present());
    // Panel/picker flags (todo 26) are never touched by the sync.
    cascade.set_panel_visible(true);
    cascade.set_picker_visible(true);
    ed.deactivate_tool();
    ed.deselect_object();
    ed.delete_tool_widget();
    ed.sync_cascade(&mut cascade);
    assert!(
        !cascade.tool_checked() && !cascade.object_selected() && !cascade.tool_widget_present()
    );
    assert!(cascade.panel_visible() && cascade.picker_visible());
}

// ---------------------------------------------------------------------------
// H. Paint bridge
// ---------------------------------------------------------------------------

fn paint_commands(ed: &EditorState, mouse: Option<LogicalPoint>) -> Vec<Command> {
    let out = output();
    let mut list = DisplayList::new();
    ed.paint_into(
        &mut list,
        &out,
        EditorView {
            mouse,
            selection: None,
            modifiers: ModifiersState::empty(),
        },
    );
    list.iter().cloned().collect()
}

#[test]
fn paint_renders_scene_outline_and_preview_in_order() {
    let mut ed = editor();
    ed.commit_object(rect_object(100.0, 100.0, 50.0, 50.0));
    ed.select_object_at(at(120.0, 120.0));
    ed.activate_tool(ToolKind::Pencil);
    take_log();
    let commands = paint_commands(&ed, Some(at(400.0, 400.0)));
    // 1: the scene object stroke.
    assert!(matches!(
        commands[0],
        Command::Stroke {
            shape: Shape::Rect { .. },
            ..
        }
    ));
    // 2: the black 3px outline box, then the white dots.
    let Command::Stroke { width, color, .. } = &commands[1] else {
        panic!("outline box");
    };
    assert_eq!(*width, OBJECT_OUTLINE_OUTER);
    assert!(color.r < 0.01 && color.g < 0.01 && color.b < 0.01);
    assert!(commands[2..].iter().any(|command| matches!(
        command,
        Command::Stroke { shape: Shape::Line { .. }, width, color }
            if *width == OBJECT_OUTLINE_INNER && color.r > 0.99 && color.g > 0.99
    )));
    // Last: the tool's mouse preview (an ellipse fill at the cursor).
    assert!(matches!(
        commands.last(),
        Some(Command::Fill {
            shape: Shape::Ellipse { .. },
            ..
        })
    ));
    assert!(take_log().contains(&"paint-preview".to_owned()));
}

#[test]
fn preview_is_gated_by_config_and_tool() {
    let mut ed = editor();
    // Config gate off: scene-only paint.
    ed.configure(EditorTools {
        mouse_preview: false,
        ..EditorTools::default()
    });
    ed.activate_tool(ToolKind::Pencil);
    take_log();
    assert!(paint_commands(&ed, Some(at(400.0, 400.0))).is_empty());
    // Tool gate off (marker stub): no preview either.
    ed.configure(EditorTools::default());
    ed.activate_tool(ToolKind::Marker);
    take_log();
    assert!(paint_commands(&ed, Some(at(400.0, 400.0))).is_empty());
    assert!(!take_log().contains(&"quiet-paint-UNEXPECTED".to_owned()));
    // No cursor yet: no preview, but the scene still paints.
    ed.activate_tool(ToolKind::Pencil);
    ed.commit_object(rect_object(0.0, 0.0, 10.0, 10.0));
    let commands = paint_commands(&ed, None);
    assert_eq!(commands.len(), 1, "scene object only");
}

#[test]
fn in_progress_stroke_paints_between_press_and_release() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    press(&mut ed, &env, MouseButton::Left, 100.0, 100.0);
    ed.pointer_move(&env, at(200.0, 200.0));
    take_log();
    let commands = paint_commands(&ed, Some(at(200.0, 200.0)));
    assert!(matches!(
        commands.last(),
        Some(Command::Stroke {
            shape: Shape::Line { .. },
            ..
        })
    ));
    assert!(take_log().contains(&"paint-stroke".to_owned()));
    release(&mut ed, &env, MouseButton::Left, 200.0, 200.0);
}

// ---------------------------------------------------------------------------
// I. OverlayCore funnel integration (the production routing path)
// ---------------------------------------------------------------------------

const SLOT: WindowSlot = WindowSlot::new(0);

fn funnel_core() -> OverlayCore {
    let output = OutputInfo::new(
        "DP-1",
        "DP-1",
        LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
        PhysicalSize::from_raw(1920, 1080),
        1.0,
        Transform::Normal,
    )
    .expect("valid fixture output");
    let layout = OutputLayout::new(vec![output]);
    let mut core = OverlayCore::new(InputRouter::new(layout, vec![0]));
    *core.editor_mut().registry_mut() = stub_registry();
    take_log();
    core
}

fn move_to(core: &mut OverlayCore, x: f64, y: f64) -> RouteReport {
    core.inject_event(SyntheticInput::pointer_moved(SLOT, x, y))
}

fn click(core: &mut OverlayCore, button: MouseButton, pressed: bool) -> RouteReport {
    core.inject_event(SyntheticInput::pointer_button(SLOT, button, pressed))
}

fn tap(core: &mut OverlayCore, key_code: KeyCode) -> RouteReport {
    core.inject_event(SyntheticInput::key_press(SLOT, key_code))
}

#[test]
fn funnel_tool_drag_commits_a_stroke_and_leaves_the_region_alone() {
    let mut core = funnel_core();
    tap(&mut core, KeyCode::KeyP);
    assert_eq!(core.editor().active_tool(), Some(ToolKind::Pencil));
    assert!(core.selection().cascade().tool_checked(), "cascade synced");
    move_to(&mut core, 400.0, 300.0);
    click(&mut core, MouseButton::Left, true);
    move_to(&mut core, 700.0, 500.0);
    click(&mut core, MouseButton::Left, false);
    assert_eq!(core.editor().scene().object_count(), 1);
    assert_eq!(core.editor().undo_stack().undo_depth(), 1);
    // The tool drag never reached the selection engine (F27 P3 > region).
    assert_eq!(core.selection().rect(), None);
}

#[test]
fn funnel_without_a_tool_still_creates_the_selection() {
    // Todo-16 regression: the editor passes region presses through.
    let mut core = funnel_core();
    move_to(&mut core, 100.0, 100.0);
    click(&mut core, MouseButton::Left, true);
    move_to(&mut core, 400.0, 300.0);
    click(&mut core, MouseButton::Left, false);
    assert_eq!(
        core.selection().rect(),
        Some(LogicalRect::from_raw(100.0, 100.0, 300.0, 200.0))
    );
    assert_eq!(core.editor().scene().object_count(), 0);
}

#[test]
fn funnel_right_click_emits_the_color_wheel_action() {
    let mut core = funnel_core();
    move_to(&mut core, 500.0, 400.0);
    let report = click(&mut core, MouseButton::Right, true);
    assert!(report.actions.contains(&Action::ColorWheel));
    assert_eq!(core.selection().rect(), None);
}

#[test]
fn funnel_digits_clip_to_fifty() {
    let mut core = funnel_core();
    tap(&mut core, KeyCode::KeyP);
    tap(&mut core, KeyCode::Digit9);
    tap(&mut core, KeyCode::Digit9);
    assert_eq!(core.editor().tool_size(), MAX_TOOL_SIZE);
}

#[test]
fn funnel_wheel_adjusts_the_shared_thickness() {
    let mut core = funnel_core();
    assert_eq!(core.editor().tool_size(), 3);
    core.inject_event(SyntheticInput::wheel(SLOT, 120));
    assert_eq!(core.editor().tool_size(), 4);
    core.inject_event(SyntheticInput::wheel(SLOT, -59));
    assert_eq!(core.editor().tool_size(), 4, "sub-threshold accumulates");
    core.inject_event(SyntheticInput::wheel(SLOT, -1));
    assert_eq!(core.editor().tool_size(), 3);
}

#[test]
fn funnel_esc_walks_the_real_tool_stage_then_closes() {
    let mut core = funnel_core();
    tap(&mut core, KeyCode::KeyP);
    assert!(core.selection().cascade().tool_checked());
    // Esc 1: deselect the tool - no exit.
    let report = tap(&mut core, KeyCode::Escape);
    assert!(!report.actions.contains(&Action::Exit));
    assert!(!core.editor().tool_active());
    assert!(!core.selection().cascade().tool_checked());
    assert!(!core.exit_requested());
    // Esc 2: empty cascade -> close.
    let report = tap(&mut core, KeyCode::Escape);
    assert!(report.actions.contains(&Action::Exit));
    assert!(core.exit_requested());
}

#[test]
fn funnel_esc_deselects_the_object_stage() {
    let mut core = funnel_core();
    core.editor_mut()
        .commit_object(rect_object(100.0, 100.0, 50.0, 50.0));
    move_to(&mut core, 120.0, 120.0);
    click(&mut core, MouseButton::Left, true);
    click(&mut core, MouseButton::Left, false);
    assert_eq!(core.editor().selected_object(), Some(0));
    assert!(core.selection().cascade().object_selected());
    let report = tap(&mut core, KeyCode::Escape);
    assert!(!report.actions.contains(&Action::Exit));
    assert_eq!(core.editor().selected_object(), None);
    assert!(!core.selection().cascade().object_selected());
}

#[test]
fn funnel_picker_swallows_presses_until_click_away() {
    // Todo 26: stage 5 is CHROME-driven (raw cascade pokes are futile - the
    // todo-20 stage 1/2/4 precedent); the real producer is the right-click
    // wheel-open through the funnel.
    let mut core = funnel_core();
    move_to(&mut core, 100.0, 100.0);
    click(&mut core, MouseButton::Right, true);
    click(&mut core, MouseButton::Right, false);
    assert!(core.chrome().color_wheel.visible);
    assert!(
        core.selection().cascade().picker_visible(),
        "cascade synced"
    );
    // P1: while the picker is visible, a left press never reaches the
    // region engine and starts no draw session - the click-away only hides.
    move_to(&mut core, 400.0, 300.0);
    click(&mut core, MouseButton::Left, true);
    assert_eq!(core.selection().rect(), None);
    assert_eq!(core.editor().scene().object_count(), 0);
    assert!(!core.chrome().color_wheel.visible);
    assert!(!core.selection().cascade().picker_visible());
    click(&mut core, MouseButton::Left, false);
}

#[test]
fn funnel_object_press_selects_and_blocks_the_region_drag() {
    let mut core = funnel_core();
    core.editor_mut()
        .commit_object(rect_object(100.0, 100.0, 80.0, 80.0));
    move_to(&mut core, 120.0, 120.0);
    click(&mut core, MouseButton::Left, true);
    move_to(&mut core, 500.0, 500.0);
    click(&mut core, MouseButton::Left, false);
    assert_eq!(core.editor().selected_object(), Some(0));
    assert_eq!(
        core.selection().rect(),
        None,
        "F27 P5 beats the region move"
    );
}

#[test]
fn funnel_undo_redo_toggle_the_stroke_visibly() {
    // The live-QA undo/redo acceptance at the funnel level.
    let mut core = funnel_core();
    tap(&mut core, KeyCode::KeyP);
    move_to(&mut core, 400.0, 300.0);
    click(&mut core, MouseButton::Left, true);
    move_to(&mut core, 900.0, 700.0);
    click(&mut core, MouseButton::Left, false);
    assert_eq!(core.editor().scene().object_count(), 1);
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::CONTROL));
    let report = tap(&mut core, KeyCode::KeyZ);
    assert!(report.actions.contains(&Action::Redraw(SLOT)));
    assert_eq!(core.editor().scene().object_count(), 0);
    core.inject_event(SyntheticInput::modifiers(
        SLOT,
        ModifiersState::CONTROL | ModifiersState::SHIFT,
    ));
    tap(&mut core, KeyCode::KeyZ);
    assert_eq!(core.editor().scene().object_count(), 1);
}

#[test]
fn funnel_selection_keys_still_reach_the_engine() {
    // Ctrl+A / Enter are the selection engine's (todo-16 contract) even
    // with a tool active.
    let mut core = funnel_core();
    tap(&mut core, KeyCode::KeyP);
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::CONTROL));
    tap(&mut core, KeyCode::KeyA);
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::empty()));
    assert_eq!(
        core.selection().rect(),
        Some(LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0))
    );
    let report = tap(&mut core, KeyCode::Enter);
    assert!(report.actions.contains(&Action::Accept));
}

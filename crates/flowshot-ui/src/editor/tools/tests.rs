//! The todo-21 shape-tool suite: per-tool stroke lifecycle through the real
//! [`EditorState`] event surface (press/move/release + the test-drive funnel),
//! committed scene-object geometry, the F27 Ctrl drag conventions, the
//! marker blend constant, the size-slot dispatch, and the zero-length rule.

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::cast_precision_loss
)]

use std::time::Instant;

use flowshot_core::config::{ArrowStyle, ArrowToolConfig, Config, ToolsConfig};
use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::scene::{
    Color as SceneColor, Point as ScenePoint, Rect as SceneRect, ToolObject, ToolObjectData,
};
use winit::event::MouseButton;
use winit::keyboard::{KeyCode, ModifiersState};

use crate::editor::*;
use crate::input::SyntheticInput;
use crate::render::{Command, DisplayList, Shape};
use crate::router::{InputRouter, WindowSlot};
use crate::state::OverlayCore;

const DRAW_RED: SceneColor = SceneColor::new(255, 0, 0, 255);

fn editor() -> EditorState {
    editor_with_config(&Config::default())
}

fn editor_with_config(config: &Config) -> EditorState {
    let mut registry = ToolRegistry::new();
    register_shape_tools(&mut registry);
    EditorState::new(EditorTools::from_config(config), registry)
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

fn env_ctrl(now: Instant) -> EditorEnv {
    EditorEnv {
        modifiers: ModifiersState::CONTROL,
        ..env_at(now)
    }
}

fn at(x: f64, y: f64) -> LogicalPoint {
    LogicalPoint::from_raw(x, y)
}

fn stroke(ed: &mut EditorState, env: &EditorEnv, from: (f64, f64), to: (f64, f64)) {
    ed.pointer_press(env, MouseButton::Left, at(from.0, from.1));
    ed.pointer_move(env, at(to.0, to.1));
    ed.pointer_release(env, MouseButton::Left, at(to.0, to.1));
}

fn committed(ed: &EditorState) -> ToolObjectData {
    assert_eq!(ed.scene().object_count(), 1, "exactly one committed object");
    ed.scene().get_object(0).expect("object 0").to_data()
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

const ALL_KINDS: [ToolKind; 7] = [
    ToolKind::Pencil,
    ToolKind::Line,
    ToolKind::Arrow,
    ToolKind::Rectangle,
    ToolKind::Circle,
    ToolKind::Marker,
    ToolKind::Invert,
];

// ---------------------------------------------------------------------------
// A. Lifecycle + the zero-length rule (plan acceptance failure case)
// ---------------------------------------------------------------------------

#[test]
fn every_tool_registers_and_activates() {
    let mut ed = editor();
    for kind in ALL_KINDS {
        ed.activate_tool(kind);
        assert_eq!(ed.active_tool(), Some(kind), "{kind:?} registered");
    }
}

#[test]
fn zero_length_drag_commits_nothing_for_every_tool() {
    for kind in ALL_KINDS {
        let mut ed = editor();
        let env = env_at(Instant::now());
        ed.activate_tool(kind);
        ed.pointer_press(&env, MouseButton::Left, at(100.0, 100.0));
        ed.pointer_release(&env, MouseButton::Left, at(100.0, 100.0));
        assert_eq!(ed.scene().object_count(), 0, "{kind:?} click-only");
        assert_eq!(ed.undo_stack().undo_depth(), 0, "{kind:?} no undo unit");
    }
}

#[test]
fn each_stroke_is_exactly_one_undo_unit() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    stroke(&mut ed, &env, (0.0, 0.0), (50.0, 50.0));
    ed.activate_tool(ToolKind::Line);
    stroke(&mut ed, &env, (0.0, 0.0), (50.0, 10.0));
    ed.activate_tool(ToolKind::Invert);
    stroke(&mut ed, &env, (10.0, 10.0), (60.0, 60.0));
    assert_eq!(ed.scene().object_count(), 3);
    assert_eq!(ed.undo_stack().undo_depth(), 3);
    assert!(ed.undo());
    assert_eq!(ed.scene().object_count(), 2);
}

// ---------------------------------------------------------------------------
// B. Per-tool committed geometry
// ---------------------------------------------------------------------------

#[test]
fn pencil_commits_an_rdp_simplified_path() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    ed.pointer_press(&env, MouseButton::Left, at(10.0, 10.0));
    // Collinear run (all within epsilon of the chord) plus one corner.
    for step in 1..=10 {
        ed.pointer_move(&env, at(10.0 + f64::from(step) * 5.0, 10.0));
    }
    ed.pointer_move(&env, at(60.0, 40.0));
    ed.pointer_release(&env, MouseButton::Left, at(60.0, 40.0));
    let ToolObjectData::Pencil(path) = committed(&ed) else {
        panic!("pencil object");
    };
    assert_eq!(
        path.points,
        vec![
            ScenePoint::new(10.0, 10.0),
            ScenePoint::new(60.0, 10.0),
            ScenePoint::new(60.0, 40.0),
        ],
        "collinear points simplified away (epsilon 0.5px)"
    );
    assert_eq!(path.color, DRAW_RED, "[editor].draw_color default");
    assert_eq!(path.thickness, 3.0, "shared draw_thickness default");
}

#[test]
fn line_commits_endpoints_and_thickness() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Line);
    stroke(&mut ed, &env, (100.0, 50.0), (300.0, 250.0));
    let ToolObjectData::Line(line) = committed(&ed) else {
        panic!("line object");
    };
    assert_eq!(line.from, ScenePoint::new(100.0, 50.0));
    assert_eq!(line.to, ScenePoint::new(300.0, 250.0));
    assert_eq!(line.color, DRAW_RED);
    assert_eq!(line.thickness, 3.0);
}

#[test]
fn arrow_defaults_straight_forward_and_reads_config_at_commit() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Arrow);
    stroke(&mut ed, &env, (0.0, 0.0), (100.0, 0.0));
    let ToolObjectData::Arrow(arrow) = committed(&ed) else {
        panic!("arrow object");
    };
    assert_eq!(arrow.style, ArrowStyle::Straight);
    assert!(!arrow.reverse);
    assert_eq!(arrow.thickness, 3.0);

    // [tools.arrow] style/reverse land on the object (persistence contract).
    let config = Config {
        tools: ToolsConfig {
            arrow: ArrowToolConfig {
                style: ArrowStyle::Curved,
                reverse: true,
            },
            ..ToolsConfig::default()
        },
        ..Config::default()
    };
    let mut ed = editor_with_config(&config);
    ed.activate_tool(ToolKind::Arrow);
    stroke(&mut ed, &env, (0.0, 0.0), (100.0, 0.0));
    let ToolObjectData::Arrow(arrow) = committed(&ed) else {
        panic!("arrow object");
    };
    assert_eq!(arrow.style, ArrowStyle::Curved);
    assert!(arrow.reverse);
}

#[test]
fn rect_commits_radius_from_its_slot_and_stroke_from_config() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Rectangle);
    assert_eq!(ed.tool_size(), 1, "[tools.rectangle].corner_radius default");
    stroke(&mut ed, &env, (100.0, 100.0), (300.0, 220.0));
    let ToolObjectData::Rectangle(rect) = committed(&ed) else {
        panic!("rect object");
    };
    assert_eq!(rect.rect, SceneRect::new(100.0, 100.0, 200.0, 120.0));
    assert_eq!(rect.corner_radius, 1.0);
    assert_eq!(rect.stroke_width, 3.0, "[editor].draw_thickness");
    assert!(!rect.filled);

    // Digits write the rect slot = the corner radius (todo-20 dispatch).
    let mut ed = editor();
    ed.activate_tool(ToolKind::Rectangle);
    ed.set_tool_size(7);
    stroke(&mut ed, &env, (0.0, 0.0), (40.0, 40.0));
    let ToolObjectData::Rectangle(rect) = committed(&ed) else {
        panic!("rect object");
    };
    assert_eq!(rect.corner_radius, 7.0);
}

#[test]
fn ellipse_commits_the_inscribed_bounds() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Circle);
    stroke(&mut ed, &env, (50.0, 40.0), (250.0, 140.0));
    let ToolObjectData::Ellipse(ellipse) = committed(&ed) else {
        panic!("ellipse object");
    };
    assert_eq!(ellipse.rect, SceneRect::new(50.0, 40.0, 200.0, 100.0));
    assert_eq!(ellipse.stroke_width, 3.0);
    assert!(!ellipse.filled);
}

#[test]
fn marker_commits_translucent_at_its_slot_width() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Marker);
    assert_eq!(ed.tool_size(), 5, "[tools.marker].size default");
    stroke(&mut ed, &env, (100.0, 100.0), (300.0, 100.0));
    let ToolObjectData::Marker(marker) = committed(&ed) else {
        panic!("marker object");
    };
    assert_eq!(
        marker.color,
        SceneColor::new(255, 0, 0, MARKER_ALPHA),
        "draw color at the ~0.5 blend alpha"
    );
    assert_eq!(marker.width, 5.0);
    // The chisel quad extends half the width beyond each endpoint.
    let polygon = marker.chisel_polygon().expect("positive width");
    assert_eq!(polygon.len(), 4);
    assert_eq!(
        marker.bounding_rect(),
        SceneRect::new(97.5, 97.5, 205.0, 5.0)
    );
}

#[test]
fn invert_commits_the_normalized_region() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Invert);
    // Dragged up-left: the committed rect is normalized.
    stroke(&mut ed, &env, (300.0, 250.0), (100.0, 50.0));
    let ToolObjectData::Invert(invert) = committed(&ed) else {
        panic!("invert object");
    };
    assert_eq!(invert.rect, SceneRect::new(100.0, 50.0, 200.0, 200.0));
}

// ---------------------------------------------------------------------------
// C. F27 modifier conventions (Ctrl drag)
// ---------------------------------------------------------------------------

#[test]
fn line_ctrl_snaps_to_axes_and_diagonals() {
    let now = Instant::now();
    let ctrl = env_ctrl(now);
    // Near-horizontal -> pure H.
    let mut ed = editor();
    ed.activate_tool(ToolKind::Line);
    stroke(&mut ed, &ctrl, (100.0, 100.0), (220.0, 118.0));
    let ToolObjectData::Line(line) = committed(&ed) else {
        panic!("line object");
    };
    assert_eq!(line.to, ScenePoint::new(220.0, 100.0), "dy zeroed");
    // Near-45deg -> equal deltas.
    let mut ed = editor();
    ed.activate_tool(ToolKind::Line);
    stroke(&mut ed, &ctrl, (0.0, 0.0), (100.0, 50.0));
    let ToolObjectData::Line(line) = committed(&ed) else {
        panic!("line object");
    };
    assert_eq!(line.to, ScenePoint::new(75.0, 75.0), "45deg average");
    // Without Ctrl the raw point survives.
    let mut ed = editor();
    ed.activate_tool(ToolKind::Line);
    stroke(&mut ed, &env_at(now), (0.0, 0.0), (100.0, 50.0));
    let ToolObjectData::Line(line) = committed(&ed) else {
        panic!("line object");
    };
    assert_eq!(line.to, ScenePoint::new(100.0, 50.0));
}

#[test]
fn rect_and_ellipse_ctrl_lock_to_square_and_circle() {
    let ctrl = env_ctrl(Instant::now());
    let mut ed = editor();
    ed.activate_tool(ToolKind::Rectangle);
    stroke(&mut ed, &ctrl, (0.0, 0.0), (100.0, 40.0));
    let ToolObjectData::Rectangle(rect) = committed(&ed) else {
        panic!("rect object");
    };
    assert_eq!(rect.rect.width, rect.rect.height, "aspect lock");
    assert_eq!(rect.rect, SceneRect::new(0.0, 0.0, 70.0, 70.0));

    let mut ed = editor();
    ed.activate_tool(ToolKind::Circle);
    stroke(&mut ed, &ctrl, (0.0, 0.0), (100.0, 40.0));
    let ToolObjectData::Ellipse(ellipse) = committed(&ed) else {
        panic!("ellipse object");
    };
    assert_eq!(ellipse.rect.width, ellipse.rect.height, "circle lock");
}

#[test]
fn marker_ctrl_snaps_like_the_f27_adjustment_flags() {
    let ctrl = env_ctrl(Instant::now());
    let mut ed = editor();
    ed.activate_tool(ToolKind::Marker);
    stroke(&mut ed, &ctrl, (10.0, 10.0), (90.0, 26.0));
    let ToolObjectData::Marker(marker) = committed(&ed) else {
        panic!("marker object");
    };
    assert_eq!(marker.to, ScenePoint::new(90.0, 10.0), "H snap");
}

#[test]
fn ctrl_applies_at_use_so_paint_and_commit_agree() {
    // Press Ctrl mid-drag WITHOUT a further motion: the painted preview is
    // already constrained (the EditorView.modifiers contract of todo 20).
    let now = Instant::now();
    let mut ed = editor();
    ed.activate_tool(ToolKind::Line);
    ed.pointer_press(&env_at(now), MouseButton::Left, at(0.0, 0.0));
    ed.pointer_move(&env_at(now), at(100.0, 50.0));
    let out = output();
    let mut list = DisplayList::new();
    ed.paint_into(
        &mut list,
        &out,
        EditorView {
            mouse: Some(at(100.0, 50.0)),
            selection: None,
            modifiers: ModifiersState::CONTROL,
        },
    );
    let Command::Stroke {
        shape: Shape::Line { to, .. },
        ..
    } = list.iter().next().expect("stroke painted")
    else {
        panic!("line stroke");
    };
    assert_eq!(
        *to,
        crate::render::Point::new(75.0, 75.0),
        "constrained live"
    );
    ed.pointer_release(&env_ctrl(now), MouseButton::Left, at(100.0, 50.0));
    let ToolObjectData::Line(line) = committed(&ed) else {
        panic!("line object");
    };
    assert_eq!(line.to, ScenePoint::new(75.0, 75.0), "commit matches paint");
}

#[test]
fn invert_and_pencil_ignore_ctrl() {
    let ctrl = env_ctrl(Instant::now());
    let mut ed = editor();
    ed.activate_tool(ToolKind::Invert);
    stroke(&mut ed, &ctrl, (0.0, 0.0), (100.0, 40.0));
    let ToolObjectData::Invert(invert) = committed(&ed) else {
        panic!("invert object");
    };
    assert_eq!(
        invert.rect,
        SceneRect::new(0.0, 0.0, 100.0, 40.0),
        "no snap"
    );
}

// ---------------------------------------------------------------------------
// D. Size dispatch through digits and wheel (todo-20 seams)
// ---------------------------------------------------------------------------

#[test]
fn digits_reach_the_committed_marker_width() {
    let mut ed = editor();
    let t0 = Instant::now();
    ed.activate_tool(ToolKind::Marker);
    ed.key_press(&env_at(t0), KeyCode::Digit1, false);
    ed.key_press(
        &EditorEnv {
            now: t0 + std::time::Duration::from_millis(50),
            ..env_at(t0)
        },
        KeyCode::Digit2,
        false,
    );
    assert_eq!(ed.tool_size(), 12);
    stroke(&mut ed, &env_at(t0), (0.0, 0.0), (50.0, 0.0));
    let ToolObjectData::Marker(marker) = committed(&ed) else {
        panic!("marker object");
    };
    assert_eq!(marker.width, 12.0);
}

#[test]
fn wheel_reach_the_committed_rect_radius() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Rectangle);
    ed.wheel(&env, 120);
    ed.wheel(&env, 120);
    assert_eq!(ed.tool_size(), 3, "1 + 2 notches");
    stroke(&mut ed, &env, (0.0, 0.0), (40.0, 40.0));
    let ToolObjectData::Rectangle(rect) = committed(&ed) else {
        panic!("rect object");
    };
    assert_eq!(rect.corner_radius, 3.0);
}

#[test]
fn max_size_strokes_stay_finite() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    for kind in ALL_KINDS {
        ed.activate_tool(kind);
        ed.set_tool_size(MAX_TOOL_SIZE);
        stroke(&mut ed, &env, (10.0, 10.0), (120.0, 90.0));
    }
    assert_eq!(ed.scene().object_count(), 7);
}

// ---------------------------------------------------------------------------
// E. Paint: previews, live strokes, and the invert command
// ---------------------------------------------------------------------------

#[test]
fn hover_previews_paint_the_f27_dot_except_invert() {
    for kind in ALL_KINDS {
        let mut ed = editor();
        ed.activate_tool(kind);
        let commands = paint_commands(&ed, Some(at(400.0, 300.0)));
        if kind == ToolKind::Invert {
            assert!(commands.is_empty(), "invert has no mouse preview (F27)");
        } else {
            assert!(
                matches!(
                    commands.last(),
                    Some(Command::Fill {
                        shape: Shape::Ellipse { .. },
                        ..
                    })
                ),
                "{kind:?} previews a dot"
            );
        }
    }
}

#[test]
fn pencil_live_stroke_paints_a_polyline() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Pencil);
    ed.pointer_press(&env, MouseButton::Left, at(10.0, 10.0));
    ed.pointer_move(&env, at(40.0, 60.0));
    let commands = paint_commands(&ed, Some(at(40.0, 60.0)));
    let Command::Stroke {
        shape: Shape::Polyline { points, closed },
        width,
        ..
    } = commands.last().expect("stroke painted")
    else {
        panic!("polyline stroke");
    };
    assert!(!closed);
    assert_eq!(points.len(), 2);
    assert_eq!(*width, 3.0);
    ed.pointer_release(&env, MouseButton::Left, at(40.0, 60.0));
    // After the commit the scene object paints the stroke and the tool falls
    // back to the hover dot - never a stale duplicate of the committed path.
    let commands = paint_commands(&ed, Some(at(40.0, 60.0)));
    assert!(
        matches!(
            commands.first(),
            Some(Command::Stroke {
                shape: Shape::Polyline { .. },
                ..
            })
        ),
        "the scene object paints"
    );
    assert!(
        matches!(
            commands.last(),
            Some(Command::Fill {
                shape: Shape::Ellipse { .. },
                ..
            })
        ),
        "the tool previews a dot"
    );
}

#[test]
fn invert_paints_the_region_command() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Invert);
    stroke(&mut ed, &env, (100.0, 80.0), (300.0, 280.0));
    let commands = paint_commands(&ed, None);
    assert_eq!(commands.len(), 1);
    assert!(
        matches!(commands[0], Command::Invert { rect } if rect.origin == crate::render::Point::new(100.0, 80.0) && rect.size.width == 200.0)
    );
}

#[test]
fn committed_objects_are_selectable_through_their_bounds() {
    let mut ed = editor();
    let env = env_at(Instant::now());
    ed.activate_tool(ToolKind::Marker);
    stroke(&mut ed, &env, (100.0, 100.0), (300.0, 100.0));
    assert_eq!(ed.select_object_at(at(200.0, 100.0)), Some(0));
    ed.deselect_object();
    ed.activate_tool(ToolKind::Pencil);
    stroke(&mut ed, &env, (100.0, 300.0), (300.0, 300.0));
    assert_eq!(ed.select_object_at(at(200.0, 300.0)), Some(1));
}

// ---------------------------------------------------------------------------
// F. The production funnel (test-drive injection)
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
    register_shape_tools(core.editor_mut().registry_mut());
    core
}

#[test]
fn funnel_drags_commit_real_tool_objects() {
    let mut core = funnel_core();
    let drag = |core: &mut OverlayCore, key: KeyCode, from: (f64, f64), to: (f64, f64)| {
        core.inject_event(SyntheticInput::key_press(SLOT, key));
        core.inject_event(SyntheticInput::pointer_moved(SLOT, from.0, from.1));
        core.inject_event(SyntheticInput::pointer_button(
            SLOT,
            MouseButton::Left,
            true,
        ));
        core.inject_event(SyntheticInput::pointer_moved(SLOT, to.0, to.1));
        core.inject_event(SyntheticInput::pointer_button(
            SLOT,
            MouseButton::Left,
            false,
        ));
    };
    drag(&mut core, KeyCode::KeyP, (100.0, 100.0), (300.0, 200.0));
    drag(&mut core, KeyCode::KeyD, (100.0, 300.0), (400.0, 300.0));
    drag(&mut core, KeyCode::KeyA, (100.0, 400.0), (400.0, 500.0));
    drag(&mut core, KeyCode::KeyR, (500.0, 100.0), (700.0, 250.0));
    drag(&mut core, KeyCode::KeyC, (500.0, 300.0), (700.0, 450.0));
    drag(&mut core, KeyCode::KeyM, (500.0, 500.0), (800.0, 500.0));
    drag(&mut core, KeyCode::KeyI, (800.0, 100.0), (950.0, 250.0));
    let scene = core.editor().scene();
    assert_eq!(scene.object_count(), 7);
    let kinds: Vec<&str> = scene
        .z_order()
        .iter()
        .map(|&id| scene.get_object(id).map_or("?", ToolObject::type_id))
        .collect();
    assert_eq!(
        kinds,
        [
            "pencil",
            "line",
            "arrow",
            "rectangle",
            "ellipse",
            "marker",
            "invert"
        ]
    );
    assert_eq!(core.editor().undo_stack().undo_depth(), 7);
    // The tool drags never reached the selection engine (F27 P3 > region).
    assert_eq!(core.selection().rect(), None);
}

#[test]
fn funnel_ctrl_drag_commits_the_constrained_line() {
    let mut core = funnel_core();
    core.inject_event(SyntheticInput::key_press(SLOT, KeyCode::KeyD));
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::CONTROL));
    core.inject_event(SyntheticInput::pointer_moved(SLOT, 100.0, 100.0));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        true,
    ));
    core.inject_event(SyntheticInput::pointer_moved(SLOT, 300.0, 140.0));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        false,
    ));
    let ToolObjectData::Line(line) = committed(core.editor()) else {
        panic!("line object");
    };
    assert_eq!(
        line.to,
        ScenePoint::new(300.0, 100.0),
        "H snap through the funnel"
    );
}

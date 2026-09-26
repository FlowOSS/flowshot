//! The todo-25 wiring table (plan acceptance: mutation-sequence round-trip,
//! move = exactly ONE undo unit, `undo_limit` eviction, z-order + undo
//! interleaving, delete + effects round-trip, Amendment #3 shortcut
//! collision validation).
//!
//! Headless at the [`EditorState`] surface (the production funnel path is
//! pinned by [`super::tests`] and the routing table by [`super::routing`]).
//! The collision table's selection-engine rows are DATA from the F12 map;
//! their live bindings are pinned by `selection/keys.rs` and
//! `selection/tests.rs` (Ctrl+A/C/Q, arrows) - this table validates the
//! CROSS-SURFACE uniqueness the funnel's pass-through order relies on.

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use std::time::Instant;

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{
    ArrowObject, Color as SceneColor, CounterObject, EllipseObject, InvertObject, LineObject,
    MarkerObject, PencilPath, Point as ScenePoint, Rect as SceneRect, RectObject, SceneData,
    TextObject, ToolObject, ToolObjectData,
};
use winit::event::MouseButton;
use winit::keyboard::{KeyCode, ModifiersState};

use crate::editor::effect::{Bake, EffectKind, PixelEffect};
use crate::editor::mutate::translated;
use crate::editor::pixelate::BakeRegion;
use crate::editor::*;

const RED: SceneColor = SceneColor::new(255, 0, 0, 255);
const BLUE: SceneColor = SceneColor::new(0, 0, 255, 255);
const NONE: ModifiersState = ModifiersState::empty();

fn editor() -> EditorState {
    EditorState::new(EditorTools::default(), ToolRegistry::new())
}

fn editor_with_undo_limit(limit: u32) -> EditorState {
    let mut config = EditorTools::default();
    config.editor.undo_limit = limit;
    EditorState::new(config, ToolRegistry::new())
}

fn env() -> EditorEnv {
    EditorEnv {
        selection: None,
        modifiers: NONE,
        now: Instant::now(),
        picker_visible: false,
        mouse: None,
    }
}

fn ctrl_env() -> EditorEnv {
    EditorEnv {
        modifiers: ModifiersState::CONTROL,
        ..env()
    }
}

fn at(x: f64, y: f64) -> LogicalPoint {
    LogicalPoint::from_raw(x, y)
}

fn rect_object(x: f32, y: f32, w: f32, h: f32) -> Box<dyn ToolObject> {
    Box::new(RectObject::new(SceneRect::new(x, y, w, h), RED, 2.0, false))
}

fn counter_object(x: f32, y: f32) -> Box<dyn ToolObject> {
    Box::new(CounterObject::new(ScenePoint::new(x, y), 12.0, RED, 0))
}

fn bounds(ed: &EditorState, id: usize) -> SceneRect {
    ed.scene()
        .get_object(id)
        .map(ToolObject::bounding_rect)
        .unwrap()
}

fn drag(ed: &mut EditorState, from: (f64, f64), motions: &[(f64, f64)]) {
    ed.pointer_press(&env(), MouseButton::Left, at(from.0, from.1));
    for motion in motions {
        ed.pointer_move(&env(), at(motion.0, motion.1));
    }
    ed.pointer_release(
        &env(),
        MouseButton::Left,
        at(motions[motions.len() - 1].0, motions[motions.len() - 1].1),
    );
}

fn effect(rect: LogicalRect) -> PixelEffect {
    PixelEffect::new(
        EffectKind::Pixelate,
        Bake {
            rect,
            region: BakeRegion {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            },
            pixels: vec![7u8; 16],
        },
    )
}

// ---------------------------------------------------------------------------
// A. Object move = ONE undo unit (F27 backup-at-first-move, push-at-release)
// ---------------------------------------------------------------------------

#[test]
fn move_drag_is_exactly_one_undo_unit() {
    let mut ed = editor();
    ed.commit_object(rect_object(100.0, 100.0, 40.0, 40.0));
    assert_eq!(ed.undo_stack().undo_depth(), 1);
    let before = bounds(&ed, 0);
    // Press, THREE motions, release: one journal entry, not per-motion.
    drag(
        &mut ed,
        (120.0, 120.0),
        &[(130.0, 125.0), (140.0, 130.0), (150.0, 140.0)],
    );
    assert_eq!(
        ed.undo_stack().undo_depth(),
        2,
        "commit + ONE move unit for 3 motions"
    );
    let after = bounds(&ed, 0);
    assert_eq!(after.x, before.x + 30.0);
    assert_eq!(after.y, before.y + 20.0);
    assert_eq!(ed.selected_object(), Some(0), "selection survives the drag");
    // One undo restores the PRE-MOVE state; redo re-applies the final one.
    assert!(ed.undo());
    assert_eq!(bounds(&ed, 0), before);
    assert!(ed.redo());
    assert_eq!(bounds(&ed, 0), after);
}

#[test]
fn click_without_motion_records_no_move_unit() {
    let mut ed = editor();
    ed.commit_object(rect_object(100.0, 100.0, 40.0, 40.0));
    ed.pointer_press(&env(), MouseButton::Left, at(120.0, 120.0));
    ed.pointer_release(&env(), MouseButton::Left, at(120.0, 120.0));
    assert_eq!(ed.undo_stack().undo_depth(), 1, "click only selected");
    assert_eq!(ed.selected_object(), Some(0));
    assert_eq!(bounds(&ed, 0).x, 100.0);
}

#[test]
fn zero_delta_motions_record_nothing() {
    let mut ed = editor();
    ed.commit_object(rect_object(100.0, 100.0, 40.0, 40.0));
    drag(&mut ed, (120.0, 120.0), &[(120.0, 120.0), (120.0, 120.0)]);
    assert_eq!(ed.undo_stack().undo_depth(), 1);
    assert_eq!(bounds(&ed, 0).x, 100.0);
}

#[test]
fn move_preserves_z_order_ids_and_counter_numbers() {
    let mut ed = editor();
    ed.commit_object(rect_object(0.0, 0.0, 20.0, 20.0));
    ed.commit_object(counter_object(100.0, 100.0));
    ed.commit_object(counter_object(300.0, 300.0));
    assert_eq!(ed.scene().counter_counts(), vec![1, 2]);
    let z_before = ed.scene().z_order().to_vec();
    // Drag counter #1 (id 1); the data roundtrip must not renumber or
    // reorder (the remove+re-add path WOULD - this pins the roundtrip).
    drag(&mut ed, (100.0, 100.0), &[(150.0, 120.0)]);
    assert_eq!(ed.scene().z_order(), z_before.as_slice());
    assert_eq!(ed.scene().counter_counts(), vec![1, 2]);
    assert_eq!(bounds(&ed, 1).x, 100.0 - 12.0 + 50.0);
    assert_eq!(bounds(&ed, 0).x, 0.0, "untouched objects stay put");
    assert_eq!(bounds(&ed, 2).x, 300.0 - 12.0);
}

#[test]
fn drag_owns_motions_and_only_the_left_release_ends_it() {
    let mut ed = editor();
    ed.commit_object(rect_object(100.0, 100.0, 40.0, 40.0));
    ed.pointer_press(&env(), MouseButton::Left, at(120.0, 120.0));
    assert!(ed.pointer_move(&env(), at(130.0, 130.0)).consumed);
    // A right release passes through (the editor opened no session for it)
    // and leaves the drag armed.
    assert!(
        !ed.pointer_release(&env(), MouseButton::Right, at(130.0, 130.0))
            .consumed
    );
    assert!(ed.pointer_move(&env(), at(140.0, 140.0)).consumed);
    ed.pointer_release(&env(), MouseButton::Left, at(140.0, 140.0));
    assert_eq!(ed.undo_stack().undo_depth(), 2);
    assert_eq!(bounds(&ed, 0).x, 120.0);
}

#[test]
fn undo_mid_drag_drops_the_drag_without_a_unit() {
    let mut ed = editor();
    ed.commit_object(rect_object(100.0, 100.0, 40.0, 40.0));
    ed.pointer_press(&env(), MouseButton::Left, at(120.0, 120.0));
    ed.pointer_move(&env(), at(160.0, 160.0));
    assert_eq!(bounds(&ed, 0).x, 140.0, "live motion applied");
    assert!(
        ed.key_press(&ctrl_env(), KeyCode::KeyZ, false, None)
            .consumed
    );
    assert_eq!(ed.scene().object_count(), 0, "undo stepped over the commit");
    // The release finds no armed drag and passes to the selection engine.
    assert!(
        !ed.pointer_release(&env(), MouseButton::Left, at(160.0, 160.0))
            .consumed
    );
    assert_eq!(ed.undo_stack().undo_depth(), 0);
    assert!(ed.redo());
    assert_eq!(bounds(&ed, 0).x, 100.0, "redo restores the COMMITTED place");
}

#[test]
fn deselect_mid_drag_rolls_the_live_motions_back() {
    let mut ed = editor();
    ed.commit_object(rect_object(100.0, 100.0, 40.0, 40.0));
    ed.pointer_press(&env(), MouseButton::Left, at(120.0, 120.0));
    ed.pointer_move(&env(), at(180.0, 120.0));
    assert_eq!(bounds(&ed, 0).x, 160.0);
    ed.deselect_object();
    assert_eq!(
        bounds(&ed, 0).x,
        100.0,
        "cancel restores the pre-move scene"
    );
    assert_eq!(ed.undo_stack().undo_depth(), 1, "no unit was recorded");
    assert!(
        !ed.pointer_release(&env(), MouseButton::Left, at(180.0, 120.0))
            .consumed
    );
}

// ---------------------------------------------------------------------------
// B. Z-order ops as undo units + interleaving
// ---------------------------------------------------------------------------

#[test]
fn raise_and_lower_are_single_undo_units() {
    let mut ed = editor();
    for x in [0.0, 100.0, 200.0] {
        ed.commit_object(rect_object(x, x, 20.0, 20.0));
    }
    assert_eq!(ed.scene().z_order(), &[0, 1, 2]);
    ed.select_layer(0);
    assert!(ed.raise_selected());
    assert_eq!(ed.scene().z_order(), &[1, 0, 2]);
    assert_eq!(ed.undo_stack().undo_depth(), 4);
    assert!(ed.undo());
    assert_eq!(ed.scene().z_order(), &[0, 1, 2]);
    assert!(ed.redo());
    assert_eq!(ed.scene().z_order(), &[1, 0, 2]);
    // Edges are silent no-ops recording NOTHING.
    let depth = ed.undo_stack().undo_depth();
    ed.select_layer(0); // id 0 sits at paint position 1 in z [1,0,2]
    assert!(ed.lower_selected());
    assert_eq!(ed.scene().z_order(), &[0, 1, 2]);
    assert!(!ed.lower_selected(), "bottom-most");
    ed.select_layer(2);
    assert!(!ed.raise_selected(), "top-most");
    assert_eq!(
        ed.undo_stack().undo_depth(),
        depth + 1,
        "only the real move recorded"
    );
}

#[test]
fn z_ops_without_selection_do_nothing() {
    let mut ed = editor();
    ed.commit_object(rect_object(0.0, 0.0, 20.0, 20.0));
    let depth = ed.undo_stack().undo_depth();
    assert!(!ed.raise_selected());
    assert!(!ed.lower_selected());
    assert!(!ed.raise_selected_to_top());
    assert!(!ed.lower_selected_to_bottom());
    assert_eq!(ed.undo_stack().undo_depth(), depth);
}

#[test]
fn to_top_and_to_bottom_jump_in_one_unit() {
    let mut ed = editor();
    for x in [0.0, 100.0, 200.0, 300.0] {
        ed.commit_object(rect_object(x, 0.0, 20.0, 20.0));
    }
    ed.select_layer(1);
    assert!(ed.raise_selected_to_top());
    assert_eq!(ed.scene().z_order(), &[0, 2, 3, 1]);
    assert!(ed.undo());
    assert_eq!(ed.scene().z_order(), &[0, 1, 2, 3]);
    ed.select_layer(2);
    assert!(ed.lower_selected_to_bottom());
    assert_eq!(ed.scene().z_order(), &[2, 0, 1, 3]);
    assert_eq!(
        ed.undo_stack().undo_depth(),
        5,
        "4 commits + 1 jump (the undo stepped back)"
    );
}

#[test]
fn z_order_interleaves_with_draw_and_delete() {
    let mut ed = editor();
    ed.commit_object(rect_object(0.0, 0.0, 20.0, 20.0)); // unit 1
    ed.commit_object(rect_object(100.0, 0.0, 20.0, 20.0)); // unit 2
    ed.select_layer(0);
    assert!(ed.raise_selected()); // unit 3: z [1,0]
    assert_eq!(ed.scene().z_order(), &[1, 0]);
    ed.select_layer(0); // id 0 is now top; delete it
    assert!(ed.delete_selected()); // unit 4
    assert_eq!(ed.scene().object_count(), 1);
    assert_eq!(ed.scene().z_order(), &[0]);
    // Peel back exactly one op per undo.
    assert!(ed.undo()); // -> after unit 3
    assert_eq!(ed.scene().object_count(), 2);
    assert_eq!(ed.scene().z_order(), &[1, 0]);
    assert!(ed.undo()); // -> after unit 2
    assert_eq!(ed.scene().z_order(), &[0, 1]);
    assert!(ed.undo()); // -> after unit 1
    assert_eq!(ed.scene().object_count(), 1);
    assert!(ed.undo()); // -> empty
    assert_eq!(ed.scene().object_count(), 0);
    assert!(!ed.undo());
}

#[test]
fn move_layer_reorders_as_one_unit() {
    let mut ed = editor();
    for x in [0.0, 100.0, 200.0, 300.0] {
        ed.commit_object(rect_object(x, 0.0, 20.0, 20.0));
    }
    let depth = ed.undo_stack().undo_depth();
    assert!(ed.move_layer(0, 2));
    assert_eq!(ed.scene().z_order(), &[1, 2, 0, 3]);
    assert_eq!(ed.undo_stack().undo_depth(), depth + 1);
    assert!(ed.undo());
    assert_eq!(ed.scene().z_order(), &[0, 1, 2, 3]);
    assert!(ed.move_layer(3, 1));
    assert_eq!(ed.scene().z_order(), &[0, 3, 1, 2]);
    // Identity and out-of-range reorders record nothing.
    let depth = ed.undo_stack().undo_depth();
    assert!(!ed.move_layer(1, 1));
    assert!(!ed.move_layer(9, 0));
    assert!(!ed.move_layer(0, 9));
    assert_eq!(ed.undo_stack().undo_depth(), depth);
}

#[test]
fn layers_list_bottom_to_top_and_select_by_click() {
    let mut ed = editor();
    ed.commit_object(rect_object(0.0, 0.0, 20.0, 20.0));
    ed.commit_object(counter_object(100.0, 100.0));
    ed.commit_object(Box::new(TextObject::new(
        ScenePoint::new(200.0, 200.0),
        "x".to_owned(),
        16.0,
        RED,
    )));
    assert_eq!(
        ed.layers(),
        vec![
            LayerEntry {
                z: 0,
                id: 0,
                kind: "rectangle"
            },
            LayerEntry {
                z: 1,
                id: 1,
                kind: "counter"
            },
            LayerEntry {
                z: 2,
                id: 2,
                kind: "text"
            },
        ]
    );
    assert!(ed.select_layer(2));
    assert_eq!(ed.selected_object(), Some(2));
    assert!(!ed.select_layer(3), "invalid id");
    // The list follows the paint order; a to-top jump on the top-most
    // layer is a no-op recording nothing.
    let depth = ed.undo_stack().undo_depth();
    assert!(!ed.raise_selected_to_top());
    assert_eq!(ed.undo_stack().undo_depth(), depth);
    ed.select_layer(0);
    assert!(ed.raise_selected_to_top());
    assert_eq!(
        ed.layers()
            .iter()
            .map(|layer| layer.kind)
            .collect::<Vec<_>>(),
        vec!["counter", "text", "rectangle"]
    );
}

// ---------------------------------------------------------------------------
// C. undo_limit wiring ([editor].undo_limit -> journal depth)
// ---------------------------------------------------------------------------

#[test]
fn undo_limit_evicts_the_oldest_units() {
    let mut ed = editor_with_undo_limit(3);
    assert_eq!(ed.undo_stack().limit(), 3);
    for x in 0..5u16 {
        ed.commit_object(rect_object(f32::from(x) * 100.0, 0.0, 20.0, 20.0));
    }
    assert_eq!(ed.undo_stack().undo_depth(), 3, "bounded by the config");
    assert!(ed.undo());
    assert!(ed.undo());
    assert!(ed.undo());
    assert_eq!(ed.scene().object_count(), 2, "oldest two are unreachable");
    assert!(!ed.undo(), "evicted history is gone");
}

#[test]
fn undo_limit_zero_disables_history() {
    let mut ed = editor_with_undo_limit(0);
    ed.commit_object(rect_object(0.0, 0.0, 20.0, 20.0));
    assert_eq!(ed.undo_stack().undo_depth(), 0);
    assert!(!ed.undo());
    assert_eq!(ed.scene().object_count(), 1, "mutations still apply");
}

#[test]
fn configure_applies_a_new_limit_live() {
    let mut ed = editor();
    assert_eq!(ed.undo_stack().limit(), 100, "config default");
    for x in 0..5u16 {
        ed.commit_object(rect_object(f32::from(x) * 100.0, 0.0, 20.0, 20.0));
    }
    let mut config = EditorTools::default();
    config.editor.undo_limit = 2;
    ed.configure(config);
    assert_eq!(ed.undo_stack().limit(), 2);
    assert_eq!(ed.undo_stack().undo_depth(), 2, "over-bound evicted");
}

// ---------------------------------------------------------------------------
// D. Delete + effects round-trip through the UNIFIED journal (todo 23)
// ---------------------------------------------------------------------------

#[test]
fn delete_and_undo_roundtrip_through_the_unified_journal() {
    let mut ed = editor();
    ed.commit_effect(effect(LogicalRect::from_raw(10.0, 10.0, 4.0, 4.0))); // unit 1
    ed.commit_object(rect_object(100.0, 100.0, 40.0, 40.0)); // unit 2
    ed.select_layer(0);
    assert!(ed.delete_selected()); // unit 3
    assert_eq!(ed.scene().object_count(), 0);
    assert_eq!(ed.pixel_effects().len(), 1, "the effect is untouched");
    assert!(ed.undo()); // -> unit 2 state
    assert_eq!(ed.scene().object_count(), 1);
    assert_eq!(ed.pixel_effects().len(), 1);
    assert!(ed.undo()); // -> unit 1 state
    assert_eq!(ed.scene().object_count(), 0);
    assert_eq!(ed.pixel_effects().len(), 1);
    assert!(ed.undo()); // -> empty
    assert_eq!(ed.pixel_effects().len(), 0);
    assert!(!ed.undo());
    assert!(ed.redo() && ed.redo() && ed.redo());
    assert_eq!(ed.scene().object_count(), 0);
    assert_eq!(ed.pixel_effects().len(), 1);
    assert_eq!(ed.pixel_effects()[0].id(), 0, "identity survives");
    assert!(!ed.redo());
}

#[test]
fn mutation_sequence_roundtrips_to_an_identical_scene() {
    // Plan acceptance: N ops -> N undos -> N redos == identical scene.
    let mut ed = editor();
    ed.commit_object(rect_object(0.0, 0.0, 20.0, 20.0)); // 1
    ed.commit_object(counter_object(100.0, 100.0)); // 2
    ed.commit_effect(effect(LogicalRect::from_raw(0.0, 0.0, 8.0, 8.0))); // 3
    drag(&mut ed, (100.0, 100.0), &[(140.0, 130.0)]); // 4: move
    ed.select_layer(0);
    assert!(ed.raise_selected()); // 5
    assert!(ed.mutate_object(0, |data| match data {
        ToolObjectData::Rectangle(mut object) => {
            object.color = BLUE;
            ToolObjectData::Rectangle(object)
        }
        other => other,
    })); // 6: property change
    ed.select_layer(1);
    assert!(ed.delete_selected()); // 7
    let units = ed.undo_stack().undo_depth();
    assert_eq!(units, 7, "every mutation is exactly one unit");
    let final_scene: SceneData = ed.scene().to_data();
    let final_effects = ed.pixel_effects().len();
    for _ in 0..units {
        assert!(ed.undo());
    }
    assert!(!ed.undo());
    assert_eq!(ed.scene().object_count(), 0);
    assert_eq!(ed.pixel_effects().len(), 0);
    for _ in 0..units {
        assert!(ed.redo());
    }
    assert!(!ed.redo());
    assert_eq!(ed.scene().to_data(), final_scene, "identical scene");
    assert_eq!(ed.pixel_effects().len(), final_effects);
}

#[test]
fn mutate_object_is_one_unit_and_validates_the_id() {
    let mut ed = editor();
    ed.commit_object(rect_object(0.0, 0.0, 20.0, 20.0));
    assert!(!ed.mutate_object(7, |data| data), "invalid id");
    assert_eq!(ed.undo_stack().undo_depth(), 1);
    assert!(ed.mutate_object(0, |data| match data {
        ToolObjectData::Rectangle(mut object) => {
            object.color = BLUE;
            ToolObjectData::Rectangle(object)
        }
        other => other,
    }));
    assert_eq!(ed.undo_stack().undo_depth(), 2);
    assert!(ed.undo());
    assert!(ed.redo());
}

#[test]
fn undo_and_redo_at_the_ends_are_silent_noops() {
    // Plan failure QA: undo at empty = no crash, no unit, log token
    // "undo at history start" (asserted live).
    let mut ed = editor();
    assert!(!ed.undo());
    assert!(!ed.redo());
    assert_eq!(ed.undo_stack().undo_depth(), 0);
}

// ---------------------------------------------------------------------------
// E. Z-order keys (unbound by default) + Amendment #3 collision table
// ---------------------------------------------------------------------------

#[test]
fn z_keys_ship_unbound_and_dispatch_when_rebound() {
    let mut ed = editor();
    ed.commit_object(rect_object(0.0, 0.0, 20.0, 20.0));
    ed.commit_object(rect_object(100.0, 0.0, 20.0, 20.0));
    ed.select_layer(0);
    // Unbound default: the key passes through to the funnel.
    assert!(!ed.key_press(&env(), KeyCode::KeyK, false, None).consumed);
    ed.shortcuts_mut()
        .rebind_z_order(Some(KeyCode::KeyK), Some(KeyCode::KeyJ));
    let raised = ed.key_press(&env(), KeyCode::KeyK, false, None);
    assert!(raised.consumed && raised.changed);
    assert_eq!(ed.scene().z_order(), &[1, 0]);
    // Auto-repeat never stacks journal entries.
    let depth = ed.undo_stack().undo_depth();
    assert!(!ed.key_press(&env(), KeyCode::KeyK, true, None).consumed);
    assert_eq!(ed.undo_stack().undo_depth(), depth);
    assert!(ed.key_press(&env(), KeyCode::KeyJ, false, None).consumed);
    assert_eq!(ed.scene().z_order(), &[0, 1]);
    // A bound z-key without selection is an eaten no-op.
    ed.deselect_object();
    let idle = ed.key_press(&env(), KeyCode::KeyK, false, None);
    assert!(idle.consumed && !idle.changed);
    // Unbind restores pass-through.
    ed.shortcuts_mut().rebind_z_order(None, None);
    assert!(!ed.key_press(&env(), KeyCode::KeyK, false, None).consumed);
}

#[derive(Debug)]
struct StubPencil;
impl Tool for StubPencil {
    fn kind(&self) -> ToolKind {
        ToolKind::Pencil
    }
}

#[test]
fn tool_keys_win_over_duplicate_z_bindings() {
    let mut registry = ToolRegistry::new();
    registry.register(ToolKind::Pencil, || Box::new(StubPencil));
    let mut ed = EditorState::new(EditorTools::default(), registry);
    ed.shortcuts_mut().rebind_z_order(Some(KeyCode::KeyP), None);
    // 'p' is the pencil key; the tool binding is checked first (documented
    // duplicate-binding resolution until todo 36 validates config).
    assert!(ed.key_press(&env(), KeyCode::KeyP, false, None).consumed);
    assert_eq!(ed.active_tool(), Some(ToolKind::Pencil));
}

/// The F12 default map as data: every simultaneously-live binding across
/// the editor and the selection engine. Amendment #3 (the Flameshot
/// Return=accept+upload collision class): no two actions share a
/// (modifier, key) pair.
#[test]
fn shortcut_collision_table_has_no_duplicate_bindings() {
    let mut table: Vec<(&str, ModifiersState, KeyCode)> = Vec::new();
    // Editor tool activation keys - asserted AGAINST the live defaults.
    let shortcuts = ToolShortcuts::default();
    for kind in ToolKind::ALL {
        if let Some(key) = kind.default_key() {
            assert_eq!(shortcuts.key_for_tool(kind), Some(key));
            table.push((kind.id(), NONE, key));
        }
    }
    // Digit size adjusters (F12: digits = tool size).
    for digit in 0..10 {
        table.push(("tool-size", NONE, digit_key(digit)));
    }
    // Editor actions (live-default cross-check for undo/redo).
    assert!(shortcuts.is_undo(KeyCode::KeyZ));
    assert!(shortcuts.is_redo(KeyCode::KeyZ));
    table.push(("undo", ModifiersState::CONTROL, KeyCode::KeyZ));
    table.push((
        "redo",
        ModifiersState::CONTROL | ModifiersState::SHIFT,
        KeyCode::KeyZ,
    ));
    table.push(("delete-object", NONE, KeyCode::Delete));
    table.push(("edit-commit", ModifiersState::CONTROL, KeyCode::Enter));
    // Selection engine (F12 rows pinned live by selection/keys.rs tests).
    table.push(("accept", NONE, KeyCode::Enter));
    table.push(("cancel", NONE, KeyCode::Escape));
    table.push(("select-all", ModifiersState::CONTROL, KeyCode::KeyA));
    table.push(("copy", ModifiersState::CONTROL, KeyCode::KeyC));
    table.push(("exit", ModifiersState::CONTROL, KeyCode::KeyQ));
    for arrow in [
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
    ] {
        table.push(("nudge", NONE, arrow));
        table.push(("resize", ModifiersState::SHIFT, arrow));
        table.push((
            "sym-resize",
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            arrow,
        ));
    }
    // Z-order ships UNBOUND (plan todo 25: panel-driven) - no table rows,
    // and the live default must answer None for every table key.
    for (_, _, key) in &table {
        assert_eq!(shortcuts.z_for_key(*key), None, "z-order unbound");
    }
    // The collision rule.
    for (index, (action, mods, key)) in table.iter().enumerate() {
        for (other, other_mods, other_key) in &table[index + 1..] {
            assert!(
                mods != other_mods || key != other_key,
                "collision: {action} vs {other} on {key:?}"
            );
        }
    }
}

fn digit_key(digit: u32) -> KeyCode {
    match digit {
        0 => KeyCode::Digit0,
        1 => KeyCode::Digit1,
        2 => KeyCode::Digit2,
        3 => KeyCode::Digit3,
        4 => KeyCode::Digit4,
        5 => KeyCode::Digit5,
        6 => KeyCode::Digit6,
        7 => KeyCode::Digit7,
        8 => KeyCode::Digit8,
        _ => KeyCode::Digit9,
    }
}

// ---------------------------------------------------------------------------
// F. The translation pure function (every scene variant)
// ---------------------------------------------------------------------------

#[test]
fn translated_shifts_every_variant() {
    let (dx, dy) = (5.0_f32, -3.0_f32);
    let point = |x: f32, y: f32| ScenePoint::new(x, y);
    let rect = SceneRect::new(10.0, 20.0, 30.0, 40.0);
    let cases = vec![
        (
            ToolObjectData::Rectangle(RectObject::new(rect, RED, 2.0, false)),
            ToolObjectData::Rectangle(RectObject::new(
                SceneRect::new(15.0, 17.0, 30.0, 40.0),
                RED,
                2.0,
                false,
            )),
        ),
        (
            ToolObjectData::Ellipse(EllipseObject::new(rect, RED, 2.0, true)),
            ToolObjectData::Ellipse(EllipseObject::new(
                SceneRect::new(15.0, 17.0, 30.0, 40.0),
                RED,
                2.0,
                true,
            )),
        ),
        (
            ToolObjectData::Arrow(ArrowObject::new(point(1.0, 2.0), point(3.0, 4.0), RED, 2.0)),
            ToolObjectData::Arrow(ArrowObject::new(
                point(6.0, -1.0),
                point(8.0, 1.0),
                RED,
                2.0,
            )),
        ),
        (
            ToolObjectData::Text(TextObject::new(point(1.0, 2.0), "hi".to_owned(), 16.0, RED)),
            ToolObjectData::Text(TextObject::new(
                point(6.0, -1.0),
                "hi".to_owned(),
                16.0,
                RED,
            )),
        ),
        (
            ToolObjectData::Counter(CounterObject::new(point(1.0, 2.0), 12.0, RED, 3)),
            ToolObjectData::Counter(CounterObject::new(point(6.0, -1.0), 12.0, RED, 3)),
        ),
        (
            ToolObjectData::Pencil(PencilPath::new(
                vec![point(0.0, 0.0), point(10.0, 10.0)],
                RED,
                2.0,
            )),
            ToolObjectData::Pencil(PencilPath::new(
                vec![point(5.0, -3.0), point(15.0, 7.0)],
                RED,
                2.0,
            )),
        ),
        (
            ToolObjectData::Line(LineObject::new(point(1.0, 2.0), point(3.0, 4.0), RED, 2.0)),
            ToolObjectData::Line(LineObject::new(point(6.0, -1.0), point(8.0, 1.0), RED, 2.0)),
        ),
        (
            ToolObjectData::Marker(MarkerObject::new(
                point(1.0, 2.0),
                point(3.0, 4.0),
                RED,
                8.0,
            )),
            ToolObjectData::Marker(MarkerObject::new(
                point(6.0, -1.0),
                point(8.0, 1.0),
                RED,
                8.0,
            )),
        ),
        (
            ToolObjectData::Invert(InvertObject::new(rect)),
            ToolObjectData::Invert(InvertObject::new(SceneRect::new(15.0, 17.0, 30.0, 40.0))),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(translated(input, dx, dy), expected);
    }
}

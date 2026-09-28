//! The destructive-region-tool suite at the editor surface: the
//! draw lifecycle through the real [`EditorState`], the pixel-overlay
//! commit channel, region clamping (frame + selection), the no-op failure
//! paths, the unified undo (original pixels restored losslessly because
//! the pristine frame is never modified), and the paint seams (black
//! pending-region preview, image quads below the scene).

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::time::Instant;

use flowshot_core::config::Config;
use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo, PhysicalSize, Transform};
use flowshot_core::scene::{Color as SceneColor, RectObject};
use winit::event::MouseButton;
use winit::keyboard::ModifiersState;

use crate::editor::*;
use crate::render::{Command, DisplayList, TextureId};

fn editor() -> EditorState {
    let mut registry = ToolRegistry::new();
    register_pixelate_tools(&mut registry);
    EditorState::new(EditorTools::from_config(&Config::default()), registry)
}

fn env() -> EditorEnv {
    EditorEnv {
        selection: None,
        modifiers: ModifiersState::empty(),
        now: Instant::now(),
        picker_visible: false,
        mouse: None,
    }
}

fn at(x: f64, y: f64) -> LogicalPoint {
    LogicalPoint::from_raw(x, y)
}

/// A 64x48 synthetic frozen frame at scale 1, origin (0,0): a smooth
/// gradient with a high-frequency ripple (visible blur/pixelate deltas).
fn test_frame() -> FramePixels {
    let (w, h) = (64u32, 48u32);
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h {
        for x in 0..w {
            let at = (y as usize * w as usize + x as usize) * 4;
            rgba[at] = (x * 4) as u8;
            rgba[at + 1] = (y * 5) as u8;
            rgba[at + 2] = if (x + y) % 2 == 0 { 200 } else { 40 };
            rgba[at + 3] = 255;
        }
    }
    FramePixels {
        rgba,
        width: w,
        height: h,
        scale: 1.0,
        origin: LogicalPoint::from_raw(0.0, 0.0),
    }
}

fn editor_with_frame() -> (EditorState, Vec<u8>) {
    let mut ed = editor();
    let frame = test_frame();
    let pristine = frame.rgba.clone();
    ed.install_frame(Some(frame));
    (ed, pristine)
}

fn stroke(ed: &mut EditorState, env: &EditorEnv, from: (f64, f64), to: (f64, f64)) {
    ed.pointer_press(env, MouseButton::Left, at(from.0, from.1));
    ed.pointer_move(env, at(to.0, to.1));
    ed.pointer_release(env, MouseButton::Left, at(to.0, to.1));
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

fn paint_commands(ed: &EditorState, selection: Option<LogicalRect>) -> Vec<Command> {
    let out = output();
    let mut list = DisplayList::new();
    let view = EditorView {
        mouse: Some(at(0.0, 0.0)),
        selection,
        modifiers: ModifiersState::empty(),
    };
    ed.paint_into(&mut list, &out, view);
    list.iter().cloned().collect()
}

// ---------------------------------------------------------------------------
// A. Commit channel: the effect layer, not the scene
// ---------------------------------------------------------------------------

#[test]
fn drag_commits_one_baked_effect_as_one_undo_unit() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    assert_eq!(ed.tool_size(), 2, "[tools.pixelate].size default");
    stroke(&mut ed, &env(), (8.0, 6.0), (40.0, 30.0));
    let effects = ed.pixel_effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].kind(), EffectKind::Pixelate);
    assert_eq!(
        effects[0].rect(),
        LogicalRect::from_raw(8.0, 6.0, 32.0, 24.0)
    );
    assert_eq!((effects[0].width(), effects[0].height()), (32, 24));
    assert_eq!(effects[0].pixels().len(), 32 * 24 * 4);
    assert_eq!(effects[0].texture_id(), effect_texture_id(0));
    assert_eq!(ed.undo_stack().undo_depth(), 1, "one drag = one unit");
    assert_eq!(ed.scene().object_count(), 0, "NOT a scene object");
}

#[test]
fn the_pristine_frame_is_never_modified_and_undo_restores_the_original() {
    // The plan's "never modify origScreenshot": the read side stays byte
    // pristine through commit AND undo; undo drops the overlay so the
    // original pixels show through losslessly (no snapshot can go stale).
    let (mut ed, pristine) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (8.0, 6.0), (40.0, 30.0));
    assert_eq!(ed.pixel_effects().len(), 1);
    assert_eq!(ed.frame().unwrap().rgba, pristine, "commit: frame pristine");
    assert!(ed.undo().0);
    assert!(ed.pixel_effects().is_empty(), "undo = remove the effect");
    assert_eq!(ed.frame().unwrap().rgba, pristine, "undo: frame pristine");
    assert!(ed.redo().0);
    assert_eq!(ed.pixel_effects().len(), 1, "redo re-applies the bake");
    assert_eq!(ed.pixel_effects()[0].texture_id(), effect_texture_id(0));
}

#[test]
fn effects_and_scene_objects_undo_in_interleaved_order() {
    let (mut ed, _) = editor_with_frame();
    ed.commit_object(Box::new(RectObject::new(
        flowshot_core::scene::Rect::new(1.0, 1.0, 4.0, 4.0),
        SceneColor::new(255, 0, 0, 255),
        2.0,
        false,
    )));
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (8.0, 6.0), (40.0, 30.0));
    assert_eq!(ed.undo_stack().undo_depth(), 2);
    assert!(ed.undo().0);
    assert!(ed.pixel_effects().is_empty(), "effect undone first");
    assert_eq!(ed.scene().object_count(), 1, "rect survives");
    assert!(ed.undo().0);
    assert_eq!(ed.scene().object_count(), 0, "rect undone second");
    assert!(ed.redo().0);
    assert_eq!(ed.scene().object_count(), 1);
    assert!(ed.redo().0);
    assert_eq!(ed.pixel_effects().len(), 1);
}

#[test]
fn effect_ids_never_reuse_so_retired_textures_stay_retired() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (4.0, 4.0), (20.0, 20.0));
    assert!(ed.undo().0);
    stroke(&mut ed, &env(), (4.0, 4.0), (24.0, 24.0));
    let effects = ed.pixel_effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(
        effects[0].id(),
        1,
        "monotonic ids: no stale-texture revival"
    );
    assert_eq!(effects[0].texture_id(), effect_texture_id(1));
}

#[test]
fn a_new_frame_starts_a_new_session_and_drops_stale_effects() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (4.0, 4.0), (20.0, 20.0));
    assert_eq!(ed.pixel_effects().len(), 1);
    ed.install_frame(Some(test_frame()));
    assert!(
        ed.pixel_effects().is_empty(),
        "old bakes reference old pixels"
    );
    assert_eq!(ed.undo_stack().undo_depth(), 0);
}

// ---------------------------------------------------------------------------
// B. Region clamping and the no-op failure paths
// ---------------------------------------------------------------------------

#[test]
fn regions_clamp_to_the_frame_bounds() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (-20.0, -10.0), (200.0, 100.0));
    let effects = ed.pixel_effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(
        effects[0].rect(),
        LogicalRect::from_raw(0.0, 0.0, 64.0, 48.0),
        "clamped to the 64x48 frame"
    );
    assert_eq!((effects[0].width(), effects[0].height()), (64, 48));
}

#[test]
fn regions_clamp_to_the_selection_bounds() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    let env = EditorEnv {
        selection: Some(LogicalRect::from_raw(10.0, 10.0, 30.0, 20.0)),
        ..env()
    };
    stroke(&mut ed, &env, (0.0, 0.0), (63.0, 47.0));
    let effects = ed.pixel_effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(
        effects[0].rect(),
        LogicalRect::from_raw(10.0, 10.0, 30.0, 20.0)
    );
}

#[test]
fn drags_outside_every_bound_commit_nothing() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    // Entirely outside the frame.
    stroke(&mut ed, &env(), (100.0, 100.0), (200.0, 200.0));
    // Entirely outside the selection.
    let env = EditorEnv {
        selection: Some(LogicalRect::from_raw(0.0, 0.0, 10.0, 10.0)),
        ..env()
    };
    stroke(&mut ed, &env, (40.0, 40.0), (60.0, 44.0));
    assert!(ed.pixel_effects().is_empty());
    assert_eq!(ed.undo_stack().undo_depth(), 0, "no unit for a no-op");
}

#[test]
fn zero_length_and_one_pixel_drags_are_noops() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    // Click without move (the shape tools' zero-length rule).
    ed.pointer_press(&env(), MouseButton::Left, at(10.0, 10.0));
    ed.pointer_release(&env(), MouseButton::Left, at(10.0, 10.0));
    // 1x1 region: the F27 grid collapses to zero (failure QA).
    stroke(&mut ed, &env(), (10.0, 10.0), (11.0, 11.0));
    assert!(ed.pixel_effects().is_empty());
    assert_eq!(ed.undo_stack().undo_depth(), 0);
}

#[test]
fn no_installed_frame_commits_nothing_and_does_not_panic() {
    let mut ed = editor();
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (4.0, 4.0), (40.0, 30.0));
    assert!(ed.pixel_effects().is_empty());
}

// ---------------------------------------------------------------------------
// C. The blur variant
// ---------------------------------------------------------------------------

#[test]
fn blur_tool_commits_a_blur_effect_that_really_smooths() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Blur);
    assert_eq!(ed.tool_size(), 2, "blur shares the pixelate size slot");
    stroke(&mut ed, &env(), (8.0, 8.0), (56.0, 40.0));
    let effects = ed.pixel_effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].kind(), EffectKind::Blur);
    // The frame's checkerboard blue channel has variance ~6400; the blur
    // variant must visibly destroy that high-frequency content.
    let blues: Vec<f64> = effects[0]
        .pixels()
        .iter()
        .skip(2)
        .step_by(4)
        .map(|c| f64::from(*c))
        .collect();
    let mean = blues.iter().sum::<f64>() / blues.len() as f64;
    let variance = blues.iter().map(|b| (b - mean) * (b - mean)).sum::<f64>() / blues.len() as f64;
    assert!(variance < 100.0, "blurred checkerboard variance {variance}");
}

#[test]
fn pixelate_and_blur_bakes_differ_and_honor_the_size_slot() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (8.0, 8.0), (56.0, 40.0));
    let small = ed.pixel_effects()[0].pixels().to_vec();
    ed.set_tool_size(6);
    assert!(ed.undo().0);
    stroke(&mut ed, &env(), (8.0, 8.0), (56.0, 40.0));
    let coarse = ed.pixel_effects()[0].pixels().to_vec();
    assert_ne!(small, coarse, "size drives the F27 grid coarseness");
    // Coarser blocks: bigger uniform runs along a row.
    let run_len = |pixels: &[u8]| -> usize {
        let first = &pixels[..4];
        pixels.chunks(4).take_while(|pixel| *pixel == first).count()
    };
    assert!(
        run_len(&coarse) > run_len(&small),
        "size 6 blocks are wider than size 2 blocks"
    );
}

// ---------------------------------------------------------------------------
// D. Paint seams: black pending preview, image quads below the scene
// ---------------------------------------------------------------------------

#[test]
fn pending_drag_paints_the_region_black() {
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    ed.pointer_press(&env(), MouseButton::Left, at(8.0, 6.0));
    ed.pointer_move(&env(), at(40.0, 30.0));
    let commands = paint_commands(&ed, None);
    let black = commands.iter().any(|command| {
        matches!(
            command,
            Command::Fill {
                shape: crate::render::Shape::Rect { .. },
                color,
            } if *color == crate::render::Color::from_rgba8(0, 0, 0, 255)
        )
    });
    assert!(black, "drawSearchArea parity: black pending region");
    ed.pointer_release(&env(), MouseButton::Left, at(40.0, 30.0));
}

#[test]
fn committed_effects_paint_as_image_quads_below_the_scene() {
    let (mut ed, _) = editor_with_frame();
    ed.commit_object(Box::new(RectObject::new(
        flowshot_core::scene::Rect::new(1.0, 1.0, 4.0, 4.0),
        SceneColor::new(255, 0, 0, 255),
        2.0,
        false,
    )));
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (8.0, 6.0), (40.0, 30.0));
    let commands = paint_commands(&ed, None);
    let image_at = commands.iter().position(|command| {
        matches!(
            command,
            Command::Image(image) if image.texture == effect_texture_id(0)
                && image.src.is_none()
        )
    });
    let scene_at = commands
        .iter()
        .position(|command| matches!(command, Command::Stroke { .. }));
    let image_at = image_at.expect("the effect quad paints");
    let scene_at = scene_at.expect("the rect paints");
    assert!(
        image_at < scene_at,
        "redactions bake UNDER the annotations (Flameshot pixmap parity)"
    );
    // The quad maps the logical rect into this output's local physical px.
    let Command::Image(image) = &commands[image_at] else {
        panic!("image command");
    };
    assert_eq!(image.dst.origin, crate::render::Point::new(8.0, 6.0));
    assert_eq!(image.dst.size.width, 32.0);
    assert_eq!(image.dst.size.height, 24.0);
    assert_eq!(
        image.texture,
        TextureId::new(1u64 << 40),
        "the documented effect texture-id scheme"
    );
}

#[test]
fn pixelate_bake_matches_the_algorithm_module_byte_for_byte() {
    // The tool path is a thin region-mapping wrapper over the algorithm:
    // same frame + same clamped region + same size = same bytes.
    let (mut ed, _) = editor_with_frame();
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (8.0, 6.0), (40.0, 30.0));
    let through_tool = ed.pixel_effects()[0].pixels().to_vec();
    let frame = test_frame();
    let direct = crate::editor::pixelate::bake_pixelate(
        &frame,
        crate::editor::pixelate::BakeRegion {
            x: 8,
            y: 6,
            w: 32,
            h: 24,
        },
        2,
    )
    .unwrap();
    assert_eq!(through_tool, direct);
}

#[test]
fn scaled_frames_map_logical_regions_to_physical_bakes() {
    // Scale 2: a 16x12 logical drag becomes a 32x24 physical bake, and the
    // painted quad covers the same logical rect on the scale-2 output.
    let mut ed = editor();
    ed.install_frame(Some(frame_with_scale2()));
    ed.activate_tool(ToolKind::Pixelate);
    stroke(&mut ed, &env(), (4.0, 3.0), (20.0, 15.0));
    let effects = ed.pixel_effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(
        effects[0].rect(),
        LogicalRect::from_raw(4.0, 3.0, 16.0, 12.0)
    );
    assert_eq!((effects[0].width(), effects[0].height()), (32, 24));
}

fn frame_with_scale2() -> FramePixels {
    let (w, h) = (128u32, 96u32);
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h {
        for x in 0..w {
            let at = (y as usize * w as usize + x as usize) * 4;
            rgba[at] = (x * 2) as u8;
            rgba[at + 1] = (y * 2) as u8;
            rgba[at + 2] = 128;
            rgba[at + 3] = 255;
        }
    }
    FramePixels {
        rgba,
        width: w,
        height: h,
        scale: 2.0,
        origin: LogicalPoint::from_raw(0.0, 0.0),
    }
}

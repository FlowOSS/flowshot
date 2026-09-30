//! The magnifier table: the acceptance formula (zoomed
//! pixel == source[cursor-8+i][cursor-8+j]), the four-border edge flip,
//! the corner on-screen guarantee, the hex readout format, the shape
//! variants, the toggle key, and the config projection.

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::time::Instant;

use flowshot_core::config::{EditorConfig, MagnifierShape};
use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo, PhysicalSize, Transform};
use flowshot_core::tokens::DesignTokens;
use winit::keyboard::{KeyCode, ModifiersState};

use super::super::effect::Bake;
use super::super::pixelate::BakeRegion;
use super::super::tool::FramePixels;
use super::paint::{grid_at, placement};
use super::*;
use crate::editor::{EditorEnv, EditorState, EditorTools, EffectKind, PixelEffect, ToolRegistry};
use crate::render::{Command, DisplayList, Shape};

fn source(x: u32, y: u32) -> [u8; 4] {
    [
        x.wrapping_mul(3).wrapping_add(5) as u8,
        y.wrapping_mul(5).wrapping_add(7) as u8,
        x.wrapping_add(y) as u8,
        255,
    ]
}

fn frame_with(w: u32, h: u32, scale: f64, origin: (f64, f64)) -> FramePixels {
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h {
        for x in 0..w {
            let at = (y as usize * w as usize + x as usize) * 4;
            rgba[at..at + 4].copy_from_slice(&source(x, y));
        }
    }
    FramePixels {
        rgba,
        width: w,
        height: h,
        scale,
        origin: LogicalPoint::from_raw(origin.0, origin.1),
    }
}

fn frame() -> FramePixels {
    frame_with(64, 48, 1.0, (0.0, 0.0))
}

fn at(x: f64, y: f64) -> LogicalPoint {
    LogicalPoint::from_raw(x, y)
}

fn env_with(modifiers: ModifiersState) -> EditorEnv {
    EditorEnv {
        selection: None,
        modifiers,
        now: Instant::now(),
        picker_visible: false,
        mouse: None,
    }
}

fn env() -> EditorEnv {
    env_with(ModifiersState::empty())
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

fn view_at(cursor: (f64, f64), surface: (u32, u32)) -> MagnifierView {
    MagnifierView {
        surface,
        cursor_local: cursor,
        cursor_global: at(cursor.0, cursor.1),
    }
}

fn editor_shaped(shape: MagnifierShape) -> EditorState {
    let tools = EditorTools {
        editor: EditorConfig {
            magnifier: true,
            magnifier_shape: shape,
            ..EditorConfig::default()
        },
        ..EditorTools::default()
    };
    let mut editor = EditorState::new(tools, ToolRegistry::new());
    editor.install_frame(Some(frame_with(640, 480, 1.0, (0.0, 0.0))));
    editor
}

// ---------------------------------------------------------------------------
// A. Sampling correctness (the plan's acceptance formula)
// ---------------------------------------------------------------------------

#[test]
fn sample_window_matches_source_around_cursor() {
    let frame = frame();
    // Cursor logical (30.5, 20.5) truncates to physical (30, 20): the
    // window top-left is (cursor - 8) = (22, 12).
    let sample = super::sample(Some(&frame), &[], at(30.5, 20.5)).unwrap();
    for j in 0..WINDOW_PX {
        for i in 0..WINDOW_PX {
            assert_eq!(
                sample.window_pixel(i, j),
                source((22 + i) as u32, (12 + j) as u32),
                "window[{i}][{j}]"
            );
        }
    }
    assert_eq!(sample.center, source(30, 20));
    assert_eq!(sample.arm_offset, (0, 0));
    // The zoomed buffer replicates every window pixel as a ZOOM x ZOOM
    // block: zoomed[j*10+r][i*10+c] == source[cursor-8+i][cursor-8+j].
    let zoomed = sample.zoomed();
    let edge = (WINDOW_PX * ZOOM) as usize;
    assert_eq!(zoomed.len(), edge * edge * 4);
    for j in 0..WINDOW_PX as usize {
        for i in 0..WINDOW_PX as usize {
            let expected = source((22 + i) as u32, (12 + j) as u32);
            for r in 0..ZOOM as usize {
                for c in 0..ZOOM as usize {
                    let start = ((j * 10 + r) * edge + (i * 10 + c)) * 4;
                    assert_eq!(&zoomed[start..start + 4], &expected[..]);
                }
            }
        }
    }
}

#[test]
fn sample_clamps_at_frame_edges_and_shifts_the_center() {
    let frame = frame();
    // Top-left: the window clamps to (0, 0), the arm offset records the
    // negative shift, and the center stays the cursor's true pixel.
    let sample = super::sample(Some(&frame), &[], at(2.5, 2.5)).unwrap();
    assert_eq!(sample.arm_offset, (-6, -6));
    assert_eq!(sample.center, source(2, 2));
    assert_eq!(sample.window_pixel(0, 0), source(0, 0));
    assert_eq!(sample.window_pixel(2, 2), source(2, 2));
    // Bottom-right: the window clamps to (width-17, height-17).
    let sample = super::sample(Some(&frame), &[], at(63.5, 47.5)).unwrap();
    assert_eq!(sample.arm_offset, (8, 8));
    assert_eq!(sample.center, source(63, 47));
    assert_eq!(sample.window_pixel(16, 16), source(63, 47));
    assert_eq!(sample.window_pixel(0, 0), source(47, 31));
}

#[test]
fn sample_honors_frame_scale_and_origin() {
    // Scale 2, origin (100, 50): logical (130.5, 70.5) -> frame-local
    // physical (61, 41) (the eyedropper's conversion, HiDPI support).
    let frame = frame_with(128, 96, 2.0, (100.0, 50.0));
    let sample = super::sample(Some(&frame), &[], at(130.5, 70.5)).unwrap();
    assert_eq!(sample.center, source(61, 41));
    assert_eq!(sample.window_pixel(0, 0), source(53, 33));
    assert_eq!(sample.arm_offset, (0, 0));
}

#[test]
fn sample_composites_the_pixel_effect_layer() {
    // The pixel-effect seam: a baked effect over the cursor redacts what the
    // magnifier shows (post-effect sampling, never the pristine frame).
    let mut frame = frame_with(64, 48, 1.0, (0.0, 0.0));
    for pixel in frame.rgba.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&[200, 30, 30, 255]);
    }
    let green = [40u8, 220, 90, 255];
    let effect = PixelEffect::new(
        EffectKind::Pixelate,
        Bake {
            rect: LogicalRect::from_raw(10.0, 10.0, 4.0, 4.0),
            region: BakeRegion {
                x: 10,
                y: 10,
                w: 4,
                h: 4,
            },
            pixels: green.repeat(16),
        },
    );
    let sample = super::sample(Some(&frame), &[effect], at(11.5, 11.5)).unwrap();
    assert_eq!(sample.center, green);
    // Window top-left is (3, 3): (8, 8) is the covered (11, 11) pixel,
    // (0, 0) stays the pristine frame.
    assert_eq!(sample.window_pixel(8, 8), green);
    assert_eq!(sample.window_pixel(0, 0), [200, 30, 30, 255]);
}

#[test]
fn sample_rejects_missing_tiny_and_outside_cursors() {
    let frame = frame();
    assert!(super::sample(None, &[], at(10.0, 10.0)).is_none());
    let tiny = frame_with(8, 8, 1.0, (0.0, 0.0));
    assert!(super::sample(Some(&tiny), &[], at(4.0, 4.0)).is_none());
    assert!(super::sample(Some(&frame), &[], at(-0.5, 5.0)).is_none());
    assert!(super::sample(Some(&frame), &[], at(5.0, -0.5)).is_none());
    assert!(super::sample(Some(&frame), &[], at(64.5, 5.0)).is_none());
    assert!(super::sample(Some(&frame), &[], at(5.0, 48.5)).is_none());
}

// ---------------------------------------------------------------------------
// B. Placement: the 16px offset, the four-edge flip, the corner guarantee
// ---------------------------------------------------------------------------

#[test]
fn placement_sits_offset_down_right_of_the_cursor() {
    let rect = placement((500.0, 400.0), (1920, 1080));
    // Center = cursor + 16 + 85; top-left = cursor + 16.
    assert_eq!(rect.origin.x, 516.0);
    assert_eq!(rect.origin.y, 416.0);
    assert_eq!(rect.size.width, RENDERED_PX as f32);
    assert_eq!(rect.size.height, RENDERED_PX as f32);
}

#[test]
fn placement_flips_at_all_four_borders() {
    let surface = (1920u32, 1080u32);
    // Left border: the default down-right side already fits - the widget
    // stays opposite the near edge, fully on-screen.
    let rect = placement((5.0, 540.0), surface);
    assert_eq!(rect.origin.x, 21.0);
    assert!(rect.origin.x > 5.0 && rect.right() <= 1920.0);
    // Right border: flips to the left of the cursor.
    let rect = placement((1915.0, 540.0), surface);
    assert!(rect.right() <= 1920.0, "right border: {rect:?}");
    assert!(rect.right() <= 1915.0, "flipped to the opposite side");
    assert_eq!(rect.origin.x, 1915.0 - 16.0 - 170.0);
    // Top border: stays below.
    let rect = placement((960.0, 5.0), surface);
    assert_eq!(rect.origin.y, 21.0);
    assert!(rect.origin.y > 5.0 && rect.bottom() <= 1080.0);
    // Bottom border: flips above.
    let rect = placement((960.0, 1075.0), surface);
    assert!(rect.bottom() <= 1080.0, "bottom border: {rect:?}");
    assert!(rect.bottom() <= 1075.0, "flipped to the opposite side");
    assert_eq!(rect.origin.y, 1075.0 - 16.0 - 170.0);
}

#[test]
fn placement_at_exact_corners_stays_fully_on_screen() {
    let surface = (1920u32, 1080u32);
    for corner in [(0.0, 0.0), (1919.0, 0.0), (0.0, 1079.0), (1919.0, 1079.0)] {
        let rect = placement(corner, surface);
        assert!(rect.origin.x >= 0.0 && rect.origin.y >= 0.0, "{corner:?}");
        assert!(
            rect.right() <= 1920.0 && rect.bottom() <= 1080.0,
            "{corner:?}"
        );
    }
}

#[test]
fn placement_degenerate_surface_clamps_to_origin() {
    // A surface smaller than the widget cannot contain it; the clamp keeps
    // the top-left at (0, 0) instead of inverting the rect.
    let rect = placement((10.0, 10.0), (100, 100));
    assert_eq!(rect.origin.x, 0.0);
    assert_eq!(rect.origin.y, 0.0);
}

// ---------------------------------------------------------------------------
// C. Readout formatting (wayshot --color equivalence) + the grid gate
// ---------------------------------------------------------------------------

fn sample_with_center(center: [u8; 4]) -> MagnifierSample {
    MagnifierSample {
        pixels: vec![0u8; (WINDOW_PX * WINDOW_PX * 4) as usize],
        center,
        arm_offset: (0, 0),
    }
}

#[test]
fn readout_formats_hex_and_rgb() {
    assert_eq!(
        sample_with_center([255, 128, 0, 255]).readout_text(),
        "#FF8000 255,128,0"
    );
    assert_eq!(
        sample_with_center([0, 0, 0, 255]).readout_text(),
        "#000000 0,0,0"
    );
    assert_eq!(
        sample_with_center([10, 171, 203, 255]).readout_text(),
        "#0AABCB 10,171,203"
    );
    assert_eq!(
        sample_with_center([255, 128, 0, 255]).center_hex(),
        "#FF8000"
    );
}

#[test]
fn grid_gates_at_zoom_eight() {
    assert!(!grid_at(GRID_MIN_ZOOM - 1));
    assert!(grid_at(GRID_MIN_ZOOM));
    assert!(grid_at(ZOOM));
}

#[test]
fn constants_match_the_f27_spec() {
    assert_eq!(MAG_PIXELS, 8);
    assert_eq!(WINDOW_PX, 17);
    assert_eq!(ZOOM, 10);
    assert_eq!(RENDERED_PX, (WINDOW_PX * ZOOM) as f64);
    assert_eq!(CURSOR_OFFSET, 16.0);
    assert_eq!(ARM_ALPHA, 130);
    let id = magnifier_texture_id().raw();
    assert_eq!(id, 1 << 14);
    // Disjoint from the atlas (1), cursor (1<<15), backdrop (1<<16+i), and
    // effect (1<<40+i) bases.
    assert_ne!(id, 1);
    assert!(id < (1 << 15) && (1 << 15) < (1 << 16) && id < (1 << 40));
}

// ---------------------------------------------------------------------------
// D. Paint: the shape variants' command streams
// ---------------------------------------------------------------------------

fn paint(shape: MagnifierShape) -> (DisplayList, MagnifierTexture) {
    let editor = editor_shaped(shape);
    let mut list = DisplayList::new();
    let texture = editor
        .paint_magnifier(
            &mut list,
            &output(),
            view_at((300.0, 200.0), (1920, 1080)),
            &DesignTokens::default(),
        )
        .expect("frame installed and cursor inside");
    (list, texture)
}

fn count(list: &DisplayList, predicate: impl Fn(&Command) -> bool) -> usize {
    list.iter().filter(|command| predicate(command)).count()
}

#[test]
fn paint_square_variant_matches_the_f27_layout() {
    let (list, texture) = paint(MagnifierShape::Square);
    assert_eq!(texture.id, magnifier_texture_id());
    assert_eq!(texture.width, 170);
    assert_eq!(texture.height, 170);
    assert_eq!(texture.pixels.len(), 170 * 170 * 4);
    let widget = crate::render::Rect::from_parts(316.0, 216.0, 170.0, 170.0);
    // The border rect: one px wider than the widget, painted UNDER it.
    let Command::Fill { shape: border, .. } = list.iter().next().unwrap() else {
        panic!("square variant opens with the border fill");
    };
    let Shape::Rect { rect, radius } = border else {
        panic!("border is a rect");
    };
    assert_eq!(*radius, 0.0);
    assert_eq!(rect.origin.x, widget.origin.x - 1.0);
    assert_eq!(rect.size.width, widget.size.width + 2.0);
    // Exactly one image quad: the zoom texture into the widget rect.
    assert_eq!(
        count(
            &list,
            |command| matches!(command, Command::Image(image) if image.texture == magnifier_texture_id() && image.dst == widget && image.src.is_none())
        ),
        1
    );
    // 16 interior grid lines per axis, 1px wide.
    assert_eq!(
        count(
            &list,
            |command| matches!(command, Command::Fill { shape: Shape::Rect { rect, .. }, .. } if rect.size.width == 1.0 || rect.size.height == 1.0)
        ),
        32
    );
    // Four crosshair arms (zero offset: all non-degenerate), at the exact
    // F27 rects: one zoomed pixel wide, centered on the cursor's column/
    // row, reaching 8 cells (80px) to each window edge.
    let arm = crate::render::Color::from_hex_token("#6366F1")
        .unwrap()
        .with_alpha8(ARM_ALPHA);
    let arms: Vec<_> = list
        .iter()
        .filter_map(|command| match command {
            Command::Fill {
                shape: Shape::Rect { rect, .. },
                color,
            } if *color == arm => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(arms.len(), 4);
    let expected = [
        // top, right, bottom, left (widget (316,216), center (401,301))
        (396.0, 216.0, 10.0, 80.0),
        (406.0, 296.0, 80.0, 10.0),
        (396.0, 306.0, 10.0, 80.0),
        (316.0, 296.0, 80.0, 10.0),
    ];
    for (rect, want) in arms.iter().zip(expected) {
        assert_eq!(rect.origin.x, want.0);
        assert_eq!(rect.origin.y, want.1);
        assert_eq!(rect.size.width, want.2);
        assert_eq!(rect.size.height, want.3);
    }
    // The readout: the fixture pixel (300, 200) = #89EFF4.
    assert_eq!(
        count(
            &list,
            |command| matches!(command, Command::Text(text) if text.text == "#89EFF4 137,239,244")
        ),
        1
    );
    // The square variant never clips.
    assert_eq!(count(&list, |c| matches!(c, Command::PushClip(_))), 0);
}

#[test]
fn paint_arms_shift_and_shrink_under_the_edge_clamp() {
    // Cursor at frame physical (2, 2): the window clamps to (0, 0), the
    // offset is (-6, -6), the top/left arms shrink to 2 cells and sit on
    // the cursor's true column/row (F27 offsetX-shifted arm parity).
    let editor = editor_shaped(MagnifierShape::Square);
    let mut list = DisplayList::new();
    editor
        .paint_magnifier(
            &mut list,
            &output(),
            view_at((2.0, 2.0), (1920, 1080)),
            &DesignTokens::default(),
        )
        .expect("cursor inside the frame");
    let arm = crate::render::Color::from_hex_token("#6366F1")
        .unwrap()
        .with_alpha8(ARM_ALPHA);
    let arms: Vec<_> = list
        .iter()
        .filter_map(|command| match command {
            Command::Fill {
                shape: Shape::Rect { rect, .. },
                color,
            } if *color == arm => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(arms.len(), 4);
    // top, right, bottom, left
    let expected = [
        (38.0, 18.0, 10.0, 20.0),
        (48.0, 38.0, 140.0, 10.0),
        (38.0, 48.0, 10.0, 140.0),
        (18.0, 38.0, 20.0, 10.0),
    ];
    for (rect, want) in arms.iter().zip(expected) {
        assert_eq!(rect.origin.x, want.0);
        assert_eq!(rect.origin.y, want.1);
        assert_eq!(rect.size.width, want.2);
        assert_eq!(rect.size.height, want.3);
    }
}

#[test]
fn paint_circle_variant_clips_and_rings() {
    let (list, _) = paint(MagnifierShape::Circle);
    let widget = crate::render::Rect::from_parts(316.0, 216.0, 170.0, 170.0);
    // The elliptic clip = the rounded-rect clip at half the widget edge.
    let clips: Vec<_> = list
        .iter()
        .filter_map(|command| match command {
            Command::PushClip(clip) => Some(clip),
            _ => None,
        })
        .collect();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].rect, widget);
    assert_eq!(clips[0].radius, 85.0);
    assert_eq!(count(&list, |c| matches!(c, Command::PopClip)), 1);
    // The 4px ring stroke, and NO square border fill.
    assert_eq!(
        count(
            &list,
            |command| matches!(command, Command::Stroke { shape: Shape::Ellipse { center, radii }, width, .. } if *width == 4.0 && center.x == 401.0 && center.y == 301.0 && radii.width == 85.0)
        ),
        1
    );
    assert_eq!(
        count(
            &list,
            |command| matches!(command, Command::Fill { shape: Shape::Rect { rect, .. }, .. } if rect.size.width == 172.0)
        ),
        0
    );
    assert_eq!(
        count(
            &list,
            |command| matches!(command, Command::Image(image) if image.dst == widget)
        ),
        1
    );
    // The clip wraps the content: push precedes the image, pop follows it.
    let push = list
        .iter()
        .position(|c| matches!(c, Command::PushClip(_)))
        .unwrap();
    let image = list
        .iter()
        .position(|c| matches!(c, Command::Image(_)))
        .unwrap();
    let pop = list
        .iter()
        .position(|c| matches!(c, Command::PopClip))
        .unwrap();
    assert!(push < image && image < pop);
}

#[test]
fn paint_nothing_when_hidden_or_frameless() {
    let mut editor = EditorState::default();
    editor.install_frame(Some(frame_with(640, 480, 1.0, (0.0, 0.0))));
    let mut list = DisplayList::new();
    // Hidden (the config default): no commands, no texture.
    assert!(
        editor
            .paint_magnifier(
                &mut list,
                &output(),
                view_at((300.0, 200.0), (1920, 1080)),
                &DesignTokens::default()
            )
            .is_none()
    );
    assert!(list.is_empty());
    // Visible but no frame installed: still nothing.
    editor.set_magnifier_visible(true);
    editor.install_frame(None);
    assert!(
        editor
            .paint_magnifier(
                &mut list,
                &output(),
                view_at((300.0, 200.0), (1920, 1080)),
                &DesignTokens::default()
            )
            .is_none()
    );
    assert!(list.is_empty());
}

// ---------------------------------------------------------------------------
// E. Toggle key + config projection
// ---------------------------------------------------------------------------

#[test]
fn key_l_toggles_the_magnifier() {
    let mut editor = EditorState::default();
    assert!(!editor.magnifier_visible());
    assert!(
        editor
            .key_press(&env(), KeyCode::KeyL, false, None)
            .consumed
    );
    assert!(editor.magnifier_visible());
    assert!(
        editor
            .key_press(&env(), KeyCode::KeyL, false, None)
            .consumed
    );
    assert!(!editor.magnifier_visible());
    // Auto-repeat never re-toggles; modified presses fall through.
    editor.set_magnifier_visible(true);
    assert!(!editor.key_press(&env(), KeyCode::KeyL, true, None).consumed);
    assert!(editor.magnifier_visible());
    let ctrl = env_with(ModifiersState::CONTROL);
    assert!(!editor.key_press(&ctrl, KeyCode::KeyL, false, None).consumed);
    assert!(editor.magnifier_visible());
}

#[test]
fn rebound_aid_keys_follow_the_keymap() {
    let mut editor = EditorState::default();
    editor
        .shortcuts_mut()
        .rebind_aids(KeyCode::KeyV, KeyCode::KeyH);
    // The shipped keys go inert; the rebound keys toggle.
    assert!(
        !editor
            .key_press(&env(), KeyCode::KeyL, false, None)
            .consumed
    );
    assert!(!editor.magnifier_visible());
    assert!(
        !editor
            .key_press(&env(), KeyCode::KeyF, false, None)
            .consumed
    );
    assert!(!editor.grid_visible());
    assert!(
        editor
            .key_press(&env(), KeyCode::KeyV, false, None)
            .consumed
    );
    assert!(editor.magnifier_visible());
    assert!(
        editor
            .key_press(&env(), KeyCode::KeyH, false, None)
            .consumed
    );
    assert!(editor.grid_visible());
    // Auto-repeat never re-toggles the rebound aids either.
    assert!(!editor.key_press(&env(), KeyCode::KeyV, true, None).consumed);
    assert!(editor.magnifier_visible());
}

#[test]
fn config_seeds_and_configure_reprojects_the_magnifier() {
    let editor = editor_shaped(MagnifierShape::Circle);
    assert!(editor.magnifier_visible());
    assert_eq!(editor.magnifier_shape(), MagnifierShape::Circle);
    let mut editor = editor;
    // A settings apply is authoritative over the session toggle.
    editor.set_magnifier_visible(false);
    editor.configure(EditorTools {
        editor: EditorConfig {
            magnifier: true,
            magnifier_shape: MagnifierShape::Square,
            ..EditorConfig::default()
        },
        ..EditorTools::default()
    });
    assert!(editor.magnifier_visible());
    assert_eq!(editor.magnifier_shape(), MagnifierShape::Square);
    editor.configure(EditorTools {
        editor: EditorConfig {
            magnifier: false,
            magnifier_shape: MagnifierShape::Circle,
            ..EditorConfig::default()
        },
        ..EditorTools::default()
    });
    assert!(!editor.magnifier_visible());
    assert_eq!(editor.magnifier_shape(), MagnifierShape::Circle);
}

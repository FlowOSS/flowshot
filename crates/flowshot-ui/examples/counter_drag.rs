//! Offscreen pixel QA for the counter drag-aim upgrade (the
//! `--verify-offscreen` pattern; NO visible window).
//!
//! Drives the PRODUCTION editor funnel (`pointer_press` / `pointer_move` /
//! `pointer_release` -> `CounterTool` -> committed scene), paints through the
//! production `ListSink` bridge, renders with the production `Renderer`
//! (cosmic-text digit shaping + glyph atlas), reads the pixels back, and
//! asserts:
//!
//! - the click WITHOUT drag stayed a plain bubble (fill, no tail);
//! - the dragged bubble carries the aim triangle along the drag axis
//!   (Flameshot `circlecounttool.cpp` parity: apex at the release point,
//!   base on the press diameter);
//! - both digits are centered (white-ink centroid on the bubble center).
//!
//! Usage (from the workspace root):
//!
//! ```text
//! cargo run -p flowshot-ui --example counter_drag -- --verify-offscreen OUT.png
//! ```
//!
//! The verdict log lands next to the PNG (same stem, `.txt`); exit is
//! non-zero when any check fails.

#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "QA pixel arithmetic: small non-negative surface constants, offsets, and counts - far inside the i32/u32/f64 exact ranges"
)]

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use flowshot_core::config::Config;
use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo, PhysicalSize, Transform};
use flowshot_core::scene::ToolObjectData;
use flowshot_ui::gpu::{GpuContext, new_instance};
use flowshot_ui::render::{
    Color, DisplayList, Rect, RenderTarget, Renderer, Shape, read_texture_rgba,
};
use flowshot_ui::{
    EditorEnv, EditorState, EditorTools, EditorView, ToolKind, ToolRegistry, register_counter_tool,
};
use winit::event::MouseButton;
use winit::keyboard::ModifiersState;

/// The offscreen surface (logical == physical at scale 1).
const W: usize = 480;
const H: usize = 200;
/// The click-without-drag bubble center.
const PLAIN: (f64, f64) = (80.0, 80.0);
/// The dragged bubble: press (center) and release (aim apex).
const DRAG_FROM: (f64, f64) = (200.0, 80.0);
const DRAG_TO: (f64, f64) = (290.0, 80.0);
/// The synthetic wallpaper (the `qa_bundle` slate).
const SLATE: [u8; 3] = [0x33, 0x41, 0x55];

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out) = args
        .iter()
        .position(|arg| arg == "--verify-offscreen")
        .and_then(|index| args.get(index + 1))
    else {
        eprintln!("usage: counter_drag --verify-offscreen OUT.png");
        return ExitCode::from(2);
    };
    match verify(Path::new(out)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("counter drag verify failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Drives the funnel, renders, pixel-asserts, and writes the evidence pair.
fn verify(out: &Path) -> Result<(), String> {
    let editor = driven_editor();
    let scene = scene_report(&editor);
    let pixels = render(&editor)?;
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    image::save_buffer(out, &pixels, W as u32, H as u32, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    let checks = pixel_checks(&pixels);
    let failed = checks.iter().any(|(_, ok, _)| !*ok);
    let verdict = format!(
        "{scene}\n{}\nverdict: {}\n",
        checks
            .iter()
            .map(|(name, ok, detail)| format!(
                "{} {name}: {detail}",
                if *ok { "PASS" } else { "FAIL" }
            ))
            .collect::<Vec<_>>()
            .join("\n"),
        if failed { "FAIL" } else { "PASS" },
    );
    let log = out.with_extension("txt");
    std::fs::write(&log, &verdict).map_err(|e| e.to_string())?;
    print!("{verdict}");
    if failed {
        return Err("pixel checks failed".to_owned());
    }
    Ok(())
}

/// The production funnel: a plain click, then a press-drag-release.
fn driven_editor() -> EditorState {
    let mut registry = ToolRegistry::new();
    register_counter_tool(&mut registry);
    let mut editor = EditorState::new(EditorTools::from_config(&Config::default()), registry);
    editor.activate_tool(ToolKind::Counter);
    let env = EditorEnv {
        selection: None,
        modifiers: ModifiersState::empty(),
        now: Instant::now(),
        picker_visible: false,
        mouse: None,
    };
    let at = |x: f64, y: f64| LogicalPoint::from_raw(x, y);
    editor.pointer_press(&env, MouseButton::Left, at(PLAIN.0, PLAIN.1));
    editor.pointer_release(&env, MouseButton::Left, at(PLAIN.0, PLAIN.1));
    editor.pointer_press(&env, MouseButton::Left, at(DRAG_FROM.0, DRAG_FROM.1));
    editor.pointer_move(&env, at(DRAG_TO.0, DRAG_TO.1));
    editor.pointer_release(&env, MouseButton::Left, at(DRAG_TO.0, DRAG_TO.1));
    editor
}

/// The committed-state half of the evidence (counts + the persisted pointer).
fn scene_report(editor: &EditorState) -> String {
    let counts = editor.scene().counter_counts();
    let pointer = editor
        .scene()
        .get_object(1)
        .and_then(|object| match object.to_data() {
            ToolObjectData::Counter(counter) => counter.pointer,
            _ => None,
        });
    format!("SCENE counts={counts:?} dragged_pointer={pointer:?}")
}

/// Paints the driven editor over the slate field and reads the pixels back.
fn render(editor: &EditorState) -> Result<Vec<u8>, String> {
    let output = OutputInfo::new(
        "QA-1",
        "QA-1",
        LogicalRect::from_raw(0.0, 0.0, W as f64, H as f64),
        PhysicalSize::from_raw(W as i32, H as i32),
        1.0,
        Transform::Normal,
    )
    .map_err(|e| e.to_string())?;
    let mut list = DisplayList::new();
    list.fill(
        Shape::Rect {
            rect: Rect::from_parts(0.0, 0.0, W as f32, H as f32),
            radius: 0.0,
        },
        Color::from_rgba8(SLATE[0], SLATE[1], SLATE[2], 255),
    );
    editor.paint_into(
        &mut list,
        &output,
        EditorView {
            // No cursor track: the frame shows ONLY committed objects (the
            // hover preview must not contaminate the pixel asserts).
            mouse: None,
            selection: None,
            modifiers: ModifiersState::empty(),
        },
    );
    let instance = new_instance();
    let gpu = GpuContext::new_headless(&instance).map_err(|_| "headless GPU init failed")?;
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let (w32, h32) = (W as u32, H as u32);
    let target = renderer
        .create_offscreen_target(&gpu.device, w32, h32)
        .map_err(|e| e.to_string())?;
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render(
            &gpu.device,
            &gpu.queue,
            &RenderTarget {
                view: &view,
                width: w32,
                height: h32,
            },
            &list,
        )
        .map_err(|e| e.to_string())?;
    read_texture_rgba(&gpu.device, &gpu.queue, &target, w32, h32).map_err(|e| e.to_string())
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
    let at = (y * W + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 70 && p[2] < 70
}

fn is_slate(p: [u8; 4]) -> bool {
    p[0].abs_diff(SLATE[0]) < 16 && p[1].abs_diff(SLATE[1]) < 16 && p[2].abs_diff(SLATE[2]) < 16
}

fn is_ink(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] > 200 && p[2] > 200
}

/// The white-digit ink centroid in a +-6/+-7 window around the center (the
/// outline hairlines sit at radius 16+, safely outside the window).
fn digit_centroid(pixels: &[u8], center: (usize, usize)) -> Option<(f64, f64, usize)> {
    let (cx, cy) = center;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut count = 0usize;
    for y in cy - 7..=cy + 7 {
        for x in cx - 6..=cx + 6 {
            if is_ink(pixel(pixels, x, y)) {
                sum_x += x as f64;
                sum_y += y as f64;
                count += 1;
            }
        }
    }
    (count > 0).then(|| (sum_x / count as f64, sum_y / count as f64, count))
}

fn sample(pixels: &[u8], x: usize, y: usize) -> String {
    format!("({x},{y})={:?}", pixel(pixels, x, y))
}

/// The pixel oracle: tail geometry, plain-bubble absence of a tail, fill,
/// and the centered digits.
fn pixel_checks(pixels: &[u8]) -> Vec<(&'static str, bool, String)> {
    let mut checks = Vec::new();
    let mut solid = |name: &'static str, at: (usize, usize), want_red: bool| {
        let (x, y) = at;
        let ok = if want_red {
            is_red(pixel(pixels, x, y))
        } else {
            is_slate(pixel(pixels, x, y))
        };
        checks.push((name, ok, sample(pixels, x, y)));
    };
    // The plain bubble: fill inside, NO tail along any axis, background
    // clear of the ring (outer radius 18 + hairline).
    solid("plain-fill", (80, 91), true);
    solid("plain-no-tail-right", (120, 80), false);
    solid("plain-no-tail-left", (40, 80), false);
    solid("plain-ring-clear", (100, 80), false);
    // The dragged bubble: fill inside + the triangle along +x (base on the
    // press diameter, apex at the release point 90px out).
    solid("dragged-fill", (200, 91), true);
    solid("tail-axis-mid", (250, 80), true);
    solid("tail-inside-edge", (230, 86), true);
    solid("tail-near-apex", (270, 80), true);
    solid("tail-outside-edge", (230, 95), false);
    solid("tail-outside-mid", (250, 90), false);
    // The centered digits: ink centroid on the bubble center.
    for (name, center) in [
        ("digit-1-centered", (80usize, 80usize)),
        ("digit-2-centered", (200, 80)),
    ] {
        match digit_centroid(pixels, center) {
            Some((x, y, count)) => {
                let cx = center.0 as f64;
                let cy = center.1 as f64;
                let ok = count >= 20 && (x - cx).abs() <= 1.5 && (y - cy).abs() <= 2.5;
                checks.push((
                    name,
                    ok,
                    format!("ink={count} centroid=({x:.2},{y:.2}) want=({cx:.0},{cy:.0})"),
                ));
            }
            None => checks.push((name, false, "no digit ink found".to_owned())),
        }
    }
    checks
}

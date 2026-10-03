//! Wheel-sizing offscreen QA (the shape-tool wheel defect evidence).
//!
//! Drives the PRODUCTION funnel through the `test-drive` injection seam and
//! renders through the production paint path ([`flowshot_ui::build_overlay_frame`])
//! offscreen - the `qa_bundle` pattern, no visible windows. For EVERY
//! size-carrying tool it proves the wheel contract end to end:
//!
//! 1. one 120-unit notch steps the dispatched size slot by +1,
//! 2. the size-notifier HUD flashes and PAINTS (pixel diff in the box
//!    region against the pre-wheel frame),
//! 3. the hover preview reflects the new size immediately (pixel diff at
//!    the cursor; the text tool has no cursor dot - its notifier is the
//!    HUD, its preview is the edit widget),
//! 4. the box hides after the Flameshot 600ms notifier timeout
//!    ([`flowshot_ui::editor::DIGIT_RESET_DELAY`]) via the production tick.
//!
//! Usage (from the workspace root):
//!
//! ```text
//! cargo run -p flowshot-ui --example wheel_sizing --features test-drive -- OUT_DIR
//! ```
//!
//! Writes `wheel-<tool>-{before,notifier,expired}.png` per tool plus the
//! assertion table on stdout; exits non-zero on any contract violation.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::config::UiConfig;
use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::tokens::DesignTokens;
use flowshot_ui::backdrop::{Backdrop, BackdropOptions, FrozenCapture};
use flowshot_ui::editor::DIGIT_RESET_DELAY;
use flowshot_ui::gpu::{GpuContext, new_instance};
use flowshot_ui::render::{RenderTarget, Renderer, RgbaImage, read_texture_rgba};
use flowshot_ui::{
    FramePixels, InputRouter, OverlayCore, SyntheticInput, ToolKind, WindowSlot,
    build_overlay_frame, register_counter_tool, register_shape_tools, register_text_tool,
};
use winit::keyboard::KeyCode;

const LOGICAL_W: f64 = 1920.0;
const LOGICAL_H: f64 = 1080.0;
const SELECTION: LogicalRect = LogicalRect::from_raw(560.0, 300.0, 800.0, 450.0);
const CURSOR: (f64, f64) = (900.0, 500.0);
/// The notifier box region (window top-left; margin + box extents with
/// headroom) and the hover-dot region around the cursor.
const HUD_REGION: (u32, u32, u32, u32) = (0, 0, 200, 80);
const DOT_REGION: (u32, u32, u32, u32) = (880, 480, 920, 520);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowshot: wheel-sizing QA failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("OUT_DIR is required (see the module docs)")?,
    );
    std::fs::create_dir_all(&out)?;

    let instance = new_instance();
    let gpu = GpuContext::new_headless(&instance)?;

    // (name, activation key - None = toolbar-only tool, activated via API;
    // has_dot = the tool paints a cursor preview dot).
    let tools: [(&str, Option<KeyCode>, ToolKind, bool); 8] = [
        ("pencil", Some(KeyCode::KeyP), ToolKind::Pencil, true),
        ("line", Some(KeyCode::KeyD), ToolKind::Line, true),
        ("arrow", Some(KeyCode::KeyA), ToolKind::Arrow, true),
        ("rectangle", Some(KeyCode::KeyR), ToolKind::Rectangle, true),
        ("circle", Some(KeyCode::KeyC), ToolKind::Circle, true),
        ("marker", Some(KeyCode::KeyM), ToolKind::Marker, true),
        ("counter", None, ToolKind::Counter, true),
        ("text", Some(KeyCode::KeyT), ToolKind::Text, false),
    ];

    let mut failures = 0;
    println!(
        "{:<10} {:>9} {:>5} {:>9} {:>9} {:>8}",
        "tool", "size", "hud", "hud_diff", "dot_diff", "expired"
    );
    for (name, key, kind, has_dot) in tools {
        let (mut bundle, t0) = build_scene(&gpu)?;
        let slot = WindowSlot::new(0);
        bundle
            .core
            .inject_event(SyntheticInput::pointer_moved(slot, CURSOR.0, CURSOR.1));
        match key {
            Some(key) => {
                bundle
                    .core
                    .inject_event(SyntheticInput::key_press(slot, key));
            }
            None => bundle.core.editor_mut().activate_tool(kind),
        }
        let size_before = bundle.core.editor().tool_size();
        let before = render(
            &gpu,
            &mut bundle,
            t0,
            &out.join(format!("wheel-{name}-before.png")),
        )?;

        bundle.core.inject_event(SyntheticInput::wheel(slot, 120));
        let size_after = bundle.core.editor().tool_size();
        let hud = bundle.core.chrome().size_hud_visible();
        let after = render(
            &gpu,
            &mut bundle,
            t0,
            &out.join(format!("wheel-{name}-notifier.png")),
        )?;
        let hud_diff = region_diff(&before, &after, HUD_REGION);
        let dot_diff = region_diff(&before, &after, DOT_REGION);

        // The deadline arms from the wheel event's own clock, which is at
        // most `Instant::now()` - anchoring here expires it deterministically
        // despite the wall time the two renders consumed.
        let later = std::time::Instant::now() + DIGIT_RESET_DELAY + Duration::from_millis(1);
        bundle.core.tick(later);
        let expired_hidden = !bundle.core.chrome().size_hud_visible();
        render(
            &gpu,
            &mut bundle,
            later,
            &out.join(format!("wheel-{name}-expired.png")),
        )?;

        let ok = size_after == size_before + 1
            && hud
            && hud_diff > 0
            && (dot_diff > 0 || !has_dot)
            && expired_hidden;
        if !ok {
            failures += 1;
        }
        println!(
            "{:<10} {:>4}->{:<4} {:>5} {:>9} {:>9} {:>8} {}",
            name,
            size_before,
            size_after,
            hud,
            hud_diff,
            dot_diff,
            expired_hidden,
            if ok { "PASS" } else { "FAIL" }
        );
    }
    if failures > 0 {
        return Err(format!("{failures} tool(s) violated the wheel contract").into());
    }
    println!("all {} tools PASS the wheel-sizing contract", tools.len());
    Ok(())
}

struct Bundle {
    core: OverlayCore,
    backdrop: Backdrop,
    renderer: Renderer,
    width: u32,
    height: u32,
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "QA fixture extents: logical size at scale 1, far inside u32/i32"
)]
fn build_scene(
    gpu: &GpuContext,
) -> Result<(Bundle, std::time::Instant), Box<dyn std::error::Error>> {
    let tokens = DesignTokens::default();
    let width = LOGICAL_W.round() as u32;
    let height = LOGICAL_H.round() as u32;
    let output = OutputInfo::new(
        "QA-1",
        "QA-1",
        LogicalRect::from_raw(0.0, 0.0, LOGICAL_W, LOGICAL_H),
        PhysicalSize::from_raw(i32::try_from(width)?, i32::try_from(height)?),
        1.0,
        Transform::Normal,
    )?;
    let capture = FrozenCapture {
        outputs: vec![output.clone()],
        frames: vec![synthetic_frame(width, height)],
        cursor: None,
    };
    let mut backdrop = Backdrop::plan(capture, &tokens);
    let layout = OutputLayout::new(vec![output]);
    let mut core = OverlayCore::new(InputRouter::new(layout, vec![0]));
    register_shape_tools(core.editor_mut().registry_mut());
    register_text_tool(core.editor_mut().registry_mut());
    register_counter_tool(core.editor_mut().registry_mut());
    core.install_frame(Some(FramePixels {
        rgba: synthetic_frame(width, height).buffer.data.to_vec(),
        width,
        height,
        scale: 1.0,
        origin: LogicalPoint::from_raw(0.0, 0.0),
    }));
    core.configure_chrome(&UiConfig::default());
    core.selection_mut().retheme(&tokens);
    core.set_motion_reduced(true);
    core.selection_mut().set_rect(Some(SELECTION));
    let t0 = std::time::Instant::now();
    core.tick(t0);

    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.textures_mut().insert(
        &gpu.device,
        &gpu.queue,
        flowshot_ui::widgets::ICON_ATLAS_ID,
        &RgbaImage {
            width: flowshot_ui::widgets::ATLAS_WIDTH,
            height: flowshot_ui::widgets::ATLAS_HEIGHT,
            data: flowshot_ui::widgets::ICON_ATLAS,
        },
    )?;
    backdrop.upload_for(0, &mut renderer, gpu)?;
    backdrop.upload_cursor(&mut renderer, gpu)?;
    Ok((
        Bundle {
            core,
            backdrop,
            renderer,
            width,
            height,
        },
        t0,
    ))
}

fn render(
    gpu: &GpuContext,
    bundle: &mut Bundle,
    now: std::time::Instant,
    path: &Path,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let Bundle {
        core,
        backdrop,
        renderer,
        width,
        height,
    } = bundle;
    let options = BackdropOptions {
        dim: true,
        cursor_visible: false,
        selection: None,
    };
    let frame = build_overlay_frame(
        core,
        WindowSlot::new(0),
        (*width, *height),
        Some((backdrop, &options)),
        now,
    );
    let target = renderer.create_offscreen_target(&gpu.device, *width, *height)?;
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.render(
        &gpu.device,
        &gpu.queue,
        &RenderTarget {
            view: &view,
            width: *width,
            height: *height,
        },
        &frame.list,
    )?;
    let pixels = read_texture_rgba(&gpu.device, &gpu.queue, &target, *width, *height)?;
    let image = image::RgbaImage::from_raw(*width, *height, pixels.clone())
        .ok_or("readback dimensions disagree")?;
    image.save(path)?;
    Ok(pixels)
}

/// Counts differing pixels between two frames inside a region.
fn region_diff(a: &[u8], b: &[u8], region: (u32, u32, u32, u32)) -> usize {
    const WIDTH: u32 = 1920;
    let (x0, y0, x1, y1) = region;
    let mut diff = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let offset = ((y * WIDTH + x) * 4) as usize;
            if a[offset..offset + 4] != b[offset..offset + 4] {
                diff += 1;
            }
        }
    }
    diff
}

fn synthetic_frame(width: u32, height: u32) -> Frame {
    let mut data = vec![0u8; (width * height * 4) as usize];
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&[0x33, 0x41, 0x55, 0xFF]);
    }
    Frame {
        buffer: FrameBuffer {
            data: bytes::BytesMut::from(data.as_slice()),
            width,
            height,
            stride: width * 4,
            format: FrameFormat::Rgba8888,
        },
        output: OutputRef::Connector("QA-1".to_owned()),
        scale: 1.0,
        transform: Transform::Normal,
    }
}

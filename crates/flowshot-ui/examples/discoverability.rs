//! Discoverability evidence renderer (tooltip + quick-aids chips).
//!
//! Renders the two discoverability surfaces OFFSCREEN through the
//! PRODUCTION paint path ([`flowshot_ui::build_overlay_frame`]) with a
//! synthetic clock, mirroring the `qa_bundle` harness pattern:
//!
//! - `tooltip-open.png`: the 400ms toolbar tooltip showing description +
//!   the ACTIVE binding (`Arrow — pointed arrow [A]`).
//! - `tooltip-rebound.png`: the same button after rebinding Arrow to W -
//!   the tooltip follows the live key map (never hardcoded).
//! - `aids-off.png`: the quick-aids chip cluster (magnifier/grid) dimmed,
//!   docked ABOVE the selection's leading edge (the adjacent placement,
//!   diagonally opposite the toolbar), clear of selection/toolbar/HUD.
//! - `aids-on.png`: both aids toggled on (L+F pressed) - chips
//!   accent-tinted, grid overlay + magnifier live in the same frame.
//! - `aids-click.png`: the magnifier chip CLICKED through the production
//!   funnel (injected motion + press + release) - the aid toggles with no
//!   keyboard, the hovered chip carries the wash, and the click started no
//!   selection drag.
//! - `aids-no-selection.png`: no selection yet - the cluster docks at the
//!   output's bottom-center edge (the documented fallback).
//!
//! Usage (from the workspace root):
//!
//! ```text
//! cargo run -p flowshot-ui --example discoverability --features test-drive -- OUT_DIR
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::tokens::DesignTokens;
use flowshot_ui::backdrop::{Backdrop, BackdropOptions, FrozenCapture};
use flowshot_ui::gpu::{GpuContext, OVERLAY_BACKENDS};
use flowshot_ui::render::{RenderTarget, Renderer, RgbaImage, read_texture_rgba};
use flowshot_ui::{
    FramePixels, InputRouter, OverlayCore, SyntheticInput, WindowSlot, build_overlay_frame,
    register_counter_tool, register_pixelate_tools, register_selection_tools, register_shape_tools,
    register_text_tool,
};
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

const LOGICAL_W: f64 = 1920.0;
const LOGICAL_H: f64 = 1080.0;
const SELECTION: LogicalRect = LogicalRect::from_raw(560.0, 300.0, 800.0, 450.0);
/// The settled frame instant: reveal (<=180ms) and wash land, the 400ms
/// tooltip delay has run for a hover injected at scene build.
const SETTLED_MS: u64 = 500;
/// The magnifier chip's center: the cluster docks above the selection's
/// leading edge at (560, 300 - 24.8 - 8); the chip box is 104.4 x 24.8
/// (base-14 font, 16.8 line, pad/gap 4, "Magnifier" = 9 chars * 14 * 0.6
/// estimated advance).
const CHIP_X: f64 = 612.2;
const CHIP_Y: f64 = 279.6;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowshot: discoverability evidence failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("OUT_DIR is required")?);
    std::fs::create_dir_all(&out)?;

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: OVERLAY_BACKENDS,
        ..wgpu::InstanceDescriptor::default()
    });
    let gpu = GpuContext::new_headless(&instance)?;

    // The default toolbar's first cell (arrow): plate x = 560+800-444,
    // cell center = plate + padding + half a 32px cell.
    let (cell_x, cell_y) = (940.0, 782.0);

    let (mut scene, t0) = build_scene(&gpu)?;
    scene
        .core
        .inject_event(SyntheticInput::pointer_moved(SLOT, cell_x, cell_y));
    render(
        &gpu,
        &mut scene,
        t0 + Duration::from_millis(SETTLED_MS),
        &out.join("tooltip-open.png"),
    )?;

    let (mut scene, t0) = build_scene(&gpu)?;
    scene
        .core
        .editor_mut()
        .shortcuts_mut()
        .rebind(flowshot_ui::ToolKind::Arrow, Some(KeyCode::KeyW));
    scene
        .core
        .inject_event(SyntheticInput::pointer_moved(SLOT, cell_x, cell_y));
    render(
        &gpu,
        &mut scene,
        t0 + Duration::from_millis(SETTLED_MS),
        &out.join("tooltip-rebound.png"),
    )?;

    let (mut scene, t0) = build_scene(&gpu)?;
    scene
        .core
        .inject_event(SyntheticInput::pointer_moved(SLOT, 960.0, 540.0));
    render(
        &gpu,
        &mut scene,
        t0 + Duration::from_millis(SETTLED_MS),
        &out.join("aids-off.png"),
    )?;

    let (mut scene, t0) = build_scene(&gpu)?;
    scene
        .core
        .inject_event(SyntheticInput::key_press(SLOT, KeyCode::KeyL));
    scene
        .core
        .inject_event(SyntheticInput::key_press(SLOT, KeyCode::KeyF));
    scene
        .core
        .inject_event(SyntheticInput::pointer_moved(SLOT, 960.0, 540.0));
    render(
        &gpu,
        &mut scene,
        t0 + Duration::from_millis(SETTLED_MS),
        &out.join("aids-on.png"),
    )?;

    let (mut scene, t0) = build_scene(&gpu)?;
    scene
        .core
        .inject_event(SyntheticInput::pointer_moved(SLOT, CHIP_X, CHIP_Y));
    scene.core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        true,
    ));
    scene.core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        false,
    ));
    render(
        &gpu,
        &mut scene,
        t0 + Duration::from_millis(SETTLED_MS),
        &out.join("aids-click.png"),
    )?;

    let (mut scene, t0) = build_scene(&gpu)?;
    scene.core.selection_mut().set_rect(None);
    scene
        .core
        .inject_event(SyntheticInput::pointer_moved(SLOT, 960.0, 540.0));
    render(
        &gpu,
        &mut scene,
        t0 + Duration::from_millis(SETTLED_MS),
        &out.join("aids-no-selection.png"),
    )?;

    println!("wrote 6 evidence PNGs into {}", out.display());
    Ok(())
}

const SLOT: WindowSlot = WindowSlot::new(0);

struct Scene {
    core: OverlayCore,
    backdrop: Backdrop,
    renderer: Renderer,
    width: u32,
    height: u32,
}

fn synthetic_frame(width: u32, height: u32, scale: f64) -> Frame {
    let mut data = vec![0u8; (width * height * 4) as usize];
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&[0x33, 0x41, 0x55, 0xFF]);
    }
    let patch = |data: &mut [u8], x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]| {
        for row in y..(y + h).min(height) {
            for column in x..(x + w).min(width) {
                let offset = ((row * width + column) * 4) as usize;
                data[offset..offset + 4].copy_from_slice(&rgba);
            }
        }
    };
    patch(
        data.as_mut_slice(),
        120,
        120,
        400,
        260,
        [0xF4, 0x3F, 0x5E, 0xFF],
    );
    patch(
        data.as_mut_slice(),
        700,
        500,
        520,
        300,
        [0x22, 0xC5, 0x5E, 0xFF],
    );
    patch(
        data.as_mut_slice(),
        1400,
        200,
        380,
        620,
        [0xEA, 0xB3, 0x08, 0xFF],
    );
    Frame {
        buffer: FrameBuffer {
            data: bytes::BytesMut::from(data.as_slice()),
            width,
            height,
            stride: width * 4,
            format: FrameFormat::Rgba8888,
        },
        output: OutputRef::Connector("QA-1".to_owned()),
        scale,
        transform: Transform::Normal,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "QA fixture extents: logical size at scale 1, far inside u32/i32"
)]
fn build_scene(gpu: &GpuContext) -> Result<(Scene, Instant), Box<dyn std::error::Error>> {
    let tokens = DesignTokens::default();
    let width = LOGICAL_W as u32;
    let height = LOGICAL_H as u32;
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
        frames: vec![synthetic_frame(width, height, 1.0)],
        cursor: None,
    };
    let mut backdrop = Backdrop::plan(capture, &tokens);
    let layout = OutputLayout::new(vec![output]);
    let mut core = OverlayCore::new(InputRouter::new(layout, vec![0]));
    register_shape_tools(core.editor_mut().registry_mut());
    register_text_tool(core.editor_mut().registry_mut());
    register_pixelate_tools(core.editor_mut().registry_mut());
    register_counter_tool(core.editor_mut().registry_mut());
    register_selection_tools(core.editor_mut().registry_mut());
    core.install_frame(Some(FramePixels {
        rgba: synthetic_frame(width, height, 1.0).buffer.data.to_vec(),
        width,
        height,
        scale: 1.0,
        origin: LogicalPoint::from_raw(0.0, 0.0),
    }));
    core.configure_chrome(&flowshot_core::config::UiConfig::default());
    core.selection_mut().retheme(&tokens);
    core.selection_mut().set_rect(Some(SELECTION));

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
    // The synthetic clock starts AFTER the GPU warm-up so the hover the
    // caller injects next always lands inside the settled render instant.
    let t0 = Instant::now();
    core.tick(t0);
    Ok((
        Scene {
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
    scene: &mut Scene,
    now: Instant,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let Scene {
        core,
        backdrop,
        renderer,
        width,
        height,
    } = scene;
    let options = BackdropOptions {
        dim: true,
        cursor_visible: false,
        selection: None,
    };
    let frame = build_overlay_frame(
        core,
        SLOT,
        (*width, *height),
        Some((backdrop, &options)),
        now,
    );
    if let Some(texture) = frame.magnifier.as_ref() {
        renderer.textures_mut().insert(
            &gpu.device,
            &gpu.queue,
            texture.id,
            &RgbaImage {
                width: texture.width,
                height: texture.height,
                data: &texture.pixels,
            },
        )?;
    }
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
    let image = image::RgbaImage::from_raw(*width, *height, pixels)
        .ok_or("readback dimensions disagree")?;
    image.save(path)?;
    println!("wrote {}", path.display());
    Ok(())
}

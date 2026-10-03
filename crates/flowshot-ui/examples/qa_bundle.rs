//! Visual QA bundle renderer (plan todo 41, no-visible-windows policy).
//!
//! Renders the polished overlay/pin/launcher surfaces OFFSCREEN through the
//! PRODUCTION paint path ([`flowshot_ui::build_overlay_frame`] - the same
//! function the live shell's `render_window` consumes) into PNGs, with
//! deterministic motion stills from a synthetic clock (fixed before/mid/
//! after offsets per the plan's MOTION QA HONESTY rule), the reduced-motion
//! oracle pair (mid == after, pixel-asserted), font-scale specimens at
//! 100/125/150/200%, dark + light theme passes, and the animated-transition
//! frame-time log (acceptance: <8ms avg).
//!
//! Usage (from the workspace root):
//!
//! ```text
//! cargo run -p flowshot-ui --example qa_bundle --features test-drive -- \
//!     OUT_DIR [--theme dark|light] [--only name1,name2] \
//!     [--reduced-motion-pair]
//! ```
//!
//! Settings tabs render through the dedicated `settings_offscreen` example;
//! the notification surface is N/A (desktop notifications are compositor-
//! rendered - `FlowShot` paints no notification chrome), recorded in the
//! shot list.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::config::{MagnifierShape, UiConfig};
use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::tokens::DesignTokens;
use flowshot_ui::backdrop::{Backdrop, BackdropOptions, FrozenCapture};
use flowshot_ui::gpu::{GpuContext, new_instance};
use flowshot_ui::render::{RenderTarget, Renderer, RgbaImage, read_texture_rgba};
use flowshot_ui::{
    EditorTools, FramePixels, InputRouter, OverlayCore, SyntheticInput, WindowSlot,
    build_overlay_frame,
};
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

const LOGICAL_W: f64 = 1920.0;
const LOGICAL_H: f64 = 1080.0;
const SELECTION: LogicalRect = LogicalRect::from_raw(560.0, 300.0, 800.0, 450.0);

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowshot: qa bundle failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Theme {
    Dark,
    Light,
}

impl Theme {
    fn ui_config(self) -> UiConfig {
        match self {
            Self::Dark => UiConfig::default(),
            // Light pass: slate-50 chrome ink, indigo-600 accent (darker on
            // light surfaces); the palette tokens drive every surface.
            Self::Light => UiConfig {
                accent_color: "#4F46E5".to_owned(),
                contrast_color: "#F8FAFC".to_owned(),
                ..UiConfig::default()
            },
        }
    }

    fn tokens(self) -> DesignTokens {
        let mut tokens = DesignTokens::default();
        let config = self.ui_config();
        tokens.palette.accent = config.accent_color;
        tokens.palette.contrast = config.contrast_color;
        tokens
    }
}

/// One bundle entry: a scene recipe, the synthetic-clock offset for the
/// motion still, the output scale, and the motion switches.
struct Shot {
    name: &'static str,
    scene: Scene,
    offset_ms: u64,
    scale: f64,
    theme: Theme,
    reduced: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    Idle,
    Selection,
    SelectionHud,
    ToolbarReveal,
    ToolbarHover,
    HandleHover,
    ColorWheel,
    SidePanel,
    MagnifierSquare,
    MagnifierCircle,
    PinZoom,
    Launcher,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().ok_or("OUT_DIR is required")?);
    let mut theme = Theme::Dark;
    let mut only: Option<Vec<String>> = None;
    let mut reduced_motion_pair = false;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--theme" => {
                theme = match args.next().as_deref() {
                    Some("light") => Theme::Light,
                    _ => Theme::Dark,
                };
            }
            "--only" => {
                only = Some(
                    args.next()
                        .ok_or("--only wants a comma list")?
                        .split(',')
                        .map(str::to_owned)
                        .collect(),
                );
            }
            "--reduced-motion-pair" => reduced_motion_pair = true,
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    std::fs::create_dir_all(&out)?;

    let instance = new_instance();
    let gpu = GpuContext::new_headless(&instance)?;

    let shots = shot_list(theme, reduced_motion_pair);
    let mut written = Vec::new();
    for shot in &shots {
        if let Some(only) = &only
            && !only.iter().any(|name| shot.name.contains(name))
        {
            continue;
        }
        let path = out.join(format!("qa-{}.png", shot.name));
        render_shot(&gpu, shot, &path)?;
        println!("wrote {}", path.display());
        written.push(shot.name.to_owned());
    }

    if reduced_motion_pair {
        let verdict = reduced_pair_verdict(&out)?;
        std::fs::write(out.join("reduced-motion-pair.txt"), &verdict)?;
        print!("{verdict}");
        if verdict.contains("FAIL") {
            return Err("reduced-motion pair differs (transitions must be instant)".into());
        }
    }

    let timing = frame_time_log(&gpu, theme)?;
    std::fs::write(out.join("frame-times.txt"), &timing)?;
    print!("{timing}");

    std::fs::write(
        out.join("shot-list.txt"),
        format!(
            "theme={theme:?} reduced_motion_pair={reduced_motion_pair}\nrendered:\n{}\n\
             deferred to dedicated harnesses:\n\
             - settings-{{general,interface,filename,shortcuts}}.png (settings_offscreen example)\n\
             - notification: N/A - desktop notifications are compositor-rendered; FlowShot paints no notification chrome\n\
             - crosshair: live vertex-overlay pipeline only (the offscreen list path excludes it by design)\n",
            written
                .iter()
                .map(|name| format!("  qa-{name}.png"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )?;
    Ok(())
}

/// The reduced-motion oracle pair (the plan's failure QA): with reduced
/// motion the mid still and the settled still must be PIXEL-IDENTICAL
/// (every transition snapped).
fn reduced_pair_verdict(out: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let mid = image::open(out.join("qa-reduced-toolbar-mid.png"))?.to_rgba8();
    let after = image::open(out.join("qa-reduced-toolbar-after.png"))?.to_rgba8();
    let identical = mid.as_raw() == after.as_raw();
    Ok(format!(
        "reduced-motion pair: mid(t0+90ms) vs after(t0+240ms) pixel-identical = {identical}\n\
         verdict: {}\n",
        if identical { "PASS" } else { "FAIL" }
    ))
}

fn shot_list(theme: Theme, reduced_pair: bool) -> Vec<Shot> {
    let at = |name: &'static str, scene: Scene, offset_ms: u64| Shot {
        name,
        scene,
        offset_ms,
        scale: 1.0,
        theme,
        reduced: false,
    };
    let scaled = |name: &'static str, scale: f64| Shot {
        name,
        scene: Scene::SelectionHud,
        offset_ms: 240,
        scale,
        theme,
        reduced: false,
    };
    let mut shots = vec![
        at("overlay-idle", Scene::Idle, 0),
        at("selection", Scene::Selection, 0),
        at("selection-hud", Scene::SelectionHud, 0),
        at("toolbar-reveal-before", Scene::ToolbarReveal, 0),
        at("toolbar-reveal-mid", Scene::ToolbarReveal, 90),
        at("toolbar-reveal-after", Scene::ToolbarReveal, 240),
        at("toolbar-hover", Scene::ToolbarHover, 240),
        at("handle-hover-before", Scene::HandleHover, 0),
        at("handle-hover-mid", Scene::HandleHover, 60),
        at("handle-hover-after", Scene::HandleHover, 200),
        at("color-wheel-mid", Scene::ColorWheel, 75),
        at("color-wheel-after", Scene::ColorWheel, 240),
        at("side-panel-mid", Scene::SidePanel, 90),
        at("side-panel-after", Scene::SidePanel, 240),
        at("magnifier-square", Scene::MagnifierSquare, 240),
        at("magnifier-circle", Scene::MagnifierCircle, 240),
        at("pin-zoom-before", Scene::PinZoom, 0),
        at("pin-zoom-mid", Scene::PinZoom, 75),
        at("pin-zoom-after", Scene::PinZoom, 200),
        at("launcher", Scene::Launcher, 0),
        // Font rendering check at 100/125/150/200% (headless fixtures).
        scaled("font-scale-100", 1.0),
        scaled("font-scale-125", 1.25),
        scaled("font-scale-150", 1.5),
        scaled("font-scale-200", 2.0),
    ];
    if theme == Theme::Dark {
        // The light-theme pass over the chrome-bearing surfaces.
        let light = |name: &'static str, scene: Scene| Shot {
            name,
            scene,
            offset_ms: 240,
            scale: 1.0,
            theme: Theme::Light,
            reduced: false,
        };
        shots.extend([
            light("light-toolbar", Scene::ToolbarReveal),
            light("light-selection-hud", Scene::SelectionHud),
            light("light-color-wheel", Scene::ColorWheel),
            light("light-side-panel", Scene::SidePanel),
        ]);
    }
    if reduced_pair {
        // The failure-QA oracle pair: with reduced motion the mid still and
        // the settled still must be pixel-identical (transitions instant).
        shots.extend([
            Shot {
                name: "reduced-toolbar-mid",
                scene: Scene::ToolbarReveal,
                offset_ms: 90,
                scale: 1.0,
                theme,
                reduced: true,
            },
            Shot {
                name: "reduced-toolbar-after",
                scene: Scene::ToolbarReveal,
                offset_ms: 240,
                scale: 1.0,
                theme,
                reduced: true,
            },
        ]);
    }
    shots
}

/// The synthetic frozen wallpaper: flat slate with deterministic patches
/// (the dim cutout, magnifier sampling, and conformance pixel reads all want
/// stable known content).
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

struct SceneBundle {
    core: OverlayCore,
    backdrop: Backdrop,
    renderer: Renderer,
    width: u32,
    height: u32,
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "QA fixture extents: logical size times a small scale factor, far inside u32/i32"
)]
fn build_scene(
    gpu: &GpuContext,
    scene: Scene,
    scale: f64,
    theme: Theme,
    reduced: bool,
) -> Result<(SceneBundle, Instant), Box<dyn std::error::Error>> {
    let tokens = theme.tokens();
    let width = (LOGICAL_W * scale).round() as u32;
    let height = (LOGICAL_H * scale).round() as u32;
    let output = OutputInfo::new(
        "QA-1",
        "QA-1",
        LogicalRect::from_raw(0.0, 0.0, LOGICAL_W, LOGICAL_H),
        PhysicalSize::from_raw(i32::try_from(width)?, i32::try_from(height)?),
        scale,
        Transform::Normal,
    )?;
    let capture = FrozenCapture {
        outputs: vec![output.clone()],
        frames: vec![synthetic_frame(width, height, scale)],
        cursor: None,
    };
    let mut backdrop = Backdrop::plan(capture, &tokens);
    let layout = OutputLayout::new(vec![output]);
    let mut core = OverlayCore::new(InputRouter::new(layout, vec![0]));
    flowshot_ui::register_shape_tools(core.editor_mut().registry_mut());
    flowshot_ui::register_text_tool(core.editor_mut().registry_mut());
    flowshot_ui::register_pixelate_tools(core.editor_mut().registry_mut());
    flowshot_ui::register_counter_tool(core.editor_mut().registry_mut());
    flowshot_ui::register_selection_tools(core.editor_mut().registry_mut());
    core.install_frame(Some(FramePixels {
        rgba: synthetic_frame(width, height, scale).buffer.data.to_vec(),
        width,
        height,
        scale,
        origin: LogicalPoint::from_raw(0.0, 0.0),
    }));
    core.configure_chrome(&theme.ui_config());
    core.selection_mut().retheme(&tokens);
    core.set_motion_reduced(reduced);

    let slot = WindowSlot::new(0);
    if scene != Scene::Idle {
        core.selection_mut().set_rect(Some(SELECTION));
    }
    match scene {
        Scene::Idle
        | Scene::Selection
        | Scene::ToolbarReveal
        | Scene::PinZoom
        | Scene::Launcher => {}
        Scene::SelectionHud => {
            // A keyboard nudge commits a geometry change -> the HUD shows
            // (the production path; set_rect alone mirrors initialSelection).
            core.inject_event(SyntheticInput::key_press(slot, KeyCode::ArrowLeft));
        }
        Scene::ToolbarHover => {
            // Hover the 3rd toolbar cell (the wash settles by the offset).
            core.inject_event(SyntheticInput::pointer_moved(slot, 1012.0, 782.0));
        }
        Scene::HandleHover => {
            // Hover the selection's bottom-right grip.
            core.inject_event(SyntheticInput::pointer_moved(slot, 1360.0, 750.0));
        }
        Scene::ColorWheel => {
            core.inject_event(SyntheticInput::pointer_moved(slot, 900.0, 500.0));
            core.inject_event(SyntheticInput::pointer_button(
                slot,
                MouseButton::Right,
                true,
            ));
        }
        Scene::SidePanel => {
            core.inject_event(SyntheticInput::key_press(slot, KeyCode::Space));
        }
        Scene::MagnifierSquare | Scene::MagnifierCircle => {
            let mut tools = EditorTools::default();
            tools.editor.magnifier = true;
            tools.editor.magnifier_shape = if scene == Scene::MagnifierCircle {
                MagnifierShape::Circle
            } else {
                MagnifierShape::Square
            };
            core.editor_mut().configure(tools);
            core.inject_event(SyntheticInput::pointer_moved(slot, 820.0, 480.0));
        }
    }
    let t0 = Instant::now();
    core.tick(t0);

    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    upload_atlas(gpu, &mut renderer)?;
    backdrop.upload_for(0, &mut renderer, gpu)?;
    backdrop.upload_cursor(&mut renderer, gpu)?;
    Ok((
        SceneBundle {
            core,
            backdrop,
            renderer,
            width,
            height,
        },
        t0,
    ))
}

fn upload_atlas(
    gpu: &GpuContext,
    renderer: &mut Renderer,
) -> Result<(), Box<dyn std::error::Error>> {
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
    Ok(())
}

fn render_shot(
    gpu: &GpuContext,
    shot: &Shot,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match shot.scene {
        Scene::PinZoom => render_pin_shot(gpu, shot, path),
        Scene::Launcher => render_launcher_shot(gpu, shot, path),
        scene => {
            let (mut bundle, t0) = build_scene(gpu, scene, shot.scale, shot.theme, shot.reduced)?;
            let now = t0 + Duration::from_millis(shot.offset_ms);
            render_bundle(gpu, &mut bundle, now, path).map(|_| ())
        }
    }
}

fn render_bundle(
    gpu: &GpuContext,
    bundle: &mut SceneBundle,
    now: Instant,
    path: &Path,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let SceneBundle {
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
    let image = image::RgbaImage::from_raw(*width, *height, pixels.clone())
        .ok_or("readback dimensions disagree")?;
    image.save(path)?;
    Ok(pixels)
}

fn render_pin_shot(
    gpu: &GpuContext,
    shot: &Shot,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use flowshot_ui::pins::{PinBehavior, PinInput, PinState, TEXTURE_ID, frame_list};

    let image = synthetic_frame(400, 300, 1.0);
    let behavior = PinBehavior {
        tokens: shot.theme.tokens(),
        reduced_motion: shot.reduced,
        ..PinBehavior::default()
    };
    let mut state = PinState::new((400, 300), (1920, 1080), 1.0, behavior);
    let t0 = Instant::now();
    state.on_input(&PinInput::CursorMoved { x: 107.0, y: 82.0 }, t0);
    // Eight committed wheel notches: a visible zoom transition.
    state.on_input(&PinInput::Wheel { units: 120.0 * 8.0 }, t0);
    let now = t0 + Duration::from_millis(shot.offset_ms);

    let (width, height) = state.target_window();
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    renderer.textures_mut().insert(
        &gpu.device,
        &gpu.queue,
        TEXTURE_ID,
        &RgbaImage {
            width: image.buffer.width,
            height: image.buffer.height,
            data: &image.buffer.data,
        },
    )?;
    let list = frame_list(&state, now);
    let target = renderer.create_offscreen_target(&gpu.device, width, height)?;
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.render(
        &gpu.device,
        &gpu.queue,
        &RenderTarget {
            view: &view,
            width,
            height,
        },
        &list,
    )?;
    let pixels = read_texture_rgba(&gpu.device, &gpu.queue, &target, width, height)?;
    let png =
        image::RgbaImage::from_raw(width, height, pixels).ok_or("pin readback size disagree")?;
    png.save(path)?;
    Ok(())
}

fn render_launcher_shot(
    gpu: &GpuContext,
    shot: &Shot,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use flowshot_ui::launcher::{LauncherModel, LauncherWindowOptions, render_offscreen};
    use flowshot_ui::settings::ThemeMode;

    let output = OutputInfo::new(
        "QA-1",
        "QA-1",
        LogicalRect::from_raw(0.0, 0.0, LOGICAL_W, LOGICAL_H),
        PhysicalSize::from_raw(1920, 1080),
        1.0,
        Transform::Normal,
    )?;
    let mut model = LauncherModel::from_outputs(vec![output]);
    "800x450+560+300".clone_into(model.geometry_text_mut());
    let system_theme = match shot.theme {
        Theme::Dark => ThemeMode::Dark,
        Theme::Light => ThemeMode::Light,
    };
    let (width, height) = (400u32, 176u32);
    let options = LauncherWindowOptions {
        tokens: shot.theme.tokens(),
        ui_config: shot.theme.ui_config(),
        system_theme,
        ..LauncherWindowOptions::default()
    };
    let pixels = render_offscreen(gpu, &options, &mut model, width, height, 1.0)?;
    let png = image::RgbaImage::from_raw(width, height, pixels)
        .ok_or("launcher readback size disagree")?;
    png.save(path)?;
    Ok(())
}

/// The animated-transition frame-time log (acceptance: <8ms avg): renders
/// the toolbar reveal sequence frame by frame, timing build + submit (the
/// live shell's per-frame work; readback is QA-only and excluded).
fn frame_time_log(gpu: &GpuContext, theme: Theme) -> Result<String, Box<dyn std::error::Error>> {
    let (bundle, t0) = build_scene(gpu, Scene::ToolbarReveal, 1.0, theme, false)?;
    let options = BackdropOptions {
        dim: true,
        cursor_visible: false,
        selection: None,
    };
    let SceneBundle {
        core,
        backdrop,
        mut renderer,
        width,
        height,
    } = bundle;
    let target = renderer.create_offscreen_target(&gpu.device, width, height)?;
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut lines = Vec::new();
    let mut total = Duration::ZERO;
    let frames = 12u32;
    for step in 0..frames {
        let now = t0 + Duration::from_millis(u64::from(step) * 15);
        let started = Instant::now();
        let frame = build_overlay_frame(
            &core,
            WindowSlot::new(0),
            (width, height),
            Some((&backdrop, &options)),
            now,
        );
        renderer.render(
            &gpu.device,
            &gpu.queue,
            &RenderTarget {
                view: &view,
                width,
                height,
            },
            &frame.list,
        )?;
        let elapsed = started.elapsed();
        total += elapsed;
        lines.push(format!(
            "frame {step:02}: {:.3}ms",
            elapsed.as_secs_f64() * 1000.0
        ));
    }
    let avg_ms = (total.as_secs_f64() * 1000.0) / f64::from(frames);
    lines.push(format!("avg over {frames} animated frames: {avg_ms:.3}ms"));
    lines.push(format!(
        "budget 8ms: {}",
        if avg_ms < 8.0 { "PASS" } else { "FAIL" }
    ));
    Ok(lines.join("\n") + "\n")
}

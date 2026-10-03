//! Offscreen dual-monitor drag-storm perf harness (interactive-freeze diagnosis).
//!
//! Reproduces the production overlay render path UNDER a synthetic motion storm
//! without a compositor: builds the full dual-monitor scene (frozen backdrop +
//! dim + selection chrome + editor scene + magnifier + HUD + toolbar) at
//! 1920x1080 + 2560x1440 (union 4480x1440), seeds committed annotation objects,
//! then drives N pointer-motion events through the PRODUCTION routing funnel
//! (`OverlayCore::inject_event`). After every event both windows are rebuilt
//! (`build_overlay_frame`) and rendered (`Renderer::render`) into offscreen
//! targets - the exact CPU work the live shell's `render_window` does, minus
//! the wgpu present (which the compositor throttles, not the CPU budget).
//!
//! Per event the harness times the four-way breakdown the freeze diagnosis
//! needs: route (selection+editor recompute), display-list build, tessellate +
//! text-shape (`FrameStats::build_time`), and encode + submit
//! (`FrameStats::encode_time`). It reports p50/p95/p99 for each plus the
//! combined event-to-submit frame latency.
//!
//! Usage (from the workspace root):
//!
//! ```text
//! cargo run -p flowshot-ui --example perf_storm --features test-drive --release -- \
//!     [--events 500] [--mode move|resize] [--panel] [--no-magnifier] \
//!     [--objects 16] [--out PATH]
//! ```
//!
//! Run in `--release`: the debug profile's unoptimized lyon/cosmic-text skews
//! the absolute numbers (the live binary is release).

// Dev-harness fixture math: logical extents times small scales and percentile
// ranks, all far inside the cast target ranges (the render_smoke precedent).
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::config::MagnifierShape;
use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::scene::{
    ArrowObject, Color as SceneColor, CounterObject, EllipseObject, LineObject,
    Point as ScenePoint, Rect as SceneRect, RectObject, TextObject,
};
use flowshot_core::tokens::DesignTokens;
use flowshot_ui::backdrop::{Backdrop, BackdropOptions, FrozenCapture};
use flowshot_ui::gpu::{GpuContext, new_instance};
use flowshot_ui::render::{RenderTarget, Renderer, RgbaImage};
use flowshot_ui::{
    EditorTools, FramePixels, InputRouter, OverlayCore, SyntheticInput, WindowSlot,
    build_overlay_frame,
};
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// Output 0: 1920x1080 at logical origin (0,0), scale 1.
const OUT0: (f64, f64, f64, f64) = (0.0, 0.0, 1920.0, 1080.0);
/// Output 1: 2560x1440 at logical origin (1920,0), scale 1 (union 4480x1440).
const OUT1: (f64, f64, f64, f64) = (1920.0, 0.0, 2560.0, 1440.0);
/// The spanning selection both windows draw chrome for (global logical).
const SELECTION: LogicalRect = LogicalRect::from_raw(600.0, 200.0, 3200.0, 1000.0);

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
            eprintln!("flowshot: perf storm failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone)]
struct Args {
    events: usize,
    resize: bool,
    panel: bool,
    magnifier: bool,
    objects: usize,
    out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut args = Args {
        events: 500,
        resize: false,
        panel: false,
        magnifier: true,
        objects: 16,
        out: None,
    };
    let mut flags = std::env::args().skip(1);
    while let Some(flag) = flags.next() {
        let mut value = |flag: &str| {
            flags
                .next()
                .ok_or_else(|| format!("{flag} requires a value argument"))
        };
        match flag.as_str() {
            "--events" => args.events = value("--events")?.parse()?,
            "--mode" => args.resize = value("--mode")? == "resize",
            "--panel" => args.panel = true,
            "--no-magnifier" => args.magnifier = false,
            "--objects" => args.objects = value("--objects")?.parse()?,
            "--out" => args.out = Some(PathBuf::from(value("--out")?)),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(args)
}

/// One window's offscreen render rig.
struct WindowRig {
    renderer: Renderer,
    /// Kept alive so `view` stays valid (dropping the texture destroys it).
    #[expect(
        dead_code,
        reason = "the texture handle owns the GPU resource the view references"
    )]
    target: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

struct Scene {
    core: OverlayCore,
    backdrop: Backdrop,
    rigs: Vec<WindowRig>,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let instance = new_instance();
    let gpu = GpuContext::new_headless(&instance)?;
    let mut scene = build_scene(&gpu, &args)?;
    let sample_count = if gpu
        .device
        .features()
        .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
    {
        8
    } else {
        4
    };
    println!(
        "# perf_storm: dual-monitor {}x{} + {}x{} (union 4480x1440), MSAA {}x, {} objects, mode={}",
        OUT0.2,
        OUT0.3,
        OUT1.2,
        OUT1.3,
        sample_count,
        args.objects,
        if args.resize { "resize" } else { "move" }
    );

    warmup(&gpu, &mut scene);
    let report = storm(&gpu, &mut scene, &args);
    print!("{report}");
    if let Some(path) = &args.out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &report)?;
        println!("# wrote {}", path.display());
    }
    Ok(())
}

fn output(
    spec: (f64, f64, f64, f64),
    connector: &str,
) -> Result<OutputInfo, Box<dyn std::error::Error>> {
    Ok(OutputInfo::new(
        connector,
        connector,
        LogicalRect::from_raw(spec.0, spec.1, spec.2, spec.3),
        PhysicalSize::from_raw(spec.2 as i32, spec.3 as i32),
        1.0,
        Transform::Normal,
    )?)
}

fn synthetic_frame(width: u32, height: u32, connector: &str) -> Frame {
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
        output: OutputRef::Connector(connector.to_owned()),
        scale: 1.0,
        transform: Transform::Normal,
    }
}

fn build_scene(gpu: &GpuContext, args: &Args) -> Result<Scene, Box<dyn std::error::Error>> {
    let tokens = DesignTokens::default();
    let out0 = output(OUT0, "HDMI-A-1")?;
    let out1 = output(OUT1, "DP-3")?;
    let capture = FrozenCapture {
        outputs: vec![out0.clone(), out1.clone()],
        frames: vec![
            synthetic_frame(OUT0.2 as u32, OUT0.3 as u32, "HDMI-A-1"),
            synthetic_frame(OUT1.2 as u32, OUT1.3 as u32, "DP-3"),
        ],
        cursor: None,
    };
    let mut backdrop = Backdrop::plan(capture, &tokens);
    let layout = OutputLayout::new(vec![out0, out1]);
    let mut core = OverlayCore::new(InputRouter::new(layout, vec![0, 1]));
    flowshot_ui::register_shape_tools(core.editor_mut().registry_mut());
    flowshot_ui::register_text_tool(core.editor_mut().registry_mut());
    flowshot_ui::register_pixelate_tools(core.editor_mut().registry_mut());
    flowshot_ui::register_counter_tool(core.editor_mut().registry_mut());
    flowshot_ui::register_selection_tools(core.editor_mut().registry_mut());
    // The editor frame the magnifier samples: output 0's pixels at logical
    // origin (the storm cursor stays inside output 0).
    let (fw, fh) = (OUT0.2 as u32, OUT0.3 as u32);
    core.install_frame(Some(FramePixels {
        rgba: synthetic_frame(fw, fh, "HDMI-A-1").buffer.data.to_vec(),
        width: fw,
        height: fh,
        scale: 1.0,
        origin: LogicalPoint::from_raw(0.0, 0.0),
    }));
    let mut tools = EditorTools::default();
    tools.editor.magnifier = args.magnifier;
    tools.editor.magnifier_shape = MagnifierShape::Square;
    core.editor_mut().configure(tools);
    core.selection_mut().set_rect(Some(SELECTION));
    seed_objects(&mut core, args.objects);
    if args.panel {
        core.inject_event(SyntheticInput::key_press(
            WindowSlot::new(0),
            KeyCode::Space,
        ));
    }
    // A geometry nudge commits -> the HUD shows (the production trigger).
    core.inject_event(SyntheticInput::key_press(
        WindowSlot::new(0),
        KeyCode::ArrowLeft,
    ));
    core.tick(Instant::now());

    let mut rigs = Vec::new();
    for (index, spec) in [OUT0, OUT1].iter().enumerate() {
        let (width, height) = (spec.2 as u32, spec.3 as u32);
        let mut renderer =
            Renderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
        upload_atlas(gpu, &mut renderer)?;
        backdrop.upload_for(index, &mut renderer, gpu)?;
        backdrop.upload_cursor(&mut renderer, gpu)?;
        let target = renderer.create_offscreen_target(&gpu.device, width, height)?;
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        rigs.push(WindowRig {
            renderer,
            target,
            view,
            width,
            height,
        });
    }
    Ok(Scene {
        core,
        backdrop,
        rigs,
    })
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

/// Seeds `count` committed annotation objects spread across the spanning
/// selection (both monitors): rects, ellipses, arrows, lines, text, counters -
/// the tessellation + text-shaping load of a real editing session.
fn seed_objects(core: &mut OverlayCore, count: usize) {
    let red = SceneColor::new(0xF4, 0x3F, 0x5E, 0xFF);
    let green = SceneColor::new(0x22, 0xC5, 0x5E, 0xFF);
    let amber = SceneColor::new(0xEA, 0xB3, 0x08, 0xFF);
    let white = SceneColor::new(0xFF, 0xFF, 0xFF, 0xFF);
    // Spread object origins across the selection's logical span (600..3800).
    for index in 0..count {
        let slot = index % 6;
        let x = 700.0 + (index as f32) * 180.0;
        let y = 300.0 + (index as f32 % 5.0) * 150.0;
        let object: Box<dyn flowshot_core::scene::ToolObject> = match slot {
            0 => Box::new(RectObject::new(
                SceneRect::new(x, y, 140.0, 90.0),
                red,
                3.0,
                false,
            )),
            1 => Box::new(EllipseObject::new(
                SceneRect::new(x, y, 120.0, 120.0),
                green,
                3.0,
                false,
            )),
            2 => Box::new(ArrowObject::new(
                ScenePoint::new(x, y),
                ScenePoint::new(x + 130.0, y + 80.0),
                amber,
                4.0,
            )),
            3 => Box::new(LineObject::new(
                ScenePoint::new(x, y + 90.0),
                ScenePoint::new(x + 140.0, y),
                red,
                3.0,
            )),
            4 => Box::new(TextObject::new(
                ScenePoint::new(x, y),
                format!("annotation label {index}"),
                18.0,
                white,
            )),
            _ => Box::new(CounterObject::new(
                ScenePoint::new(x, y),
                14.0,
                red,
                index as u32,
            )),
        };
        core.editor_mut().commit_object(object);
    }
}

/// One production frame for window `index`: rebuild the display list, upload
/// the magnifier texture (the live `render_window` order), render offscreen.
/// Returns the four-way timing breakdown in microseconds.
struct FrameTiming {
    list_build: f64,
    tess_shape: f64,
    encode_submit: f64,
}

fn render_window(
    gpu: &GpuContext,
    scene: &mut Scene,
    index: usize,
    now: Instant,
) -> Result<FrameTiming, Box<dyn std::error::Error>> {
    let options = BackdropOptions {
        dim: true,
        cursor_visible: false,
        selection: scene.core.selection().rect(),
    };
    let build_started = Instant::now();
    let frame = build_overlay_frame(
        &scene.core,
        WindowSlot::new(index),
        (scene.rigs[index].width, scene.rigs[index].height),
        Some((&scene.backdrop, &options)),
        now,
    );
    let list_build_us = build_started.elapsed().as_secs_f64() * 1e6;
    let rig = &mut scene.rigs[index];
    if let Some(texture) = frame.magnifier.as_ref() {
        rig.renderer.textures_mut().insert(
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
    let stats = rig.renderer.render(
        &gpu.device,
        &gpu.queue,
        &RenderTarget {
            view: &rig.view,
            width: rig.width,
            height: rig.height,
        },
        &frame.list,
    )?;
    Ok(FrameTiming {
        list_build: list_build_us,
        tess_shape: stats.build_time.as_secs_f64() * 1e6,
        encode_submit: stats.encode_time().as_secs_f64() * 1e6,
    })
}

/// One storm step: route the motion event, then render both windows. Returns
/// the route cost and the combined event-to-submit latency (microseconds).
fn step(
    gpu: &GpuContext,
    scene: &mut Scene,
    input: SyntheticInput,
    now: Instant,
) -> Result<(f64, f64, Vec<FrameTiming>), Box<dyn std::error::Error>> {
    let route_started = Instant::now();
    scene.core.inject_event(input);
    let route_us = route_started.elapsed().as_secs_f64() * 1e6;
    let mut frames = Vec::with_capacity(scene.rigs.len());
    let frame_started = Instant::now();
    for index in 0..scene.rigs.len() {
        frames.push(render_window(gpu, scene, index, now)?);
    }
    let frame_us = frame_started.elapsed().as_secs_f64() * 1e6;
    Ok((route_us, route_us + frame_us, frames))
}

fn warmup(gpu: &GpuContext, scene: &mut Scene) {
    // Prime the glyph atlas, pipelines, and tessellator so the measured storm
    // reflects steady-state shaping (not first-frame rasterization).
    let now = Instant::now();
    for index in 0..10 {
        let x = 1000.0 + f64::from(index) * 2.0;
        let input = SyntheticInput::pointer_moved(WindowSlot::new(0), x, 600.0);
        let _ = step(gpu, scene, input, now);
    }
}

struct Metric {
    name: &'static str,
    samples: Vec<f64>,
}

impl Metric {
    fn push(&mut self, us: f64) {
        self.samples.push(us);
    }
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (pct / 100.0) * (sorted.len() as f64 - 1.0);
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    if low == high {
        return sorted[low];
    }
    let frac = rank - low as f64;
    sorted[low] * (1.0 - frac) + sorted[high] * frac
}

fn storm(gpu: &GpuContext, scene: &mut Scene, args: &Args) -> String {
    let mut route = Metric {
        name: "route (selection+editor recompute)",
        samples: Vec::new(),
    };
    let mut list_build = Metric {
        name: "display-list build (both win)",
        samples: Vec::new(),
    };
    let mut tess_shape = Metric {
        name: "tessellate+text-shape (both win)",
        samples: Vec::new(),
    };
    let mut encode = Metric {
        name: "encode+submit (both win)",
        samples: Vec::new(),
    };
    let mut frame_total = Metric {
        name: "FRAME render (both win, no route)",
        samples: Vec::new(),
    };
    let mut event_to_submit = Metric {
        name: "EVENT-TO-SUBMIT (route+frame)",
        samples: Vec::new(),
    };
    let mut win0 = Metric {
        name: "win0 (1920x1080) frame total",
        samples: Vec::new(),
    };
    let mut win1 = Metric {
        name: "win1 (2560x1440) frame total",
        samples: Vec::new(),
    };

    // The drag: press inside the selection, then oscillate the cursor so the
    // selection keeps changing (changed=true -> full both-window redraw) while
    // staying inside the union bounds.
    let slot0 = WindowSlot::new(0);
    let grab_x = if args.resize { 3790.0 } else { 1000.0 };
    let grab_y = if args.resize { 1190.0 } else { 600.0 };
    scene
        .core
        .inject_event(SyntheticInput::pointer_moved(slot0, grab_x, grab_y));
    scene.core.inject_event(SyntheticInput::pointer_button(
        slot0,
        MouseButton::Left,
        true,
    ));

    let start = Instant::now();
    for event in 0..args.events {
        // Oscillate +/-120px around the grab point (a fast back-and-forth drag).
        let phase = (event as f64 * 0.35).sin();
        let (x, y) = if args.resize {
            (3790.0 + phase * 100.0, 1190.0 + phase * 60.0)
        } else {
            (1000.0 + phase * 120.0, 600.0 + phase * 40.0)
        };
        // The synthetic clock advances ~1ms per event so motion timelines and
        // the HUD see time pass (the live path's per-frame `now`).
        let now = start + Duration::from_micros(event as u64 * 1000);
        let input = SyntheticInput::pointer_moved(slot0, x, y);
        match step(gpu, scene, input, now) {
            Ok((route_us, e2s_us, frames)) => {
                route.push(route_us);
                event_to_submit.push(e2s_us);
                let mut lb = 0.0;
                let mut ts = 0.0;
                let mut en = 0.0;
                for (index, frame) in frames.iter().enumerate() {
                    lb += frame.list_build;
                    ts += frame.tess_shape;
                    en += frame.encode_submit;
                    let total = frame.list_build + frame.tess_shape + frame.encode_submit;
                    match index {
                        0 => win0.push(total),
                        1 => win1.push(total),
                        _ => {}
                    }
                }
                list_build.push(lb);
                tess_shape.push(ts);
                encode.push(en);
                frame_total.push(lb + ts + en);
            }
            Err(error) => {
                eprintln!("storm step {event} failed: {error}");
                break;
            }
        }
    }
    scene.core.inject_event(SyntheticInput::pointer_button(
        slot0,
        MouseButton::Left,
        false,
    ));

    report(
        args,
        &mut [
            &mut route,
            &mut list_build,
            &mut tess_shape,
            &mut encode,
            &mut frame_total,
            &mut event_to_submit,
            &mut win0,
            &mut win1,
        ],
    )
}

fn report(args: &Args, metrics: &mut [&mut Metric]) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "# events={} mode={} panel={} magnifier={} objects={}",
        args.events,
        if args.resize { "resize" } else { "move" },
        args.panel,
        args.magnifier,
        args.objects
    ));
    lines.push(format!(
        "{:<40} {:>9} {:>9} {:>9} {:>9}",
        "metric (microseconds)", "p50", "p95", "p99", "max"
    ));
    let mut acceptance = String::new();
    for metric in metrics.iter() {
        let mut sorted = metric.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p50 = percentile(&sorted, 50.0);
        let p95 = percentile(&sorted, 95.0);
        let p99 = percentile(&sorted, 99.0);
        let max = sorted.last().copied().unwrap_or(0.0);
        lines.push(format!(
            "{:<40} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            metric.name, p50, p95, p99, max
        ));
        if metric.name.starts_with("EVENT-TO-SUBMIT") {
            let p95_ms = p95 / 1000.0;
            let p99_ms = p99 / 1000.0;
            acceptance = format!(
                "# ACCEPTANCE event-to-submit: p95={p95_ms:.3}ms (<=8) p99={p99_ms:.3}ms (<=16) -> {}",
                if p95_ms <= 8.0 && p99_ms <= 16.0 {
                    "PASS"
                } else {
                    "FAIL"
                }
            );
        }
    }
    lines.push(acceptance);
    lines.join("\n") + "\n"
}

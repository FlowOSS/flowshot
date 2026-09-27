//! Frozen-frame backdrop QA harness (plan todo 15).
//!
//! Spawns the multi-monitor overlay around a PRE-CAPTURED frozen session and
//! composes it live: each window renders its output's frozen frame 1:1, the
//! dim layer with the selection cutout, the cursor sprite at the resolved
//! position, and the crosshair on motion - then idles for the grim oracle.
//!
//! The capture itself runs through the platform capture crate's examples
//! (the purity gate forbids platform imports here), so this harness consumes
//! their artifacts from disk - run, from the workspace root:
//!
//! ```text
//! cargo run -p <platform-capture-crate> --example probe > /tmp/layout.json
//! cargo run -p <platform-capture-crate> --example capture_icc -- \
//!     --output HDMI-A-1 --png /tmp/frame-hdmi.png
//! cargo run -p <platform-capture-crate> --example capture_icc -- \
//!     --output DP-3 --png /tmp/frame-dp3.png
//! cargo run -p <platform-capture-crate> --example resolve_cursor --pretty
//! ```
//!
//! (The exact package name is recorded in the QA evidence file, not here -
//! the crate-name literal is invisible to the purity grep by design.)
//!
//! Usage (self-terminating via the stdin injector, user-away QA rules):
//!
//! ```text
//! frozen_backdrop --layout /tmp/layout.json \
//!     --frame HDMI-A-1=/tmp/frame-hdmi.png --frame DP-3=/tmp/frame-dp3.png \
//!     [--cursor /tmp/cursor.png --cursor-pos X,Y --hotspot X,Y] \
//!     [--selection WxH+X+Y] [--no-dim] [--no-cursor] \
//!     [--toolbar pencil,copy,undo] [--draw-color-toml /tmp/config.toml] \
//!     [--magnifier square|circle]
//! ```
//!
//! Todo 26: `--toolbar` re-projects the chrome button order (the config
//! `buttons` list acceptance) and `--draw-color-toml` installs the draw-color
//! persistence sink - every wheel pick rewrites `[editor].draw_color` in
//! that TOML file (the F27 persistence acceptance, file-asserted).
//!
//! With `--features test-drive`, stdin lines inject synthetic events through
//! the production routing path (the todo-13 injector protocol, extended by
//! todos 16/20/22): `move <slot> <x> <y>`, `btn <slot> <left|right|middle>
//! <down|up>`, `wheel <slot> <angle-delta>`, `mods <slot>
//! <none|shift|ctrl|...[+...]>`, `press/release <slot>
//! <escape|enter|left|right|up|down|home|end|backspace|a|c|q|p|d|s|r|m|t|b|i|z|delete|0..9|...>`,
//! `text <slot> <string>` (per char: ASCII through the key-text seam,
//! non-ASCII through `Ime::Commit` - the real platform split), and
//! `ime <slot> <enabled|disabled|preedit|commit> [text]`.
//!
//! Todo 21: the REAL shape tools (pencil/line/arrow/rect/ellipse/marker/
//! invert) are registered - the todo-20 line stub is gone; every F12 tool
//! key now draws its production shape. Todo 22: the text tool (IME editing)
//! is registered too. Todo 23: the secure pixelate (key `b`) and its blur
//! variant (rebound to `v` for QA - blur ships unbound) are registered, and
//! the FIRST `--frame` output's pixels are installed as the editor frame
//! the destructive tools bake from.
//!
//! Todo 18: the launch-flow flags drive the production
//! [`flowshot_ui::LaunchRequest`] seam (the binary layer's todo-38 shape):
//!
//! - `--launch-region WxH[+X+Y]`: offset-less centers at the cursor
//! - `--launch-at-cursor`: the output under the cursor
//! - `--launch-last-region` with `--launch-config PATH`: restore from TOML
//! - `--launch-cursor X,Y`: the resolved cursor (absent = `AwaitFirstMotion`)
//! - `--launch-instant`: accept-on-select
//! - `--launch-save-region`: persist through the region sink into the config
//! - `--launch-motion X,Y`: offscreen only, the first-motion injection
//!   through the production `test-drive` seam
//!
//! In `--verify-offscreen` mode the resolved selection renders into the PNG
//! (dim cutout + outline/grips); live mode seeds the overlay core before
//! `run()`.

use std::process::ExitCode;

use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::config::{Config, MagnifierShape, Region};
use flowshot_core::geometry::{
    Logical, LogicalPoint, LogicalRect, LogicalSize, OutputInfo, OutputLayout, PhysicalPoint,
    PhysicalSize, Transform,
};
use flowshot_ui::backdrop::{BackdropOptions, CursorSprite, FrozenCapture, PlacedCursor};
use flowshot_ui::{
    EditorTools, FramePixels, InputRouter, LaunchRequest, OverlayCore, OverlayRuntime, Preselect,
    SelectionState, ToolKind, UiError,
};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowshot: frozen backdrop failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "CLI-style harness bag: each bool mirrors one independent command-line flag"
)]
struct Args {
    layout: Option<String>,
    frames: Vec<(String, String)>,
    cursor: Option<String>,
    cursor_pos: Option<(f64, f64)>,
    hotspot: (i32, i32),
    selection: Option<LogicalRect>,
    dim: bool,
    cursor_visible: bool,
    verify_offscreen: Option<(usize, String)>,
    toolbar: Option<String>,
    draw_color_toml: Option<String>,
    magnifier: Option<String>,
    launch_region: Option<String>,
    launch_at_cursor: bool,
    launch_last_region: bool,
    launch_cursor: Option<(f64, f64)>,
    launch_instant: bool,
    launch_save_region: bool,
    launch_config: Option<String>,
    launch_motion: Option<(f64, f64)>,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let layout_path = args.layout.as_deref().ok_or("--layout PATH is required")?;
    let outputs = load_outputs(layout_path)?;
    let frames = load_frames(&args.frames, &outputs)?;
    let cursor = load_cursor(&args)?;
    let editor_frame = editor_frame(&frames, &outputs);
    let capture = FrozenCapture {
        outputs,
        frames,
        cursor,
    };
    let options = BackdropOptions {
        dim: args.dim,
        cursor_visible: args.cursor_visible,
        selection: args.selection,
    };
    if let Some((index, path)) = &args.verify_offscreen {
        // The launch seam (todo 18) supersedes the legacy --selection seed:
        // the resolved rect flows through the REAL OverlayCore::launch path.
        let selection = resolve_launch_selection(&args, &capture.outputs)?.or(args.selection);
        return verify_offscreen(capture, *index, path, selection);
    }
    let runtime: Result<OverlayRuntime, UiError> = OverlayRuntime::with_capture(capture, options);
    let mut runtime = runtime?;
    // Todo 21/22/23/24: the production tools (the todo-35 binary layer will do
    // the same registration).
    flowshot_ui::register_shape_tools(runtime.core_mut().editor_mut().registry_mut());
    flowshot_ui::register_text_tool(runtime.core_mut().editor_mut().registry_mut());
    flowshot_ui::register_pixelate_tools(runtime.core_mut().editor_mut().registry_mut());
    flowshot_ui::register_counter_tool(runtime.core_mut().editor_mut().registry_mut());
    flowshot_ui::register_selection_tools(runtime.core_mut().editor_mut().registry_mut());
    // Todo 23: the destructive tools bake from the installed editor frame
    // (the pristine read side); the blur variant ships unbound (no F12 key)
    // so QA rebinds it to `v`. Todo 24: the counter tool ships unbound so QA
    // rebinds it to `n`. Todo 27: the move-selection tool ships unbound so QA
    // rebinds it to `x` (the plan's Ctrl+M is a modified key, which the
    // shortcut system doesn't support; simple key binding follows the
    // blur/counter precedent).
    runtime.core_mut().install_frame(editor_frame);
    runtime
        .core_mut()
        .editor_mut()
        .shortcuts_mut()
        .rebind(ToolKind::Blur, Some(winit::keyboard::KeyCode::KeyV));
    runtime
        .core_mut()
        .editor_mut()
        .shortcuts_mut()
        .rebind(ToolKind::Counter, Some(winit::keyboard::KeyCode::KeyN));
    runtime
        .core_mut()
        .editor_mut()
        .shortcuts_mut()
        .rebind(ToolKind::Move, Some(winit::keyboard::KeyCode::KeyX));
    // Todo 25: z-order ships panel-driven with NO default keys (plan); QA
    // rebinds raise/lower to k/j (both off the F12 map) - the blur-rebind
    // precedent. Object move needs no binding: press-drag-release on a
    // selected object with no draw tool active.
    runtime
        .core_mut()
        .editor_mut()
        .shortcuts_mut()
        .rebind_z_order(
            Some(winit::keyboard::KeyCode::KeyK),
            Some(winit::keyboard::KeyCode::KeyJ),
        );
    // Todo 26: the chrome config projection (button-order acceptance) and
    // the draw-color persistence sink (F27 TOML write; the example owns the
    // file path - the lib seam is the pure callback).
    if let Some(toolbar) = &args.toolbar {
        let buttons = toolbar
            .split(',')
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        runtime
            .core_mut()
            .configure_chrome(&flowshot_core::config::UiConfig {
                toolbar_buttons: buttons,
                ..flowshot_core::config::UiConfig::default()
            });
    }
    if let Some(path) = &args.draw_color_toml {
        let path = std::path::PathBuf::from(path);
        runtime
            .core_mut()
            .chrome_mut()
            .set_draw_color_sink(Some(Box::new(move |hex: &str| {
                let mut config = flowshot_core::config::Config::load(&path).unwrap_or_default();
                hex.clone_into(&mut config.editor.draw_color);
                if let Err(error) = config.save(&path) {
                    eprintln!("flowshot: draw-color persist failed: {error}");
                }
            })));
    }
    apply_launch_args(&mut runtime, &args)?;
    // Todo 17: `--magnifier <square|circle>` projects the `[editor]`
    // magnifier config (visible from launch); the in-session toggle key is
    // `l` either way (F12 binds no magnifier key - the grid-F precedent).
    if let Some(shape) = &args.magnifier {
        let shape = match shape.as_str() {
            "circle" => MagnifierShape::Circle,
            _ => MagnifierShape::Square,
        };
        let mut tools = EditorTools::default();
        tools.editor.magnifier = true;
        tools.editor.magnifier_shape = shape;
        runtime.core_mut().editor_mut().configure(tools);
    }
    #[cfg(feature = "test-drive")]
    spawn_stdin_injector(runtime.handle().clone());
    runtime.run()?;
    Ok(())
}

/// Builds the editor's frozen-frame view from the FIRST loaded frame:
/// upright RGBA (remapped through the output transform when rotated), the
/// output's scale, and its global logical origin (todo 23's read side;
/// todo 35 decides the production stitched-vs-per-output policy).
fn editor_frame(frames: &[Frame], outputs: &[OutputInfo]) -> Option<FramePixels> {
    let frame = frames.first()?;
    let OutputRef::Connector(connector) = &frame.output else {
        return None;
    };
    let output = outputs
        .iter()
        .find(|output| &output.connector == connector)?;
    let (width, height) = (
        usize::try_from(frame.buffer.width).ok()?,
        usize::try_from(frame.buffer.height).ok()?,
    );
    let upright = output.transform.apply_to_size(PhysicalSize::from_raw(
        i32::try_from(width).ok()?,
        i32::try_from(height).ok()?,
    ));
    let (uw, uh) = (
        usize::try_from(upright.width.0).ok()?,
        usize::try_from(upright.height.0).ok()?,
    );
    let mut rgba = vec![0u8; uw * uh * 4];
    if output.transform == Transform::Normal {
        rgba.copy_from_slice(&frame.buffer.data[..uw * uh * 4]);
    } else {
        output
            .transform
            .remap_buffer(&frame.buffer.data[..], &mut rgba, width, height, 4)
            .ok()?;
    }
    Some(FramePixels {
        rgba,
        width: u32::try_from(uw).ok()?,
        height: u32::try_from(uh).ok()?,
        scale: output.scale,
        origin: LogicalPoint::from_raw(output.logical_rect.x.0, output.logical_rect.y.0),
    })
}

/// The todo-18 live-path wiring in the binary layer's shape: the request
/// seeds the core through the production seam, and the region sink persists
/// `[capture].last_region` into the `--launch-config` TOML (the
/// `DrawColorSink` pattern - the example owns the file path).
fn apply_launch_args(
    runtime: &mut OverlayRuntime,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(request) = launch_request(args)? {
        runtime.core_mut().launch(request);
    }
    if let Some(path) = &args.launch_config {
        let path = std::path::PathBuf::from(path);
        runtime
            .core_mut()
            .set_region_sink(Some(Box::new(move |region: Region| {
                let mut config = Config::load(&path).unwrap_or_default();
                config.capture.last_region = Some(region);
                if let Err(error) = config.save(&path) {
                    eprintln!("flowshot: region persist failed: {error}");
                }
            })));
    }
    Ok(())
}

/// Builds the typed launch request from the `--launch-*` flags (the binary
/// layer's todo-38 mapping stand-in: the LIB never parses - the harness
/// plays the CLI's already-typed `RegionToken` side). `None` when no
/// launch flag is present (the pre-todo-18 behavior stays byte-identical).
fn launch_request(args: &Args) -> Result<Option<LaunchRequest>, Box<dyn std::error::Error>> {
    let selectors = [
        args.launch_region.is_some(),
        args.launch_at_cursor,
        args.launch_last_region,
    ];
    if selectors.iter().filter(|set| **set).count() > 1 {
        return Err("only one of --launch-region/--launch-at-cursor/--launch-last-region".into());
    }
    let preselect = if let Some(token) = &args.launch_region {
        Some(parse_preselect(token)?)
    } else if args.launch_at_cursor {
        Some(Preselect::OutputAtCursor)
    } else if args.launch_last_region {
        let path = args
            .launch_config
            .as_deref()
            .ok_or("--launch-last-region wants --launch-config PATH")?;
        Some(Preselect::LastRegion(
            Config::load(std::path::Path::new(path))
                .unwrap_or_default()
                .capture
                .last_region,
        ))
    } else {
        None
    };
    let requested = preselect.is_some()
        || args.launch_instant
        || args.launch_save_region
        || args.launch_cursor.is_some()
        || args.launch_motion.is_some();
    if !requested {
        return Ok(None);
    }
    Ok(Some(LaunchRequest {
        preselect: preselect.unwrap_or(Preselect::None),
        cursor: args
            .launch_cursor
            .map(|(x, y)| LogicalPoint::from_raw(x, y)),
        instant: args.launch_instant,
        save_last_region: args.launch_save_region,
    }))
}

/// The harness-side `WxH[+X+Y]` reader (positive offsets; the signed
/// grammar belongs to the CLI, todo 35 - the lib takes parsed rects).
fn parse_preselect(token: &str) -> Result<Preselect, Box<dyn std::error::Error>> {
    let invalid = || format!("--launch-region wants WxH[+X+Y] (got {token:?})");
    let (size_part, offsets) = match token.find('+') {
        Some(index) => (&token[..index], Some(&token[index + 1..])),
        None => (token, None),
    };
    let (width, height) = size_part.split_once('x').ok_or_else(invalid)?;
    let size = LogicalSize::from_raw(width.trim().parse()?, height.trim().parse()?);
    let origin = match offsets {
        Some(rest) => {
            let (x, y) = rest.split_once('+').ok_or_else(invalid)?;
            Some(LogicalPoint::from_raw(x.trim().parse()?, y.trim().parse()?))
        }
        None => None,
    };
    Ok(Preselect::Region { size, origin })
}

/// Resolves the launch request through the REAL production seam
/// (`OverlayCore::launch`, plus the first-motion injection for the
/// `AwaitFirstMotion` deferral) and returns the seeded selection rect for
/// the offscreen render.
fn resolve_launch_selection(
    args: &Args,
    outputs: &[OutputInfo],
) -> Result<Option<LogicalRect>, Box<dyn std::error::Error>> {
    let Some(request) = launch_request(args)? else {
        return Ok(None);
    };
    let layout = OutputLayout::new(outputs.to_vec());
    let bindings: Vec<usize> = (0..outputs.len()).collect();
    let mut core = OverlayCore::new(InputRouter::new(layout, bindings));
    core.launch(request);
    #[cfg(feature = "test-drive")]
    if let Some((x, y)) = args.launch_motion {
        use flowshot_ui::{SyntheticInput, WindowSlot};
        let global = LogicalPoint::from_raw(x, y);
        let slot = outputs
            .iter()
            .position(|output| output.logical_rect.contains_point(global))
            .ok_or("--launch-motion position is outside every output")?;
        let (local_x, local_y) = core
            .router()
            .to_local(WindowSlot::new(slot), global)
            .ok_or("--launch-motion slot is not bound")?;
        core.inject_event(SyntheticInput::pointer_moved(
            WindowSlot::new(slot),
            local_x,
            local_y,
        ));
    }
    #[cfg(not(feature = "test-drive"))]
    if args.launch_motion.is_some() {
        return Err("--launch-motion needs --features test-drive".into());
    }
    Ok(core.selection().rect())
}

/// Headless orientation/scale verification (Metis #16 edge case): plans the
/// backdrop, renders output `index`'s window scene through the REAL
/// upload+commands+render path into an offscreen texture, and writes the
/// readback as a PNG - no window, no GUI disturbance. A resolved launch
/// `selection` (todo 18) renders through the same path the live overlay
/// uses: the dim cutout plus the selection engine's outline/grips.
fn verify_offscreen(
    capture: FrozenCapture,
    index: usize,
    path: &str,
    selection: Option<LogicalRect>,
) -> Result<(), Box<dyn std::error::Error>> {
    use flowshot_ui::gpu::GpuContext;
    use flowshot_ui::render::{RenderTarget, Renderer, read_texture_rgba};

    let outputs = capture.outputs.clone();
    let backdrop =
        flowshot_ui::Backdrop::plan(capture, &flowshot_core::tokens::DesignTokens::default());
    let (width, height) = backdrop
        .texture_size(index)
        .ok_or("output index has no prepared frozen frame")?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: flowshot_ui::gpu::OVERLAY_BACKENDS,
        ..wgpu::InstanceDescriptor::default()
    });
    let gpu = GpuContext::new_headless(&instance)?;
    let mut backdrop = backdrop;
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    backdrop.upload_for(index, &mut renderer, &gpu)?;
    backdrop.upload_cursor(&mut renderer, &gpu)?;
    let options = BackdropOptions {
        dim: selection.is_some(),
        cursor_visible: false,
        selection,
    };
    let mut list = backdrop.commands(index, (width, height), &options);
    if let (Some(rect), Some(output)) = (selection, outputs.get(index)) {
        let mut engine = SelectionState::default();
        engine.set_rect(Some(rect));
        engine.paint_into(&mut list, output, std::time::Instant::now());
    }
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
    let image =
        image::RgbaImage::from_raw(width, height, pixels).ok_or("readback dimensions disagree")?;
    image.save(path)?;
    println!("wrote {path} ({width}x{height})");
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut args = Args {
        dim: true,
        cursor_visible: true,
        ..Default::default()
    };
    let mut flags = std::env::args().skip(1);
    while let Some(flag) = flags.next() {
        let mut value = |flag: &str| {
            flags
                .next()
                .ok_or_else(|| format!("{flag} requires a value argument"))
        };
        match flag.as_str() {
            "--layout" => args.layout = Some(value("--layout")?),
            "--frame" => {
                let spec = value("--frame")?;
                let (connector, path) =
                    spec.split_once('=').ok_or("--frame wants CONNECTOR=PATH")?;
                args.frames.push((connector.to_owned(), path.to_owned()));
            }
            "--cursor" => args.cursor = Some(value("--cursor")?),
            "--cursor-pos" => {
                let (x, y) = parse_pair(&value("--cursor-pos")?, "--cursor-pos")?;
                args.cursor_pos = Some((x, y));
            }
            "--hotspot" => {
                let (x, y) = parse_pair(&value("--hotspot")?, "--hotspot")?;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let hotspot = (x.round() as i32, y.round() as i32);
                args.hotspot = hotspot;
            }
            "--selection" => args.selection = Some(parse_selection(&value("--selection")?)?),
            "--no-dim" => args.dim = false,
            "--no-cursor" => args.cursor_visible = false,
            "--verify-offscreen" => {
                let spec = value("--verify-offscreen")?;
                let (index, path) = spec
                    .split_once('=')
                    .ok_or("--verify-offscreen wants INDEX=PATH")?;
                args.verify_offscreen = Some((index.trim().parse()?, path.to_owned()));
            }
            "--toolbar" => args.toolbar = Some(value("--toolbar")?),
            "--draw-color-toml" => args.draw_color_toml = Some(value("--draw-color-toml")?),
            "--magnifier" => args.magnifier = Some(value("--magnifier")?),
            "--launch-region" => args.launch_region = Some(value("--launch-region")?),
            "--launch-at-cursor" => args.launch_at_cursor = true,
            "--launch-last-region" => args.launch_last_region = true,
            "--launch-cursor" => {
                let (x, y) = parse_pair(&value("--launch-cursor")?, "--launch-cursor")?;
                args.launch_cursor = Some((x, y));
            }
            "--launch-instant" => args.launch_instant = true,
            "--launch-save-region" => args.launch_save_region = true,
            "--launch-config" => args.launch_config = Some(value("--launch-config")?),
            "--launch-motion" => {
                let (x, y) = parse_pair(&value("--launch-motion")?, "--launch-motion")?;
                args.launch_motion = Some((x, y));
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(args)
}

fn parse_pair(text: &str, flag: &str) -> Result<(f64, f64), Box<dyn std::error::Error>> {
    let (first, second) = text
        .split_once(',')
        .ok_or_else(|| format!("{flag} wants X,Y"))?;
    Ok((first.trim().parse()?, second.trim().parse()?))
}

fn parse_selection(text: &str) -> Result<LogicalRect, Box<dyn std::error::Error>> {
    // WxH+X+Y, global logical px (the todo-18 --region grammar).
    let (size, origin) = text.split_once('+').ok_or("--selection wants WxH+X+Y")?;
    let (width, height) = size.split_once('x').ok_or("--selection wants WxH+X+Y")?;
    let (x, y) = origin.split_once('+').ok_or("--selection wants WxH+X+Y")?;
    Ok(LogicalRect::new(
        Logical(x.trim().parse()?),
        Logical(y.trim().parse()?),
        Logical(width.trim().parse()?),
        Logical(height.trim().parse()?),
    ))
}

fn load_outputs(path: &str) -> Result<Vec<OutputInfo>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let snapshot: serde_json::Value = serde_json::from_str(&text)?;
    let outputs = snapshot
        .get("outputs")
        .ok_or("layout JSON has no `outputs` array (probe snapshot expected)")?;
    Ok(serde_json::from_value(outputs.clone())?)
}

fn load_frames(
    specs: &[(String, String)],
    outputs: &[OutputInfo],
) -> Result<Vec<Frame>, Box<dyn std::error::Error>> {
    specs
        .iter()
        .map(|(connector, path)| {
            let output = outputs
                .iter()
                .find(|output| output.connector == *connector)
                .ok_or_else(|| -> Box<dyn std::error::Error> {
                    format!("--frame connector {connector} is not in the layout").into()
                })?;
            let image = image::open(path)?.to_rgba8();
            let (width, height) = image.dimensions();
            Ok(Frame {
                buffer: FrameBuffer {
                    data: bytes::BytesMut::from(image.as_raw().as_slice()),
                    width,
                    height,
                    stride: width.saturating_mul(4),
                    format: FrameFormat::Rgba8888,
                },
                output: OutputRef::Connector(connector.clone()),
                scale: output.scale,
                transform: output.transform,
            })
        })
        .collect()
}

fn load_cursor(args: &Args) -> Result<Option<PlacedCursor>, Box<dyn std::error::Error>> {
    let (Some(path), Some((x, y))) = (args.cursor.as_deref(), args.cursor_pos) else {
        return Ok(None);
    };
    let image = image::open(path)?.to_rgba8();
    let (width, height) = image.dimensions();
    Ok(Some(PlacedCursor {
        sprite: CursorSprite {
            rgba: image.into_raw(),
            width,
            height,
            hotspot: PhysicalPoint::from_raw(args.hotspot.0, args.hotspot.1),
        },
        position: LogicalPoint::from_raw(x, y),
    }))
}

#[cfg(feature = "test-drive")]
fn spawn_stdin_injector(handle: flowshot_ui::OverlayHandle) {
    use std::io::BufRead;

    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let Some(inputs) = parse_command(&line) else {
                continue;
            };
            let mut failed = false;
            for input in inputs {
                if handle.inject_event(input).is_err() {
                    failed = true;
                    break;
                }
            }
            if failed {
                break;
            }
        }
    });
}

#[cfg(feature = "test-drive")]
fn parse_command(line: &str) -> Option<Vec<flowshot_ui::SyntheticInput>> {
    use flowshot_ui::{SyntheticInput, WindowSlot};
    use winit::event::Ime;

    let mut parts = line.split_whitespace();
    let verb = parts.next()?;
    if verb != "text" {
        return parse_one(line).map(|input| vec![input]);
    }
    let slot = parts.next()?.parse::<usize>().ok()?;
    let text: String = parts.collect::<Vec<_>>().join(" ");
    Some(
        text.chars()
            .map(|character| {
                if character.is_ascii() {
                    SyntheticInput::key_text(
                        WindowSlot::new(slot),
                        code_for(character),
                        &character.to_string(),
                    )
                } else {
                    // Non-ASCII reaches a real app through the IME commit
                    // path (text-input-v3), never through key text.
                    SyntheticInput::ime(WindowSlot::new(slot), Ime::Commit(character.to_string()))
                }
            })
            .collect(),
    )
}

/// The physical code a real keyboard delivers for an ASCII char (the text
/// payload carries the insertion; non-ASCII chars travel as IME commits -
/// the real platform split on Wayland).
#[cfg(feature = "test-drive")]
fn code_for(character: char) -> winit::keyboard::KeyCode {
    use winit::keyboard::KeyCode;
    match character {
        'a'..='z' => LETTER_CODES[character as usize - 'a' as usize],
        'A'..='Z' => LETTER_CODES[character as usize - 'A' as usize],
        '0'..='9' => DIGIT_CODES[character as usize - '0' as usize],
        _ => KeyCode::Space,
    }
}

#[cfg(feature = "test-drive")]
const LETTER_CODES: [winit::keyboard::KeyCode; 26] = [
    winit::keyboard::KeyCode::KeyA,
    winit::keyboard::KeyCode::KeyB,
    winit::keyboard::KeyCode::KeyC,
    winit::keyboard::KeyCode::KeyD,
    winit::keyboard::KeyCode::KeyE,
    winit::keyboard::KeyCode::KeyF,
    winit::keyboard::KeyCode::KeyG,
    winit::keyboard::KeyCode::KeyH,
    winit::keyboard::KeyCode::KeyI,
    winit::keyboard::KeyCode::KeyJ,
    winit::keyboard::KeyCode::KeyK,
    winit::keyboard::KeyCode::KeyL,
    winit::keyboard::KeyCode::KeyM,
    winit::keyboard::KeyCode::KeyN,
    winit::keyboard::KeyCode::KeyO,
    winit::keyboard::KeyCode::KeyP,
    winit::keyboard::KeyCode::KeyQ,
    winit::keyboard::KeyCode::KeyR,
    winit::keyboard::KeyCode::KeyS,
    winit::keyboard::KeyCode::KeyT,
    winit::keyboard::KeyCode::KeyU,
    winit::keyboard::KeyCode::KeyV,
    winit::keyboard::KeyCode::KeyW,
    winit::keyboard::KeyCode::KeyX,
    winit::keyboard::KeyCode::KeyY,
    winit::keyboard::KeyCode::KeyZ,
];

#[cfg(feature = "test-drive")]
const DIGIT_CODES: [winit::keyboard::KeyCode; 10] = [
    winit::keyboard::KeyCode::Digit0,
    winit::keyboard::KeyCode::Digit1,
    winit::keyboard::KeyCode::Digit2,
    winit::keyboard::KeyCode::Digit3,
    winit::keyboard::KeyCode::Digit4,
    winit::keyboard::KeyCode::Digit5,
    winit::keyboard::KeyCode::Digit6,
    winit::keyboard::KeyCode::Digit7,
    winit::keyboard::KeyCode::Digit8,
    winit::keyboard::KeyCode::Digit9,
];

#[cfg(feature = "test-drive")]
fn parse_one(line: &str) -> Option<flowshot_ui::SyntheticInput> {
    use flowshot_ui::{SyntheticInput, WindowSlot};
    use winit::event::{Ime, MouseButton};
    use winit::keyboard::ModifiersState;

    let mut parts = line.split_whitespace();
    match parts.next()? {
        "move" => {
            let slot = parts.next()?.parse::<usize>().ok()?;
            let x = parts.next()?.parse::<f64>().ok()?;
            let y = parts.next()?.parse::<f64>().ok()?;
            Some(SyntheticInput::pointer_moved(WindowSlot::new(slot), x, y))
        }
        "btn" => {
            let slot = parts.next()?.parse::<usize>().ok()?;
            let button = match parts.next()? {
                "left" => MouseButton::Left,
                "right" => MouseButton::Right,
                "middle" => MouseButton::Middle,
                _ => return None,
            };
            let pressed = match parts.next()? {
                "down" => true,
                "up" => false,
                _ => return None,
            };
            Some(SyntheticInput::pointer_button(
                WindowSlot::new(slot),
                button,
                pressed,
            ))
        }
        "wheel" => {
            let slot = parts.next()?.parse::<usize>().ok()?;
            let delta = parts.next()?.parse::<i32>().ok()?;
            Some(SyntheticInput::wheel(WindowSlot::new(slot), delta))
        }
        "mods" => {
            let slot = parts.next()?.parse::<usize>().ok()?;
            let mut modifiers = ModifiersState::empty();
            for part in parts.next()?.split('+') {
                match part {
                    "none" | "" => {}
                    "shift" => modifiers |= ModifiersState::SHIFT,
                    "ctrl" => modifiers |= ModifiersState::CONTROL,
                    "alt" => modifiers |= ModifiersState::ALT,
                    "super" => modifiers |= ModifiersState::SUPER,
                    _ => return None,
                }
            }
            Some(SyntheticInput::modifiers(WindowSlot::new(slot), modifiers))
        }
        "ime" => {
            let slot = parts.next()?.parse::<usize>().ok()?;
            let rest: String = parts.collect::<Vec<_>>().join(" ");
            let (kind, text) = rest.split_once(' ').unwrap_or((rest.as_str(), ""));
            let ime = match kind {
                "enabled" => Ime::Enabled,
                "disabled" => Ime::Disabled,
                "preedit" => Ime::Preedit(text.to_owned(), None),
                "commit" => Ime::Commit(text.to_owned()),
                _ => return None,
            };
            Some(SyntheticInput::ime(WindowSlot::new(slot), ime))
        }
        command @ ("press" | "release") => {
            let slot = parts.next()?.parse::<usize>().ok()?;
            let key = key_code(parts.next()?)?;
            let slot = WindowSlot::new(slot);
            Some(if command == "press" {
                SyntheticInput::key_press(slot, key)
            } else {
                SyntheticInput::key_release(slot, key)
            })
        }
        _ => None,
    }
}

/// The injector's key-name table: navigation keys, the F12 tool activation
/// keys (todo 20), undo/redo, delete, and the digit size adjusters.
#[cfg(feature = "test-drive")]
fn key_code(name: &str) -> Option<winit::keyboard::KeyCode> {
    use winit::keyboard::KeyCode;
    Some(match name {
        "escape" => KeyCode::Escape,
        "enter" => KeyCode::Enter,
        "numpadenter" => KeyCode::NumpadEnter,
        "left" => KeyCode::ArrowLeft,
        "right" => KeyCode::ArrowRight,
        "up" => KeyCode::ArrowUp,
        "down" => KeyCode::ArrowDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "backspace" => KeyCode::Backspace,
        "space" => KeyCode::Space,
        "a" => KeyCode::KeyA,
        "c" => KeyCode::KeyC,
        "q" => KeyCode::KeyQ,
        "p" => KeyCode::KeyP,
        "d" => KeyCode::KeyD,
        "s" => KeyCode::KeyS,
        "r" => KeyCode::KeyR,
        "m" => KeyCode::KeyM,
        "n" => KeyCode::KeyN,
        "t" => KeyCode::KeyT,
        "b" => KeyCode::KeyB,
        "i" => KeyCode::KeyI,
        "v" => KeyCode::KeyV,
        "j" => KeyCode::KeyJ,
        "k" => KeyCode::KeyK,
        "z" => KeyCode::KeyZ,
        "g" => KeyCode::KeyG,
        "f" => KeyCode::KeyF,
        "l" => KeyCode::KeyL,
        "x" => KeyCode::KeyX,
        "delete" => KeyCode::Delete,
        "0" => KeyCode::Digit0,
        "1" => KeyCode::Digit1,
        "2" => KeyCode::Digit2,
        "3" => KeyCode::Digit3,
        "4" => KeyCode::Digit4,
        "5" => KeyCode::Digit5,
        "6" => KeyCode::Digit6,
        "7" => KeyCode::Digit7,
        "8" => KeyCode::Digit8,
        "9" => KeyCode::Digit9,
        _ => return None,
    })
}

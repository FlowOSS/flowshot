//! Headless end-to-end execution harness.
//!
//! Drives the PRODUCTION executor wiring ([`flowshot_daemon::execute`])
//! without a window: the same [`configure_core`] the live overlay session
//! runs, the same input funnel (test-drive injections), the same export
//! implementation ([`flowshot_ui::render_export`] -> `composite_selection`),
//! and the same post-capture pipeline ([`run_post`]: real save files, real
//! `wl-clipboard` offers). This is the "headless execution mode":
//! the minimal honest stand-in for the visible overlay leg (a virtual seat
//! would need a nested compositor, which the plan forbids).
//!
//! Two capture sources:
//! - synthetic: `--layout probe.json --frame CONNECTOR=shot.png` (the
//!   `frozen_backdrop` fixture format);
//! - LIVE INVISIBLE: `--live` runs the real negotiation ladder + protocol
//!   capture against the running compositor (no window is created; capture
//!   is an invisible protocol read under the user-present QA policy).
//!
//! Usage (flags):
//! - `--live` / `--layout PATH` + `--frame CONNECTOR=PATH` (repeatable)
//! - `--inject "SPEC"` (repeatable): `move SLOT X Y`, `down SLOT [BUTTON]`,
//!   `up SLOT [BUTTON]`, `key SLOT NAME`, `text SLOT STRING`,
//!   `mods SLOT ctrl|shift|alt|none`, `wheel SLOT DELTA`
//! - request flags: `--instant`, `--no-edit`, `--region TOKEN`,
//!   `--last-region`, `--hide-cursor`, `--copy`, `--save PATH`,
//!   `--delay MS`
//! - `--color` (eyedropper session), `--config PATH` (config TOML),
//!   `--cursor X,Y` (synthetic-mode cursor), `--hold-clipboard SECS`
//!   (keep the process - and thus the data-control offer - alive)
//!
//! Stable stdout tokens (flow scripts assert on these):
//! `LAYOUT outputs=N`, `SESSION outcome=...`, `EXPORT WxH selection=...`,
//! `SAVED path WxH`, `COPIED`, `PERF capture_ready_us=N frame_ready_us=N
//! total_us=N`, `COLOR hex`.
//!
//! NEVER pass `--pin` here: the pin action spawns a real (visible) window.

#![expect(
    clippy::struct_excessive_bools,
    reason = "CLI-style harness bag: each bool mirrors one independent command-line flag (the frozen_backdrop precedent)"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "harness printouts round logical geometry into integer tokens; out-of-range values degrade to garbage text, never UB"
)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use flowshot_capture::{Frame, FrameBuffer, FrameFormat, OutputRef};
use flowshot_core::Config;
use flowshot_core::geometry::{LogicalPoint, OutputInfo, OutputLayout};
use flowshot_daemon::execute::backend::{open_session, resolve_cursor};
use flowshot_daemon::execute::headless::{HeadlessSession, run_headless};
use flowshot_daemon::execute::overlay::{SessionOutcome, stitched_editor_frame};
use flowshot_daemon::execute::{ExecCtx, post};
use flowshot_daemon::request::CaptureRequest;
use flowshot_ui::{FrozenCapture, SyntheticInput, WindowSlot};
use winit::event::MouseButton;
use winit::keyboard::{KeyCode, ModifiersState};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("e2e_headless failed: {error}");
            let mut source = std::error::Error::source(&*error);
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Args {
    live: bool,
    layout: Option<String>,
    frames: Vec<(String, String)>,
    injects: Vec<String>,
    instant: bool,
    no_edit: bool,
    region: Option<String>,
    last_region: bool,
    hide_cursor: bool,
    copy: bool,
    save: Option<String>,
    delay_ms: u32,
    color: bool,
    config: Option<String>,
    cursor: Option<(f64, f64)>,
    hold_clipboard: Option<u64>,
    rebinds: Vec<(String, String)>,
    cursor_sprite: Option<String>,
    cursor_at: Option<(f64, f64)>,
    hotspot: (i32, i32),
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let started = Instant::now();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (config, config_path) = load_config(args.config.as_deref());
    let request = CaptureRequest {
        delay_ms: args.delay_ms,
        instant: args.instant,
        no_edit: args.no_edit,
        copy: args.copy,
        output: args.save.clone(),
        hide_cursor: args.hide_cursor,
        region: args.region.clone(),
        last_region: args.last_region,
        ..CaptureRequest::default()
    };
    let (frozen, editor_frame, cursor, capture_ready_us) =
        runtime.block_on(capture_source(&args, &request, started))?;
    println!("LAYOUT outputs={}", frozen.outputs.len());
    let inputs = parse_injects(&args.injects)?;
    let mut rebinds = Vec::new();
    for (tool, key) in &args.rebinds {
        rebinds.push((tool_kind(tool)?, key_code(key)?));
    }
    let outcome = run_headless(HeadlessSession {
        frozen,
        editor_frame,
        config: config.clone(),
        config_path: config_path.clone(),
        request: request.clone(),
        cursor,
        color_mode: args.color,
        cursor_visible: !request.hide_cursor && !config.capture.hide_cursor,
        inputs,
        rebinds,
    })?;
    let frame_ready_us = flowshot_daemon::execute::elapsed(started);
    match outcome {
        SessionOutcome::Completed(completion) => {
            println!("SESSION outcome=completed kind={:?}", completion.kind);
            println!(
                "EXPORT {}x{} selection={}x{}+{}+{}",
                completion.image.width,
                completion.image.height,
                completion.selection.width.0.round() as i64,
                completion.selection.height.0.round() as i64,
                completion.selection.x.0.round() as i64,
                completion.selection.y.0.round() as i64,
            );
            let ctx = ExecCtx::one_shot(config_path.clone());
            let executed = runtime.block_on(post::run_post(
                completion,
                &request,
                &config,
                config_path.as_deref(),
                &ctx,
            ))?;
            let flowshot_daemon::execute::ExecOutcome::Done(report) = executed else {
                return Err("post-capture did not complete".into());
            };
            for outcome in &report.outcomes {
                report_outcome(outcome);
            }
            if let Some(path) = &report.saved_path {
                let image = image::open(path)?;
                println!(
                    "SAVED {} {}x{}",
                    path.display(),
                    image.width(),
                    image.height()
                );
            }
        }
        SessionOutcome::ColorPicked(hex) => {
            println!("SESSION outcome=color");
            println!("COLOR {hex}");
        }
        SessionOutcome::Cancelled => println!("SESSION outcome=cancelled"),
        SessionOutcome::Failed(error) => return Err(Box::new(error)),
    }
    println!(
        "PERF capture_ready_us={capture_ready_us} frame_ready_us={frame_ready_us} total_us={}",
        flowshot_daemon::execute::elapsed(started)
    );
    if let Some(secs) = args.hold_clipboard {
        eprintln!("holding the clipboard offer for {secs}s");
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
    Ok(())
}

fn report_outcome(outcome: &flowshot_actions::clipboard::ActionOutcome) {
    use flowshot_actions::clipboard::ActionOutcome;
    match outcome {
        ActionOutcome::Copied => println!("COPIED"),
        ActionOutcome::Saved(path) => println!("ACTION saved {}", path.display()),
        ActionOutcome::PathCopied(path) => println!("ACTION path-copied {}", path.display()),
        ActionOutcome::Uploaded { url } => println!("ACTION uploaded {url}"),
        ActionOutcome::OpenedWith => println!("ACTION opened-with"),
        ActionOutcome::Notified => println!("ACTION notified"),
        ActionOutcome::NotificationGated => println!("ACTION notification-gated"),
        ActionOutcome::PathCopyNoSave => println!("ACTION path-copy-no-save"),
        ActionOutcome::OpenWithNoSave => println!("ACTION open-with-no-save"),
        ActionOutcome::Deferred(action) => println!("ACTION deferred {action:?}"),
        ActionOutcome::Failed { action, message } => {
            println!("ACTION failed {action:?}: {message}");
        }
    }
}

async fn capture_source(
    args: &Args,
    request: &CaptureRequest,
    started: Instant,
) -> Result<
    (
        FrozenCapture,
        Option<flowshot_ui::FramePixels>,
        Option<LogicalPoint>,
        u64,
    ),
    Box<dyn std::error::Error>,
> {
    if args.live {
        let session = open_session().await?;
        let hide_cursor = request.hide_cursor;
        let frozen = flowshot_ui::capture_frozen(session.backend.as_ref(), !hide_cursor).await?;
        let layout = OutputLayout::new(frozen.outputs.clone());
        let editor_frame = stitched_editor_frame(&frozen, session.kind, &layout);
        let cursor = resolve_cursor().await;
        let ready_us = flowshot_daemon::execute::elapsed(started);
        return Ok((frozen, editor_frame, cursor, ready_us));
    }
    let layout_path = args
        .layout
        .as_deref()
        .ok_or("synthetic mode needs --layout PATH (or use --live)")?;
    let outputs = load_outputs(layout_path)?;
    let frames = load_frames(&args.frames, &outputs)?;
    let frozen = FrozenCapture {
        outputs: outputs.clone(),
        frames,
        cursor: placed_cursor(args)?,
    };
    let layout = OutputLayout::new(outputs);
    let editor_frame = stitched_editor_frame(
        &frozen,
        flowshot_capture::BackendKind::ExtImageCopyCapture,
        &layout,
    );
    let cursor = args.cursor.map(|(x, y)| LogicalPoint::from_raw(x, y));
    let ready_us = flowshot_daemon::execute::elapsed(started);
    Ok((frozen, editor_frame, cursor, ready_us))
}

fn load_config(path: Option<&str>) -> (Config, Option<PathBuf>) {
    match path {
        Some(path) => {
            let path = PathBuf::from(path);
            let config = Config::load(&path).unwrap_or_else(|error| {
                eprintln!("config load failed ({error}); using defaults");
                Config::default()
            });
            (config, Some(path))
        }
        None => (Config::default(), None),
    }
}

fn placed_cursor(
    args: &Args,
) -> Result<Option<flowshot_ui::PlacedCursor>, Box<dyn std::error::Error>> {
    let (Some(path), Some((x, y))) = (args.cursor_sprite.as_deref(), args.cursor_at) else {
        return Ok(None);
    };
    let image = image::open(path)?.to_rgba8();
    let (width, height) = image.dimensions();
    Ok(Some(flowshot_ui::PlacedCursor {
        sprite: flowshot_ui::CursorSprite {
            rgba: image.into_raw(),
            width,
            height,
            hotspot: flowshot_core::geometry::PhysicalPoint::from_raw(
                args.hotspot.0,
                args.hotspot.1,
            ),
        },
        position: LogicalPoint::from_raw(x, y),
    }))
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

fn parse_injects(specs: &[String]) -> Result<Vec<SyntheticInput>, Box<dyn std::error::Error>> {
    specs.iter().map(|spec| parse_inject(spec)).collect()
}

fn parse_inject(spec: &str) -> Result<SyntheticInput, Box<dyn std::error::Error>> {
    let mut parts = spec.splitn(3, ' ');
    let command = parts.next().ok_or("empty injection spec")?;
    let invalid =
        || -> Box<dyn std::error::Error> { format!("bad injection spec {spec:?}").into() };
    match command {
        "move" => {
            let slot = slot_arg(parts.next().ok_or_else(invalid)?)?;
            let mut coords = parts
                .next()
                .ok_or_else(invalid)?
                .split(' ')
                .map(str::parse::<f64>);
            let (Some(Ok(x)), Some(Ok(y))) = (coords.next(), coords.next()) else {
                return Err(invalid());
            };
            Ok(SyntheticInput::pointer_moved(slot, x, y))
        }
        "down" | "up" => {
            let slot = slot_arg(parts.next().ok_or_else(invalid)?)?;
            let button = match parts.next().unwrap_or("left") {
                "left" => MouseButton::Left,
                "right" => MouseButton::Right,
                "middle" => MouseButton::Middle,
                _ => return Err(invalid()),
            };
            Ok(SyntheticInput::pointer_button(
                slot,
                button,
                command == "down",
            ))
        }
        "key" => {
            let slot = slot_arg(parts.next().ok_or_else(invalid)?)?;
            let code = key_code(parts.next().ok_or_else(invalid)?)?;
            Ok(SyntheticInput::key_press(slot, code))
        }
        "text" => {
            let slot = slot_arg(parts.next().ok_or_else(invalid)?)?;
            let text = parts.next().ok_or_else(invalid)?;
            Ok(SyntheticInput::key_text(slot, KeyCode::KeyA, text))
        }
        "mods" => {
            let slot = slot_arg(parts.next().ok_or_else(invalid)?)?;
            let modifiers = match parts.next().ok_or_else(invalid)? {
                "ctrl" => ModifiersState::CONTROL,
                "shift" => ModifiersState::SHIFT,
                "alt" => ModifiersState::ALT,
                "none" => ModifiersState::empty(),
                _ => return Err(invalid()),
            };
            Ok(SyntheticInput::modifiers(slot, modifiers))
        }
        "wheel" => {
            let slot = slot_arg(parts.next().ok_or_else(invalid)?)?;
            let delta = parts.next().ok_or_else(invalid)?.parse::<i32>()?;
            Ok(SyntheticInput::wheel(slot, delta))
        }
        _ => Err(invalid()),
    }
}

fn tool_kind(name: &str) -> Result<flowshot_ui::ToolKind, Box<dyn std::error::Error>> {
    use flowshot_ui::ToolKind;
    Ok(match name {
        "pencil" => ToolKind::Pencil,
        "line" => ToolKind::Line,
        "arrow" => ToolKind::Arrow,
        "selection" => ToolKind::Selection,
        "rectangle" => ToolKind::Rectangle,
        "circle" => ToolKind::Circle,
        "marker" => ToolKind::Marker,
        "text" => ToolKind::Text,
        "pixelate" => ToolKind::Pixelate,
        "blur" => ToolKind::Blur,
        "invert" => ToolKind::Invert,
        "counter" => ToolKind::Counter,
        "move" => ToolKind::Move,
        "eyedropper" => ToolKind::Eyedropper,
        other => return Err(format!("unknown tool kind {other:?}").into()),
    })
}

fn slot_arg(text: &str) -> Result<WindowSlot, Box<dyn std::error::Error>> {
    Ok(WindowSlot::new(text.parse::<usize>()?))
}

fn key_code(name: &str) -> Result<KeyCode, Box<dyn std::error::Error>> {
    let code = match name {
        "escape" => KeyCode::Escape,
        "enter" => KeyCode::Enter,
        "space" => KeyCode::Space,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        other => {
            const LETTERS: [KeyCode; 26] = [
                KeyCode::KeyA,
                KeyCode::KeyB,
                KeyCode::KeyC,
                KeyCode::KeyD,
                KeyCode::KeyE,
                KeyCode::KeyF,
                KeyCode::KeyG,
                KeyCode::KeyH,
                KeyCode::KeyI,
                KeyCode::KeyJ,
                KeyCode::KeyK,
                KeyCode::KeyL,
                KeyCode::KeyM,
                KeyCode::KeyN,
                KeyCode::KeyO,
                KeyCode::KeyP,
                KeyCode::KeyQ,
                KeyCode::KeyR,
                KeyCode::KeyS,
                KeyCode::KeyT,
                KeyCode::KeyU,
                KeyCode::KeyV,
                KeyCode::KeyW,
                KeyCode::KeyX,
                KeyCode::KeyY,
                KeyCode::KeyZ,
            ];
            const DIGITS: [KeyCode; 10] = [
                KeyCode::Digit0,
                KeyCode::Digit1,
                KeyCode::Digit2,
                KeyCode::Digit3,
                KeyCode::Digit4,
                KeyCode::Digit5,
                KeyCode::Digit6,
                KeyCode::Digit7,
                KeyCode::Digit8,
                KeyCode::Digit9,
            ];
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c @ ('a'..='z')), None) => LETTERS[(c as u32 - 'a' as u32) as usize],
                (Some(c @ ('0'..='9')), None) => DIGITS[(c as u32 - '0' as u32) as usize],
                _ => return Err(format!("unknown key name {name:?}").into()),
            }
        }
    };
    Ok(code)
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut args = Args::default();
    let mut flags = std::env::args().skip(1);
    while let Some(flag) = flags.next() {
        let mut value = |flag: &str| {
            flags
                .next()
                .ok_or_else(|| format!("{flag} requires a value argument"))
        };
        match flag.as_str() {
            "--live" => args.live = true,
            "--layout" => args.layout = Some(value("--layout")?),
            "--frame" => {
                let spec = value("--frame")?;
                let (connector, path) =
                    spec.split_once('=').ok_or("--frame wants CONNECTOR=PATH")?;
                args.frames.push((connector.to_owned(), path.to_owned()));
            }
            "--inject" => args.injects.push(value("--inject")?),
            "--instant" => args.instant = true,
            "--no-edit" => args.no_edit = true,
            "--region" => args.region = Some(value("--region")?),
            "--last-region" => args.last_region = true,
            "--hide-cursor" => args.hide_cursor = true,
            "--copy" => args.copy = true,
            "--save" => args.save = Some(value("--save")?),
            "--delay" => args.delay_ms = value("--delay")?.parse::<u32>()?,
            "--color" => args.color = true,
            "--config" => args.config = Some(value("--config")?),
            "--cursor" => {
                let spec = value("--cursor")?;
                let (x, y) = spec.split_once(',').ok_or("--cursor wants X,Y")?;
                args.cursor = Some((x.trim().parse()?, y.trim().parse()?));
            }
            "--hold-clipboard" => args.hold_clipboard = Some(value("--hold-clipboard")?.parse()?),
            "--cursor-sprite" => args.cursor_sprite = Some(value("--cursor-sprite")?),
            "--cursor-at" => {
                let spec = value("--cursor-at")?;
                let (x, y) = spec.split_once(',').ok_or("--cursor-at wants X,Y")?;
                args.cursor_at = Some((x.trim().parse()?, y.trim().parse()?));
            }
            "--hotspot" => {
                let spec = value("--hotspot")?;
                let (x, y) = spec.split_once(',').ok_or("--hotspot wants X,Y")?;
                args.hotspot = (x.trim().parse()?, y.trim().parse()?);
            }
            "--rebind" => {
                let spec = value("--rebind")?;
                let (tool, key) = spec.split_once('=').ok_or("--rebind wants TOOL=KEY")?;
                args.rebinds.push((tool.to_owned(), key.to_owned()));
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(args)
}

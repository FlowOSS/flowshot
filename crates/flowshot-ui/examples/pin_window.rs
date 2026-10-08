//! Pin window QA harness.
//!
//! Spawns real pin windows from PNG files or a synthetic marker fixture and
//! idles for the grim/hyprctl oracle. This example is the REFERENCE
//! COMPOSITION of the pin action seam: it plays the binary layer's role
//! by implementing `PinActionSink` over
//! `flowshot_actions::pin` and
//! mirroring pin lifecycle into `flowshot_actions::pin::PinRegistry` (the
//! "pins alive" persistence reason). The Wayland `app_id`
//! (`flowshot-pin`) is applied through the `WindowCustomizer` seam - the
//! lib crate stays platform-pure (the `app_id` deferral pattern).
//!
//! Usage (self-terminating via the stdin injector, user-away QA rules):
//!
//! ```text
//! pin_window [--fixture WxH]... [--png PATH]... [--save-dir DIR]
//! ```
//!
//! With `--features test-drive`, stdin lines inject synthetic events
//! through the production routing path (the injector protocol):
//! `move <pin> <x> <y>`, `wheel <pin> <units>`, `btn <pin> <left|right>
//! <down|up>`, `key <pin> <escape|q|r|0..9>`, `mods <pin>
//! <none|shift|ctrl[+...]>`, `touch <pin> <tid> <started|moved|ended> <x>
//! <y>`, and `exit`. Pins are 1-indexed in spawn order.
//!
//! The synthetic fixture is the oracle's ground truth: gray (128) field
//! with an 8x8 red marker at (w/4, h/4), green at the center, blue at
//! (3w/4, 3h/4) - anchor sampling tracks a marker across zoom.

use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use flowshot_actions::export::NullNotifySink;
use flowshot_actions::pin::{
    PinImage as ActionPinImage, PinRecord, PinRegistry, copy_pin, save_pin,
};
use flowshot_actions::{Clipboard, ExportError};
use flowshot_core::config::SaveConfig;
#[cfg(feature = "test-drive")]
use flowshot_ui::pins::PinInput;
use flowshot_ui::pins::{PinActionSink, PinBehavior, PinId, PinImage, PinRuntime, PinSpec};
#[cfg(feature = "test-drive")]
use winit::event::{MouseButton, TouchPhase};
use winit::keyboard::KeyCode;
#[cfg(feature = "test-drive")]
use winit::keyboard::ModifiersState;

mod common;

use common::session_window_customizer;

struct NoDialog;

impl flowshot_actions::export::FileDialogSink for NoDialog {
    fn pick_save_path(
        &self,
        _default_name: &str,
    ) -> Result<Option<std::path::PathBuf>, ExportError> {
        Ok(None)
    }
}

/// The binary-layer stand-in: UI callback trait -> actions-crate modules.
#[derive(Debug)]
struct ActionBridge {
    registry: Arc<Mutex<PinRegistry>>,
    clipboard: Clipboard,
    save_config: SaveConfig,
}

impl PinActionSink for ActionBridge {
    fn copy(
        &self,
        snapshot: flowshot_ui::pins::PinSnapshot,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let image = ActionPinImage {
            width: snapshot.width,
            height: snapshot.height,
            rgba: snapshot.rgba,
        };
        copy_pin(&image, &self.save_config, &self.clipboard)?;
        println!(
            "SINK copy pin={} {}x{}",
            snapshot.id.raw(),
            image.width,
            image.height
        );
        Ok(())
    }

    fn save(
        &self,
        snapshot: flowshot_ui::pins::PinSnapshot,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let image = ActionPinImage {
            width: snapshot.width,
            height: snapshot.height,
            rgba: snapshot.rgba,
        };
        let path = save_pin(&image, &self.save_config, &NoDialog, &NullNotifySink)?;
        println!(
            "SINK save pin={} path={}",
            snapshot.id.raw(),
            path.display()
        );
        Ok(())
    }

    fn pin_closed(&self, id: PinId) {
        let mut registry = lock(&self.registry);
        registry.unregister(id.raw());
        println!("REGISTRY closed pin={} alive={}", id.raw(), registry.len());
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Headless ground-truth render (the verify-offscreen pattern):
/// drives the PRODUCTION state machine (key injections), builds the frame
/// with the production `frame_list`, renders through the production
/// `Renderer` into an offscreen target, and reads the surface pixels back.
/// Whatever the live grim sample shows beyond these values is compositor
/// contribution (decoration/blur), not pin rendering.
/// Drives the production state machine to the verify pose (rotations x R,
/// opacity digit key) and composes the upload buffer.
fn verify_state(
    width: u32,
    height: u32,
    rotations: u32,
    tenths: u8,
) -> Result<(flowshot_ui::pins::PinState, Vec<u8>), String> {
    use flowshot_ui::pins::{PinInput, PinState};

    let image = marker_fixture(width, height);
    let mut state = PinState::new((width, height), (1920, 1080), 1.0, PinBehavior::default());
    let t0 = std::time::Instant::now();
    let key = |code| PinInput::Key {
        code,
        pressed: true,
        repeat: false,
    };
    for _ in 0..rotations {
        state.on_input(&key(KeyCode::KeyR), t0);
    }
    if tenths != 10 {
        let digits = [
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
        let index = usize::from(tenths % 10);
        state.on_input(&key(digits[index]), t0);
    }
    let buffer = image
        .composed(state.rotation(), state.opacity())
        .map_err(|e| e.to_string())?;
    Ok((state, buffer))
}

fn verify_offscreen(out: &str, width: u32, height: u32, rotations: u32, tenths: u8) -> ExitCode {
    use flowshot_ui::gpu::{GpuContext, new_instance};
    use flowshot_ui::pins::{TEXTURE_ID, frame_list, image_rect};
    use flowshot_ui::render::{RenderTarget, Renderer, RgbaImage, read_texture_rgba};

    let (state, buffer) = match verify_state(width, height, rotations, tenths) {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("verify state failed: {error}");
            return ExitCode::from(1);
        }
    };
    let (tw, th) = state.target_window();
    let instance = new_instance();
    let Ok(gpu) = GpuContext::new_headless(&instance) else {
        eprintln!("headless GPU init failed");
        return ExitCode::from(1);
    };
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let (iw, ih) = state.image_size();
    if let Err(error) = renderer.textures_mut().insert(
        &gpu.device,
        &gpu.queue,
        TEXTURE_ID,
        &RgbaImage {
            width: iw,
            height: ih,
            data: &buffer,
        },
    ) {
        eprintln!("texture insert failed: {error}");
        return ExitCode::from(1);
    }
    let list = frame_list(&state, std::time::Instant::now());
    let Ok(target) = renderer.create_offscreen_target(&gpu.device, tw, th) else {
        eprintln!("offscreen target failed");
        return ExitCode::from(1);
    };
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let render_target = RenderTarget {
        view: &view,
        width: tw,
        height: th,
    };
    if let Err(error) = renderer.render(&gpu.device, &gpu.queue, &render_target, &list) {
        eprintln!("render failed: {error}");
        return ExitCode::from(1);
    }
    let Ok(pixels) = read_texture_rgba(&gpu.device, &gpu.queue, &target, tw, th) else {
        eprintln!("readback failed");
        return ExitCode::from(1);
    };
    if let Err(error) = image::save_buffer(out, &pixels, tw, th, image::ColorType::Rgba8) {
        eprintln!("png write failed: {error}");
        return ExitCode::from(1);
    }
    // Sample points: the red marker center (rotation-mapped) and a field
    // point, both in window coordinates (margin + image point * scale).
    let rect = image_rect(&state, std::time::Instant::now());
    let scale = state.scale();
    let marker_img = match rotations % 4 {
        0 => (104.0, 79.0),
        1 => (220.0, 104.0),
        2 => (295.0, 220.0),
        _ => (79.0, 195.0),
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sample coordinates are small non-negative window offsets"
    )]
    let sample = |ix: f64, iy: f64| -> [u8; 4] {
        let x = (f64::from(rect.origin.x) + ix * scale).round().max(0.0) as usize;
        let y = (f64::from(rect.origin.y) + iy * scale).round().max(0.0) as usize;
        let at = ((y * usize::try_from(tw).unwrap_or(usize::MAX)) + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    };
    println!(
        "VERIFY window=({tw},{th}) scale={scale:.6} tenths={} marker={:?} field={:?}",
        state.opacity_tenths(),
        sample(marker_img.0, marker_img.1),
        sample(30.0, 30.0)
    );
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(out) = arg_value(&args, "--verify-offscreen") {
        let dims = arg_value(&args, "--fixture").unwrap_or_else(|| "400x300".to_owned());
        let (w, h) = dims.split_once('x').unwrap_or(("400", "300"));
        let rot = arg_value(&args, "--rot").unwrap_or_else(|| "0".to_owned());
        let tenths = arg_value(&args, "--tenths").unwrap_or_else(|| "10".to_owned());
        return verify_offscreen(
            &out,
            w.parse().unwrap_or(400),
            h.parse().unwrap_or(300),
            rot.parse().unwrap_or(0),
            tenths.parse().unwrap_or(10),
        );
    }
    let save_dir = arg_value(&args, "--save-dir").unwrap_or_else(|| "/tmp".to_owned());
    let specs = build_specs(&args);
    if specs.is_empty() {
        eprintln!("usage: pin_window [--fixture WxH | --png PATH]... [--save-dir DIR]");
        return ExitCode::from(2);
    }

    let registry = Arc::new(Mutex::new(PinRegistry::new()));
    {
        let mut registry = lock(&registry);
        for spec in &specs {
            registry.register(PinRecord {
                id: spec.id.raw(),
                width: spec.image.width,
                height: spec.image.height,
                opened_at: std::time::SystemTime::now(),
            });
        }
        println!("REGISTRY alive={}", registry.len());
    }
    let clipboard = match Clipboard::for_session() {
        Ok(clipboard) => clipboard,
        Err(error) => {
            eprintln!("no clipboard session available: {error}");
            return ExitCode::from(1);
        }
    };
    let bridge = Arc::new(ActionBridge {
        registry: Arc::clone(&registry),
        clipboard,
        save_config: SaveConfig {
            path: save_dir,
            path_fixed: true,
            filename_pattern: "flowshot-pin".to_owned(),
            ..SaveConfig::default()
        },
    });
    let runtime = match PinRuntime::new(specs, Some(bridge), PinBehavior::default()) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("pin host startup failed: {error}");
            return ExitCode::from(1);
        }
    };
    // The binary layer's platform hook: the session's shell-facing name the
    // compositor/WM reports as the window class (the daemon wires the real
    // binary through execute::window::session_window_customizer).
    let runtime =
        runtime.with_window_customizer(session_window_customizer("flowshot-pin", "FlowShot Pin"));
    #[cfg(feature = "test-drive")]
    spawn_stdin_injector(runtime.handle().clone());
    match runtime.run() {
        Ok(()) => {
            println!("REGISTRY final alive={}", lock(&registry).len());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("pin host exited with error: {error}");
            ExitCode::from(1)
        }
    }
}

/// Parses `--fixture WxH` / `--png PATH` arguments into pin specs
/// (1-indexed ids in spawn order).
fn build_specs(args: &[String]) -> Vec<PinSpec> {
    let mut specs: Vec<PinSpec> = Vec::new();
    let mut index = 0u64;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--fixture" => {
                let Some(dims) = iter.next() else { continue };
                let Some((w, h)) = dims.split_once('x') else {
                    continue;
                };
                let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) else {
                    continue;
                };
                index += 1;
                specs.push(PinSpec {
                    id: PinId::new(index),
                    image: marker_fixture(w, h),
                });
            }
            "--png" => {
                let Some(path) = iter.next() else { continue };
                match load_png(path) {
                    Ok(image) => {
                        index += 1;
                        specs.push(PinSpec {
                            id: PinId::new(index),
                            image,
                        });
                    }
                    Err(error) => eprintln!("skipping {path}: {error}"),
                }
            }
            "--save-dir" => {
                iter.next();
            }
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    specs
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|at| args.get(at + 1))
        .cloned()
}

/// Gray field with 8x8 marker squares at the quarter points and center -
/// the anchor-sampling ground truth (see the module header).
fn marker_fixture(width: u32, height: u32) -> PinImage {
    let mut rgba = vec![128u8; usize::try_from(width * height * 4).unwrap_or(usize::MAX)];
    for pixel in rgba.as_chunks_mut::<4>().0.iter_mut() {
        pixel[3] = 255; // opaque field (alpha=128 gray was the QA tint bug)
    }
    let put = |rgba: &mut [u8], x: u32, y: u32, color: [u8; 3]| {
        for dy in 0..8u32 {
            for dx in 0..8u32 {
                let px = x + dx;
                let py = y + dy;
                if px >= width || py >= height {
                    continue;
                }
                let at = usize::try_from((py * width + px) * 4).unwrap_or(usize::MAX);
                rgba[at..at + 3].copy_from_slice(&color);
                rgba[at + 3] = 255;
            }
        }
    };
    put(&mut rgba, width / 4, height / 4, [255, 0, 0]);
    put(&mut rgba, width / 2, height / 2, [0, 255, 0]);
    put(&mut rgba, width * 3 / 4, height * 3 / 4, [0, 0, 255]);
    PinImage::new(width, height, rgba).unwrap_or_else(|e| panic!("fixture: {e}"))
}

fn load_png(path: &str) -> Result<PinImage, String> {
    let decoded = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let (width, height) = decoded.dimensions();
    PinImage::new(width, height, decoded.into_raw()).map_err(|e| e.to_string())
}

#[cfg(feature = "test-drive")]
fn spawn_stdin_injector(handle: flowshot_ui::pins::PinHandle) {
    use std::io::BufRead;

    std::thread::spawn(move || {
        eprintln!("INJECTOR ONLINE");
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            eprintln!("INJECT LINE: {line}");
            let command = line.split_whitespace().next().unwrap_or_default();
            if command == "exit" {
                let _ = handle.request_exit();
                break;
            }
            let Some((id, input)) = parse_command(&line) else {
                continue;
            };
            if handle.inject_event(id, input).is_err() {
                break;
            }
        }
    });
}

#[cfg(feature = "test-drive")]
fn parse_command(line: &str) -> Option<(PinId, PinInput)> {
    let mut parts = line.split_whitespace();
    let command = parts.next()?;
    let pin = PinId::new(parts.next()?.parse::<u64>().ok()?);
    let input = match command {
        "move" => PinInput::CursorMoved {
            x: parts.next()?.parse().ok()?,
            y: parts.next()?.parse().ok()?,
        },
        "wheel" => PinInput::Wheel {
            units: parts.next()?.parse().ok()?,
        },
        "btn" => PinInput::Button {
            button: match parts.next()? {
                "left" => MouseButton::Left,
                "right" => MouseButton::Right,
                "middle" => MouseButton::Middle,
                _ => return None,
            },
            pressed: match parts.next()? {
                "down" => true,
                "up" => false,
                _ => return None,
            },
        },
        "key" => PinInput::Key {
            code: match parts.next()? {
                "escape" => KeyCode::Escape,
                "q" => KeyCode::KeyQ,
                "r" => KeyCode::KeyR,
                digit @ ("0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
                    match digit {
                        "0" => KeyCode::Digit0,
                        "1" => KeyCode::Digit1,
                        "2" => KeyCode::Digit2,
                        "3" => KeyCode::Digit3,
                        "4" => KeyCode::Digit4,
                        "5" => KeyCode::Digit5,
                        "6" => KeyCode::Digit6,
                        "7" => KeyCode::Digit7,
                        "8" => KeyCode::Digit8,
                        _ => KeyCode::Digit9,
                    }
                }
                _ => return None,
            },
            pressed: true,
            repeat: false,
        },
        "mods" => {
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
            PinInput::Modifiers(modifiers)
        }
        "touch" => PinInput::Touch {
            id: parts.next()?.parse().ok()?,
            phase: match parts.next()? {
                "started" => TouchPhase::Started,
                "moved" => TouchPhase::Moved,
                "ended" => TouchPhase::Ended,
                "cancelled" => TouchPhase::Cancelled,
                _ => return None,
            },
            x: parts.next()?.parse().ok()?,
            y: parts.next()?.parse().ok()?,
        },
        "enter" => PinInput::CursorEntered,
        "leave" => PinInput::CursorLeft,
        _ => return None,
    };
    Some((pin, input))
}

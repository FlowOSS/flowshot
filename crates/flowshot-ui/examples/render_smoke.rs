//! Renderer smoke QA harness (plan todo 14).
//!
//! One borderless-fullscreen transparent window on the primary monitor,
//! rendering the acceptance scene through [`flowshot_ui::render::Renderer`]:
//! frozen-frame texture, dim layer with selection cutout, selection outline,
//! a diagonal (antialiasing evidence), rounded toolbar rect with a token drop
//! shadow, and `FlowShot` text. Runs at scales 1.0 AND 2.0, 300 timed frames
//! each (avg must stay under 8 ms at native res).
//!
//! Self-terminating and timeout-bounded (user-away live-QA rules): each
//! scale phase renders 300 frames, then holds the presented frame on screen
//! for a grim oracle window, then exits 0. The current phase is mirrored to
//! `/tmp/flowshot-smoke-phase.txt` so the QA script can grim at the right
//! moment; stdin EOF also exits early (the todo-13 injector pattern).
//!
//! All scene colors/radii/shadow/typography come from
//! `flowshot_core::tokens::DesignTokens`; geometry derives from token values
//! (the toolbar height uses the F27 `buttonBaseSize = font_line * 2.2`
//! formula). The frame-texture stand-in is a checkerboard of the token
//! accent/contrast colors.

// Physical pixel dimensions and frame counts are far below f32/f64 exact
// integer ranges (2^24); `as` narrowing cannot lose precision here.
#![allow(clippy::cast_precision_loss)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::UiError;
use flowshot_ui::gpu::{GpuContext, OVERLAY_BACKENDS, configure_overlay_surface};
use flowshot_ui::render::{
    Color, DisplayList, Point, Rect, RenderTarget, Renderer, RgbaImage, ShadowSpec, Shape,
    TextCommand, TextureId,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId, WindowLevel};

const FRAME_TEXTURE: TextureId = TextureId::new(1);
const TIMED_FRAMES: u32 = 300;
const HOLD: Duration = Duration::from_secs(6);
const PHASE_MARKER: &str = "/tmp/flowshot-smoke-phase.txt";

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
            eprintln!("flowshot: render smoke failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), UiError> {
    let event_loop = EventLoop::<()>::new()?;
    let mut app = SmokeApp::new();
    event_loop.run_app(&mut app)?;
    app.take_fatal_error()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Render { scale_index: usize },
    Hold { scale_index: usize, until: Instant },
    Finished,
}

const SCALES: [f32; 2] = [1.0, 2.0];

#[derive(Debug)]
struct SmokeApp {
    tokens: DesignTokens,
    phase: Phase,
    frames: u32,
    frame_times_ms: Vec<f64>,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    gpu: Option<GpuContext>,
    renderer: Option<Renderer>,
    list: DisplayList,
    size: (u32, u32),
    monitor_name: String,
    fatal_error: Option<UiError>,
}

impl SmokeApp {
    fn new() -> Self {
        Self {
            tokens: DesignTokens::default(),
            phase: Phase::Render { scale_index: 0 },
            frames: 0,
            frame_times_ms: Vec::new(),
            window: None,
            surface: None,
            config: None,
            gpu: None,
            renderer: None,
            list: DisplayList::new(),
            size: (0, 0),
            monitor_name: String::from("primary"),
            fatal_error: None,
        }
    }

    fn take_fatal_error(&mut self) -> Result<(), UiError> {
        match self.fatal_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn fail(&mut self, target: &ActiveEventLoop, error: UiError) {
        tracing::error!(%error, "smoke run failed; exiting");
        self.fatal_error = Some(error);
        target.exit();
    }

    fn spawn(&mut self, target: &ActiveEventLoop) -> Result<(), UiError> {
        let monitor = target
            .primary_monitor()
            .or_else(|| target.available_monitors().next());
        let Some(monitor) = monitor else {
            return Err(UiError::NoMonitors);
        };
        self.monitor_name = monitor.name().unwrap_or_else(|| "smoke-monitor".to_owned());
        let attributes = Window::default_attributes()
            .with_title("FlowShot")
            .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))))
            .with_transparent(true)
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        let monitor_name = self.monitor_name.clone();
        let window = Arc::new(target.create_window(attributes).map_err(|source| {
            UiError::WindowCreation {
                monitor: monitor_name.clone(),
                source,
            }
        })?);
        window.set_cursor_visible(false);
        let size = window.inner_size();
        self.size = (size.width.max(1), size.height.max(1));

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: OVERLAY_BACKENDS,
            ..wgpu::InstanceDescriptor::default()
        });
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(|source| UiError::SurfaceCreation {
                monitor: self.monitor_name.clone(),
                source,
            })?;
        let gpu = GpuContext::new(&instance, &surface)?;
        let config = configure_overlay_surface(
            &surface,
            &gpu.adapter,
            &gpu.device,
            self.size.0,
            self.size.1,
            &self.monitor_name,
        )?;
        let mut renderer = Renderer::new(&gpu.device, &gpu.queue, config.format);
        let data = frame_texture_data(&self.tokens, 512);
        renderer.textures_mut().insert(
            &gpu.device,
            &gpu.queue,
            FRAME_TEXTURE,
            &RgbaImage {
                width: 512,
                height: 512,
                data: &data,
            },
        )?;
        tracing::info!(
            monitor = %self.monitor_name,
            width = self.size.0,
            height = self.size.1,
            "smoke window mapped"
        );
        self.window = Some(window);
        self.surface = Some(surface);
        self.config = Some(config);
        self.gpu = Some(gpu);
        self.renderer = Some(renderer);
        self.rebuild_scene();
        write_phase("render1");
        Ok(())
    }

    fn current_scale(&self) -> f32 {
        match self.phase {
            Phase::Render { scale_index } | Phase::Hold { scale_index, .. } => {
                SCALES.get(scale_index).copied().unwrap_or(1.0)
            }
            Phase::Finished => 1.0,
        }
    }

    fn rebuild_scene(&mut self) {
        let scale = self.current_scale();
        self.list = smoke_scene(&self.tokens, self.size, scale);
    }

    fn render_frame(&mut self) {
        let (Some(gpu), Some(surface), Some(config), Some(renderer)) = (
            self.gpu.as_ref(),
            self.surface.as_ref(),
            self.config.as_ref(),
            self.renderer.as_mut(),
        ) else {
            return;
        };
        let frame = match surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                surface.configure(&gpu.device, config);
                return;
            }
            Err(wgpu::SurfaceError::Timeout) => return,
            Err(wgpu::SurfaceError::OutOfMemory) => {
                let error = UiError::OutOfMemory;
                self.fatal_error = Some(error);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let target = RenderTarget {
            view: &view,
            width: config.width,
            height: config.height,
        };
        match renderer.render(&gpu.device, &gpu.queue, &target, &self.list) {
            Ok(stats) => {
                self.frame_times_ms
                    .push(stats.cpu_time.as_secs_f64() * 1000.0);
                self.frames += 1;
            }
            Err(error) => {
                tracing::error!(%error, "smoke frame failed");
                self.fatal_error = Some(error);
            }
        }
        frame.present();
    }

    fn finish_timing(&mut self, scale_index: usize) {
        let count = self.frame_times_ms.len();
        if count > 0 {
            let sum: f64 = self.frame_times_ms.iter().sum();
            let avg = sum / count as f64;
            let max = self.frame_times_ms.iter().copied().fold(f64::MIN, f64::max);
            let scale = SCALES.get(scale_index).copied().unwrap_or(1.0);
            tracing::info!(
                scale,
                frames = count,
                avg_ms = format!("{avg:.3}"),
                max_ms = format!("{max:.3}"),
                width = self.size.0,
                height = self.size.1,
                "frame timing summary"
            );
            println!(
                "FRAME_TIME scale={scale} frames={count} avg_ms={avg:.3} max_ms={max:.3} size={}x{}",
                self.size.0, self.size.1
            );
        }
        self.frame_times_ms.clear();
        self.frames = 0;
    }
}

impl ApplicationHandler<()> for SmokeApp {
    fn resumed(&mut self, target: &ActiveEventLoop) {
        if self.window.is_some() || self.fatal_error.is_some() {
            return;
        }
        if let Err(error) = self.spawn(target) {
            self.fail(target, error);
        }
    }

    fn window_event(&mut self, target: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => target.exit(),
            WindowEvent::KeyboardInput { event: key, .. } => {
                if key.state == ElementState::Pressed
                    && let PhysicalKey::Code(KeyCode::Escape) = key.physical_key
                {
                    target.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return;
                }
                self.size = (size.width, size.height);
                let reconfigured = {
                    let (Some(gpu), Some(surface)) = (self.gpu.as_ref(), self.surface.as_ref())
                    else {
                        return;
                    };
                    configure_overlay_surface(
                        surface,
                        &gpu.adapter,
                        &gpu.device,
                        size.width,
                        size.height,
                        &self.monitor_name,
                    )
                };
                match reconfigured {
                    Ok(new_config) => {
                        self.config = Some(new_config);
                        self.rebuild_scene();
                    }
                    Err(error) => self.fail(target, error),
                }
            }
            WindowEvent::RedrawRequested => {
                if !matches!(self.phase, Phase::Render { .. }) {
                    return;
                }
                self.render_frame();
                if self.fatal_error.is_some() {
                    target.exit();
                    return;
                }
                if self.frames >= TIMED_FRAMES {
                    let scale_index = match self.phase {
                        Phase::Render { scale_index } => scale_index,
                        Phase::Hold { .. } | Phase::Finished => 0,
                    };
                    self.finish_timing(scale_index);
                    if scale_index + 1 < SCALES.len() {
                        self.phase = Phase::Hold {
                            scale_index,
                            until: Instant::now() + HOLD,
                        };
                        write_phase(if scale_index == 0 { "hold1" } else { "hold2" });
                        target.set_control_flow(ControlFlow::wait_duration(self.hold_remaining()));
                    } else {
                        self.phase = Phase::Finished;
                        write_phase("done");
                        target.exit();
                    }
                } else if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, target: &ActiveEventLoop) {
        match self.phase {
            Phase::Hold { scale_index, until } => {
                if Instant::now() >= until {
                    let next = scale_index + 1;
                    self.phase = Phase::Render { scale_index: next };
                    self.rebuild_scene();
                    write_phase(if next == 1 { "render2" } else { "render" });
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                } else {
                    target.set_control_flow(ControlFlow::wait_duration(self.hold_remaining()));
                }
            }
            Phase::Render { .. } => target.set_control_flow(ControlFlow::Poll),
            Phase::Finished => target.exit(),
        }
    }

    fn exiting(&mut self, _target: &ActiveEventLoop) {
        write_phase("exited");
        tracing::info!("render smoke exiting");
    }
}

impl SmokeApp {
    fn hold_remaining(&self) -> Duration {
        match self.phase {
            Phase::Hold { until, .. } => until.saturating_duration_since(Instant::now()),
            Phase::Render { .. } | Phase::Finished => Duration::ZERO,
        }
    }
}

fn write_phase(marker: &str) {
    println!("PHASE {marker}");
    if let Err(error) = std::fs::write(PHASE_MARKER, marker) {
        tracing::debug!(%error, "phase marker write failed");
    }
}

/// The acceptance scene, physical px, every visual value token-derived.
fn smoke_scene(tokens: &DesignTokens, size: (u32, u32), scale: f32) -> DisplayList {
    let mut list = DisplayList::new();
    let (w, h) = (size.0 as f32, size.1 as f32);
    let window = Rect::from_parts(0.0, 0.0, w, h);
    let accent = token_color(&tokens.palette.accent);
    let contrast = token_color(&tokens.palette.contrast);
    let dim = Color::dim_from_palette(&tokens.palette).unwrap_or(contrast.with_alpha8(128));
    let base = tokens.spacing.base as f32 * scale;
    let large = tokens.spacing.large as f32 * scale;

    // Frozen-frame stand-in.
    list.image(FRAME_TEXTURE, window, None);
    // Dim layer with the selection cutout (even-odd).
    let selection = Rect::from_parts(w * 0.3, h * 0.2, w * 0.4, h * 0.4);
    list.dim(window, vec![selection], dim);
    // Selection outline + diagonal (antialiasing evidence on the diagonal).
    list.stroke(
        Shape::Rect {
            rect: selection,
            radius: 0.0,
        },
        base,
        accent,
    );
    list.stroke(
        Shape::Line {
            from: Point::new(selection.origin.x, selection.origin.y),
            to: Point::new(selection.right(), selection.bottom()),
        },
        base,
        accent,
    );
    // Toolbar: token shadow, rounded rect (radii.large), height from the F27
    // buttonBaseSize formula (font line * 2.2).
    let font_size = tokens.typography.base_size as f32 * scale;
    let line_height = font_size * 1.2;
    let toolbar_height = line_height * 2.2;
    let toolbar = Rect::from_parts(
        selection.origin.x,
        selection.bottom() + large,
        toolbar_height * 4.0,
        toolbar_height,
    );
    let radius = tokens.radii.large as f32 * scale;
    if let Some(spec) = ShadowSpec::from_token(&tokens.shadows.medium, scale) {
        list.shadow(toolbar, radius, spec);
    }
    list.fill(
        Shape::Rect {
            rect: toolbar,
            radius,
        },
        contrast,
    );
    list.stroke(
        Shape::Rect {
            rect: toolbar,
            radius,
        },
        base * 0.5,
        accent,
    );
    list.text(TextCommand {
        position: Point::new(
            toolbar.origin.x + large,
            toolbar.origin.y + (toolbar_height - line_height) * 0.5,
        ),
        text: "FlowShot".to_owned(),
        font_size,
        line_height,
        color: accent,
        family: Some(tokens.typography.family.clone()),
        max_width: None,
    });
    list
}

fn token_color(hex: &str) -> Color {
    // Malformed tokens fall back to a loud diagnostic magenta (the same
    // failure signal the renderer uses for missing textures).
    Color::from_hex_token(hex).unwrap_or_else(|| Color::from_rgba8(255, 0, 255, 255))
}

/// Checkerboard stand-in for a frozen capture frame, built from the token
/// accent/contrast colors (8x8 cells).
fn frame_texture_data(tokens: &DesignTokens, dim: usize) -> Vec<u8> {
    let accent = hex_bytes(&tokens.palette.accent);
    let contrast = hex_bytes(&tokens.palette.contrast);
    let cell = (dim / 8).max(1);
    let mut data = Vec::with_capacity(dim * dim * 4);
    for y in 0..dim {
        for x in 0..dim {
            let (r, g, b) = if (x / cell + y / cell).is_multiple_of(2) {
                contrast
            } else {
                accent
            };
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    data
}

fn hex_bytes(hex: &str) -> (u8, u8, u8) {
    let digits = hex.strip_prefix('#').unwrap_or("");
    let byte = |index: usize| {
        digits
            .get(index..index + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .unwrap_or(0)
    };
    (byte(0), byte(2), byte(4))
}

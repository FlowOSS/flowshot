#![allow(clippy::cast_precision_loss, clippy::unwrap_used)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::UiError;
use flowshot_ui::gpu::{GpuContext, OVERLAY_BACKENDS, configure_overlay_surface};
use flowshot_ui::render::{
    Color, DisplayList, Rect, RenderTarget, Renderer, RgbaImage, Shape, TextureId,
};
use flowshot_ui::widgets::{
    ATLAS_HEIGHT, ATLAS_WIDTH, Button, ContextMenu, ICON_ATLAS, Icon, IconButton, Popover,
    Separator, Slider, Toggle, button::ButtonState,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId, WindowLevel};

const ATLAS_TEXTURE: TextureId = TextureId::new(1);
const TIMED_FRAMES: u32 = 300;
const HOLD: Duration = Duration::from_secs(6);
const PHASE_MARKER: &str = "/tmp/flowshot-gallery-phase.txt";

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
            eprintln!("flowshot: widget gallery failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), UiError> {
    let event_loop = EventLoop::<()>::new()?;
    let mut app = GalleryApp::new();
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
struct GalleryApp {
    tokens: DesignTokens,
    phase: Phase,
    frames: u32,
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

impl GalleryApp {
    fn new() -> Self {
        Self {
            tokens: DesignTokens::default(),
            phase: Phase::Render { scale_index: 0 },
            frames: 0,
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
        tracing::error!(%error, "gallery run failed; exiting");
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
        self.monitor_name = monitor
            .name()
            .unwrap_or_else(|| "gallery-monitor".to_owned());
        let attributes = Window::default_attributes()
            .with_title("FlowShot Widget Gallery")
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

        let img = image::load_from_memory(ICON_ATLAS).unwrap().to_rgba8();
        renderer.textures_mut().insert(
            &gpu.device,
            &gpu.queue,
            ATLAS_TEXTURE,
            &RgbaImage {
                width: ATLAS_WIDTH,
                height: ATLAS_HEIGHT,
                data: &img,
            },
        )?;

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
        self.list = gallery_scene(&self.tokens, self.size, scale);
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
            Ok(_) => {
                self.frames += 1;
            }
            Err(error) => {
                tracing::error!(%error, "gallery frame failed");
                self.fatal_error = Some(error);
            }
        }
        frame.present();
    }
}

impl ApplicationHandler<()> for GalleryApp {
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
        tracing::info!("gallery exiting");
    }
}

impl GalleryApp {
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

fn gallery_scene(tokens: &DesignTokens, size: (u32, u32), scale: f32) -> DisplayList {
    let mut list = DisplayList::new();
    let (w, h) = (size.0 as f32, size.1 as f32);
    let window = Rect::from_parts(0.0, 0.0, w, h);
    let contrast = Color::from_hex_token(&tokens.palette.contrast).unwrap();

    // Background
    list.fill(
        Shape::Rect {
            rect: window,
            radius: 0.0,
        },
        contrast,
    );

    let mut y = 50.0 * scale;
    let x = 50.0 * scale;

    // Buttons
    let states = [
        ButtonState::Idle,
        ButtonState::Hover,
        ButtonState::Press,
        ButtonState::Focus,
        ButtonState::Disabled,
    ];

    let mut bx = x;
    for state in states {
        let btn = Button::new(Rect::from_parts(bx, y, 120.0 * scale, 40.0 * scale))
            .state(state)
            .icon(Icon::Save)
            .label("Save");
        btn.draw(&mut list, tokens, scale, ATLAS_TEXTURE);
        bx += 140.0 * scale;
    }
    y += 60.0 * scale;

    // Icon Buttons
    let mut bx = x;
    for state in states {
        let btn = IconButton::new(
            Rect::from_parts(bx, y, 40.0 * scale, 40.0 * scale),
            Icon::Pencil,
        )
        .state(state);
        btn.draw(&mut list, tokens, scale, ATLAS_TEXTURE);
        bx += 60.0 * scale;
    }
    y += 60.0 * scale;

    // Toggles
    let mut bx = x;
    for checked in [false, true] {
        let toggle = Toggle::new(Rect::from_parts(bx, y, 60.0 * scale, 30.0 * scale), checked);
        toggle.draw(&mut list, tokens, scale);
        bx += 80.0 * scale;
    }
    y += 60.0 * scale;

    // Sliders
    let mut bx = x;
    for value in [0.0, 0.5, 1.0] {
        let slider = Slider::new(Rect::from_parts(bx, y, 120.0 * scale, 30.0 * scale), value);
        slider.draw(&mut list, tokens, scale);
        bx += 140.0 * scale;
    }
    y += 60.0 * scale;

    // Separator
    let sep = Separator::new(Rect::from_parts(x, y, 400.0 * scale, 20.0 * scale));
    sep.draw(&mut list, tokens, scale);
    y += 40.0 * scale;

    // Popover
    let popover = Popover::new(Rect::from_parts(x, y, 200.0 * scale, 150.0 * scale));
    popover.draw(&mut list, tokens, scale);

    // Context Menu
    let menu = ContextMenu::new(Rect::from_parts(
        x + 250.0 * scale,
        y,
        150.0 * scale,
        200.0 * scale,
    ));
    menu.draw(&mut list, tokens, scale);

    list
}

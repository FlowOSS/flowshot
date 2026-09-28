//! The winit `ApplicationHandler`: window/GPU spawn, surface lifecycle, and
//! event plumbing. Frame rendering lives in [`super::frame`].

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowId};

use crate::error::UiError;
use crate::gpu::{self, GpuContext, OVERLAY_BACKENDS};

use super::super::model::SettingsModel;
use super::super::strings;
use super::frame;
use super::options::SettingsWindowOptions;
use super::runtime::SettingsEvent;
use crate::egui_host::input::WindowSignal;
use crate::egui_host::{EguiSurface, points, scale_to_ppp};

/// The error-label for surface operations (the overlay passes a monitor
/// name; the settings window is a single normal window).
const SURFACE_LABEL: &str = "settings";

#[derive(Debug)]
pub(super) struct SettingsApp {
    pub(super) options: SettingsWindowOptions,
    pub(super) model: SettingsModel,
    pub(super) window: Option<Arc<Window>>,
    pub(super) gpu: Option<GpuContext>,
    pub(super) surface: Option<wgpu::Surface<'static>>,
    pub(super) surface_config: Option<wgpu::SurfaceConfiguration>,
    pub(super) egui: Option<EguiSurface>,
    pub(super) next_repaint: Option<Instant>,
    pub(super) fatal: Option<UiError>,
}

impl SettingsApp {
    pub(super) fn new(model: SettingsModel, options: SettingsWindowOptions) -> Self {
        Self {
            options,
            model,
            window: None,
            gpu: None,
            surface: None,
            surface_config: None,
            egui: None,
            next_repaint: None,
            fatal: None,
        }
    }

    fn spawn(&mut self, event_loop: &ActiveEventLoop) -> Result<(), UiError> {
        let attributes = Window::default_attributes()
            .with_title(strings::WINDOW_TITLE)
            .with_inner_size(LogicalSize::new(860.0, 620.0))
            .with_min_inner_size(LogicalSize::new(480.0, 400.0));
        let attributes = match &self.options.window_customizer {
            Some(customizer) => customizer.apply(attributes),
            None => attributes,
        };
        let window = Arc::new(event_loop.create_window(attributes).map_err(|source| {
            UiError::WindowCreation {
                monitor: SURFACE_LABEL.to_owned(),
                source,
            }
        })?);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: OVERLAY_BACKENDS,
            ..wgpu::InstanceDescriptor::default()
        });
        let surface =
            instance
                .create_surface(window.clone())
                .map_err(|source| UiError::SurfaceCreation {
                    monitor: SURFACE_LABEL.to_owned(),
                    source,
                })?;
        let gpu = GpuContext::new(&instance, &surface)?;
        let size = window.inner_size();
        let pixels_per_point = scale_to_ppp(window.scale_factor());
        let surface_config = gpu::configure_opaque_surface(
            &surface,
            &gpu.adapter,
            &gpu.device,
            size.width,
            size.height,
            SURFACE_LABEL,
        )?;
        let egui = EguiSurface::new(
            &gpu,
            surface_config.format,
            pixels_per_point,
            points(
                surface_config.width,
                surface_config.height,
                pixels_per_point,
            ),
            self.options.clipboard.clone(),
        );
        window.request_redraw();
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.surface = Some(surface);
        self.surface_config = Some(surface_config);
        self.egui = Some(egui);
        Ok(())
    }

    pub(super) fn resize(&mut self) {
        let Self {
            window,
            gpu,
            surface,
            surface_config,
            egui,
            ..
        } = self;
        let (Some(window), Some(gpu), Some(surface), Some(surface_config), Some(egui)) =
            (window, gpu, surface, surface_config, egui)
        else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return; // minimized; the next valid resize reconfigures
        }
        match gpu::configure_opaque_surface(
            surface,
            &gpu.adapter,
            &gpu.device,
            size.width,
            size.height,
            SURFACE_LABEL,
        ) {
            Ok(config) => *surface_config = config,
            Err(error) => {
                tracing::error!(%error, "settings surface reconfigure failed");
                self.fatal = Some(error);
                return;
            }
        }
        let pixels_per_point = scale_to_ppp(window.scale_factor());
        egui.input_mut().set_pixels_per_point(pixels_per_point);
        egui.input_mut().set_screen_size_points(points(
            surface_config.width,
            surface_config.height,
            pixels_per_point,
        ));
    }
}

impl ApplicationHandler<SettingsEvent> for SettingsApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.spawn(event_loop) {
            tracing::error!(%error, "settings window startup failed");
            self.fatal = Some(error);
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: SettingsEvent) {
        match event {
            SettingsEvent::Exit => event_loop.exit(),
            SettingsEvent::SetSystemTheme(mode) => {
                self.options.system_theme = mode;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => event_loop.exit(),
            WindowEvent::RedrawRequested => frame::render(self, event_loop),
            other => {
                let signal = self.egui.as_mut().map_or(WindowSignal::None, |egui| {
                    egui.input_mut().on_window_event(&other)
                });
                match signal {
                    WindowSignal::Close => event_loop.exit(),
                    WindowSignal::Resized => {
                        self.resize();
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                    WindowSignal::None => {
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(match self.next_repaint {
            Some(at) if at > Instant::now() => ControlFlow::WaitUntil(at),
            Some(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                ControlFlow::Wait
            }
            None => ControlFlow::Wait,
        });
    }
}

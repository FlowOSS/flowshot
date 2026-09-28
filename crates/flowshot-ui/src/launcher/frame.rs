//! The launcher frame pipeline: acquire -> egui frame -> paint -> present,
//! and the dispatch of the frame's [`LauncherAction`] (Capture = callback +
//! close, Cancel = close).

use std::time::{Duration, Instant};

use winit::event_loop::ActiveEventLoop;

use crate::egui_host::{cursor_icon, points, scale_to_ppp, theme};
use crate::error::UiError;

use super::app::LauncherApp;
use super::ui::{self as widgets, LauncherAction};

pub(super) fn render(app: &mut LauncherApp, event_loop: &ActiveEventLoop) {
    let frame = match app.surface.as_ref().map(wgpu::Surface::get_current_texture) {
        Some(Ok(frame)) => frame,
        Some(Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated)) => {
            app.resize();
            return;
        }
        Some(Err(wgpu::SurfaceError::Timeout)) => {
            if let Some(window) = &app.window {
                window.request_redraw();
            }
            return;
        }
        Some(Err(wgpu::SurfaceError::OutOfMemory)) => {
            app.fatal = Some(UiError::OutOfMemory);
            event_loop.exit();
            return;
        }
        None => return,
    };
    let LauncherApp {
        options,
        model,
        window,
        gpu,
        surface_config,
        egui,
        next_repaint,
        ..
    } = app;
    let (Some(window), Some(gpu), Some(surface_config), Some(egui)) =
        (window, gpu, surface_config, egui)
    else {
        return;
    };
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let pixels_per_point = scale_to_ppp(window.scale_factor());
    egui.input_mut().set_pixels_per_point(pixels_per_point);
    egui.input_mut().set_screen_size_points(points(
        surface_config.width,
        surface_config.height,
        pixels_per_point,
    ));
    let style = theme::style(&options.tokens, &options.ui_config, options.system_theme);
    let (output, action) = egui.frame_with(egui::CentralPanel::default(), style, |ui| {
        widgets::show(ui, model)
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("launcher-frame-encoder"),
        });
    egui.paint(
        gpu,
        &mut encoder,
        &view,
        [surface_config.width, surface_config.height],
        &output,
    );
    gpu.queue.submit(Some(encoder.finish()));
    frame.present();
    window.set_cursor(cursor_icon(output.platform_output.cursor_icon));
    let repaint_delay = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(Duration::MAX, |viewport| viewport.repaint_delay);
    *next_repaint = (repaint_delay < Duration::MAX).then(|| Instant::now() + repaint_delay);
    match action {
        LauncherAction::None => {}
        LauncherAction::Capture(request) => {
            tracing::info!(request = ?request, "launcher capture dispatched");
            if let Some(callback) = &options.on_capture {
                callback.invoke(&request);
            }
            event_loop.exit();
        }
        LauncherAction::Cancel => event_loop.exit(),
    }
}

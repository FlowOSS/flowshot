//! The consent frame pipeline: acquire -> egui frame -> paint -> present,
//! and the dispatch of the frame's [`ConsentAction`]. Every answering path
//! (button, Enter, Esc, window close) funnels through [`choose`], which
//! invokes the persistence seam and exits - one action per frame plus the
//! exit-on-dispatch is what makes the callback fire exactly once.

use std::time::{Duration, Instant};

use winit::event_loop::ActiveEventLoop;

use crate::egui_host::{cursor_icon, points, scale_to_ppp, theme};
use crate::error::UiError;
use crate::settings::FormMetrics;

use super::app::ConsentApp;
use super::model::ConsentModel;
use super::ui::{self as widgets, ConsentAction};

pub(super) fn render(app: &mut ConsentApp, event_loop: &ActiveEventLoop) {
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
    let ConsentApp {
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
    let style = theme::settings_style(&options.tokens, &options.ui_config, options.system_theme);
    let metrics = FormMetrics::from_tokens(&options.tokens);
    let panel = egui::CentralPanel::default().frame(
        egui::Frame::none()
            .fill(style.visuals.panel_fill)
            .inner_margin(egui::Margin::same(metrics.window_margin())),
    );
    let (output, action) = egui.frame_with(panel, style, |ui| widgets::show(ui, model, &metrics));
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("consent-frame-encoder"),
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
    choose(app, event_loop, action);
}

/// Dispatches one answered action: the persistence seam receives the
/// chosen config (the deferred one for "Not now") and the loop exits.
/// [`ConsentAction::None`] keeps the dialog open.
pub(super) fn choose(app: &mut ConsentApp, event_loop: &ActiveEventLoop, action: ConsentAction) {
    let choice = match action {
        ConsentAction::None => return,
        ConsentAction::Save(choice) => choice,
        ConsentAction::NotNow => ConsentModel::deferred(),
    };
    tracing::info!(
        enabled = choice.enabled,
        technical_details = choice.include_technical_details,
        "telemetry consent recorded"
    );
    if let Some(callback) = &app.options.on_choice {
        callback.invoke(choice);
    }
    event_loop.exit();
}

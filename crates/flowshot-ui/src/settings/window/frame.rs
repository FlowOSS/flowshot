//! The frame pipeline: acquire -> egui frame -> paint -> present, the Apply
//! persistence path, and the egui->winit cursor-icon projection.

use std::time::{Duration, Instant};

use winit::event_loop::ActiveEventLoop;

use crate::egui_host::{cursor_icon, points, scale_to_ppp, theme};

use super::super::layout::FormMetrics;
use super::super::model::{Banner, SettingsModel};
use super::super::tabs::{self, FrameAction, TabContext};
use super::app::SettingsApp;
use super::options::SettingsWindowOptions;

pub(super) fn render(app: &mut SettingsApp, event_loop: &ActiveEventLoop) {
    let frame = match app.surface.as_ref().map(wgpu::Surface::get_current_texture) {
        Some(
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame),
        ) => frame,
        Some(wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated) => {
            app.resize();
            return;
        }
        Some(
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation,
        ) => {
            if let Some(window) = &app.window {
                window.request_redraw();
            }
            return;
        }
        None => return,
    };
    let SettingsApp {
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
    let mode = model.theme().resolve(options.system_theme);
    let style = theme::settings_style(&options.tokens, &model.config().ui, mode);
    let metrics = FormMetrics::from_tokens(&options.tokens);
    let panel = egui::CentralPanel::default().frame(
        egui::Frame::NONE
            .fill(style.visuals.panel_fill)
            .inner_margin(metrics.window_margin()),
    );
    let context = TabContext {
        system_theme: options.system_theme,
        path_picker: options.path_picker.as_ref(),
        metrics,
    };
    let (mut output, action) = egui.frame_with(panel, style, |ui| tabs::show(ui, model, &context));
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("settings-frame-encoder"),
        });
    egui.paint(
        gpu,
        &mut encoder,
        &view,
        [surface_config.width, surface_config.height],
        &mut output,
    );
    gpu.queue.submit(Some(encoder.finish()));
    gpu.queue.present(frame);
    window.set_cursor(cursor_icon(output.platform_output.cursor_icon));
    let repaint_delay = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(Duration::MAX, |viewport| viewport.repaint_delay);
    *next_repaint = (repaint_delay < Duration::MAX).then(|| Instant::now() + repaint_delay);
    match action {
        FrameAction::None => {}
        FrameAction::Apply => apply(model, options),
        FrameAction::Close => event_loop.exit(),
    }
    if action != FrameAction::None {
        window.request_redraw();
    }
}

/// The Apply pipeline: validation gate -> migration-safe TOML write ->
/// applied callback (`ConfigChanged` emitter) -> clean state.
fn apply(model: &mut SettingsModel, options: &SettingsWindowOptions) {
    if !model.validate().is_empty() {
        model.set_banner(Some(Banner::Validation));
        return;
    }
    let config = model.config().clone();
    match config.save(&options.config_path) {
        Ok(()) => {
            model.mark_clean();
            model.set_banner(None);
            if let Some(callback) = &options.on_applied {
                callback.invoke(&config);
            }
            tracing::info!(path = %options.config_path.display(), "settings applied");
        }
        Err(error) => {
            tracing::error!(path = %options.config_path.display(), %error, "settings save failed");
            model.set_banner(Some(Banner::SaveFailed(error.to_string())));
        }
    }
}

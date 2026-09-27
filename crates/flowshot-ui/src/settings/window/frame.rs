//! The frame pipeline: acquire -> egui frame -> paint -> present, the Apply
//! persistence path, and the egui->winit cursor-icon projection.

use std::time::{Duration, Instant};

use winit::event_loop::ActiveEventLoop;
use winit::window::CursorIcon;

use crate::error::UiError;

use super::super::model::{Banner, SettingsModel};
use super::super::tabs::{FrameAction, TabContext};
use super::super::theme;
use super::app::SettingsApp;
use super::options::SettingsWindowOptions;

pub(super) fn render(app: &mut SettingsApp, event_loop: &ActiveEventLoop) {
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
    let style = theme::style(&options.tokens, &model.config().ui, mode);
    let context = TabContext {
        system_theme: options.system_theme,
        path_picker: options.path_picker.as_ref(),
    };
    let (output, action) = egui.frame(model, &context, style);
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

/// winit reports the scale factor as `f64`; egui works in `f32` points.
#[expect(
    clippy::cast_possible_truncation,
    reason = "scale factors are small values far within f32 range"
)]
pub(super) fn scale_to_ppp(scale_factor: f64) -> f32 {
    scale_factor as f32
}

#[expect(
    clippy::cast_precision_loss,
    reason = "u32 pixel extents -> f32 egui points; window sizes are far within f32 precision"
)]
pub(super) fn points(width: u32, height: u32, pixels_per_point: f32) -> egui::Vec2 {
    egui::Vec2::new(
        width as f32 / pixels_per_point,
        height as f32 / pixels_per_point,
    )
}

fn cursor_icon(icon: egui::output::CursorIcon) -> CursorIcon {
    use egui::output::CursorIcon as Egui;
    match icon {
        // A hidden cursor has no winit icon equivalent; the settings UI
        // never requests one, so the default arrow is the honest fallback.
        Egui::Default | Egui::None => CursorIcon::Default,
        Egui::ContextMenu => CursorIcon::ContextMenu,
        Egui::Help => CursorIcon::Help,
        Egui::PointingHand => CursorIcon::Pointer,
        Egui::Progress => CursorIcon::Progress,
        Egui::Wait => CursorIcon::Wait,
        Egui::Cell => CursorIcon::Cell,
        Egui::Crosshair => CursorIcon::Crosshair,
        Egui::Text => CursorIcon::Text,
        Egui::VerticalText => CursorIcon::VerticalText,
        Egui::Alias => CursorIcon::Alias,
        Egui::Copy => CursorIcon::Copy,
        Egui::Move => CursorIcon::Move,
        Egui::NoDrop => CursorIcon::NoDrop,
        Egui::NotAllowed => CursorIcon::NotAllowed,
        Egui::Grab => CursorIcon::Grab,
        Egui::Grabbing => CursorIcon::Grabbing,
        Egui::AllScroll => CursorIcon::AllScroll,
        Egui::ResizeHorizontal => CursorIcon::EwResize,
        Egui::ResizeVertical => CursorIcon::NsResize,
        Egui::ResizeNeSw => CursorIcon::NeswResize,
        Egui::ResizeNwSe => CursorIcon::NwseResize,
        Egui::ResizeNorth => CursorIcon::NResize,
        Egui::ResizeSouth => CursorIcon::SResize,
        Egui::ResizeEast => CursorIcon::EResize,
        Egui::ResizeWest => CursorIcon::WResize,
        Egui::ResizeNorthEast => CursorIcon::NeResize,
        Egui::ResizeNorthWest => CursorIcon::NwResize,
        Egui::ResizeSouthEast => CursorIcon::SeResize,
        Egui::ResizeSouthWest => CursorIcon::SwResize,
        Egui::ResizeColumn => CursorIcon::ColResize,
        Egui::ResizeRow => CursorIcon::RowResize,
        Egui::ZoomIn => CursorIcon::ZoomIn,
        Egui::ZoomOut => CursorIcon::ZoomOut,
    }
}

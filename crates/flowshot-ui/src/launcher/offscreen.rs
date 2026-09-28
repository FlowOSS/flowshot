//! The launcher dialog's headless QA path: drives ONE
//! launcher frame through the shared embedded egui stack
//! ([`crate::egui_host`]) into a texture and reads it back - no window, no
//! display server (the no-visible-windows QA policy path, mirroring
//! [`crate::settings::render_offscreen`]).

use crate::egui_host::{EguiSurface, theme};
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::render::read_texture_rgba;
use crate::settings::{FormMetrics, OFFSCREEN_FORMAT};

use super::model::LauncherModel;
use super::options::LauncherWindowOptions;
use super::ui;

/// Renders ONE launcher frame headlessly into a texture and reads it back as
/// tightly packed RGBA bytes (two frames: the warm frame primes egui's
/// previous-frame persistent state, the painted frame is the deterministic
/// second one - the settings offscreen precedent).
///
/// # Errors
///
/// [`UiError::TextureTooLarge`] when the extent exceeds the device limits.
#[expect(
    clippy::cast_precision_loss,
    reason = "u32 pixel extents -> f32 egui points; dialog sizes are far within f32 precision"
)]
pub fn render_offscreen(
    gpu: &GpuContext,
    options: &LauncherWindowOptions,
    model: &mut LauncherModel,
    width: u32,
    height: u32,
    pixels_per_point: f32,
) -> Result<Vec<u8>, UiError> {
    let limit = gpu.device.limits().max_texture_dimension_2d;
    if width.max(height) > limit {
        return Err(UiError::TextureTooLarge {
            width,
            height,
            max: limit,
        });
    }
    let mut surface = EguiSurface::new(
        gpu,
        OFFSCREEN_FORMAT,
        pixels_per_point,
        egui::Vec2::new(
            width as f32 / pixels_per_point,
            height as f32 / pixels_per_point,
        ),
        None,
    );
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("launcher-offscreen"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let style = theme::settings_style(&options.tokens, &options.ui_config, options.system_theme);
    let metrics = FormMetrics::from_tokens(&options.tokens);
    for encoder_label in [
        "launcher-offscreen-warm-encoder",
        "launcher-offscreen-encoder",
    ] {
        let panel = egui::CentralPanel::default().frame(
            egui::Frame::none()
                .fill(style.visuals.panel_fill)
                .inner_margin(egui::Margin::same(metrics.window_margin())),
        );
        let (output, _action) = surface.frame_with(panel, style.clone(), |ui| {
            ui::show(ui, &mut *model, &metrics)
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(encoder_label),
            });
        surface.paint(gpu, &mut encoder, &view, [width, height], &output);
        gpu.queue.submit(Some(encoder.finish()));
    }
    read_texture_rgba(&gpu.device, &gpu.queue, &texture, width, height)
}

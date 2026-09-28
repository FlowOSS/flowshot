//! The settings window's headless QA path: [`render_offscreen`] drives ONE
//! settings frame through the shared embedded egui stack
//! ([`crate::egui_host`]) into a texture and reads it back - no window, no
//! display server (the no-visible-windows QA policy path).

use flowshot_core::tokens::DesignTokens;

use crate::egui_host::{EguiSurface, theme::ThemeMode};
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::render::read_texture_rgba;

use super::layout::FormMetrics;
use super::model::SettingsModel;
use super::tabs::{self, TabContext};
use crate::egui_host::theme;

/// The offscreen target format for headless settings renders (egui-wgpu
/// recommends a gamma-space 8-bit target; readback expects `Rgba8Unorm*`).
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Renders ONE settings frame headlessly into a texture and reads it back as
/// tightly packed RGBA bytes - no window, no display server (the
/// no-visible-windows QA path).
///
/// # Errors
///
/// [`UiError::TextureTooLarge`] when the extent exceeds the device limits.
#[expect(
    clippy::cast_precision_loss,
    reason = "u32 pixel extents -> f32 egui points; window sizes are far within f32 precision"
)]
pub fn render_offscreen(
    gpu: &GpuContext,
    tokens: &DesignTokens,
    model: &mut SettingsModel,
    system_theme: ThemeMode,
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
        label: Some("settings-offscreen"),
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
    let style = theme::settings_style(tokens, &model.config().ui, system_theme);
    let metrics = FormMetrics::from_tokens(tokens);
    let panel_fill = style.visuals.panel_fill;
    let panel = move || {
        egui::CentralPanel::default().frame(
            egui::Frame::none()
                .fill(panel_fill)
                .inner_margin(egui::Margin::same(metrics.window_margin())),
        )
    };
    let context = TabContext {
        system_theme,
        path_picker: None,
        metrics,
    };
    // Two frames: egui's ScrollArea clip rect and scrollbar state are
    // previous-frame persistent state, so the FIRST frame of a fresh
    // context paints unclipped. The warm frame primes that state (the live
    // window gets the same second frame from egui's own repaint request);
    // the painted + read-back frame is the deterministic second one.
    let (warm, _warm_action) =
        surface.frame_with(panel(), style.clone(), |ui| tabs::show(ui, model, &context));
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("settings-offscreen-warm-encoder"),
        });
    surface.paint(gpu, &mut encoder, &view, [width, height], &warm);
    gpu.queue.submit(Some(encoder.finish()));
    let (output, _action) =
        surface.frame_with(panel(), style, |ui| tabs::show(ui, model, &context));
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("settings-offscreen-encoder"),
        });
    surface.paint(gpu, &mut encoder, &view, [width, height], &output);
    gpu.queue.submit(Some(encoder.finish()));
    read_texture_rgba(&gpu.device, &gpu.queue, &texture, width, height)
}

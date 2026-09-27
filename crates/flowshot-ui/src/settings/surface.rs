//! The embedded egui surface (draft D8(b), Ruffle pattern): an
//! [`egui::Context`] + [`egui_wgpu::Renderer`] pair driven by OUR winit/wgpu
//! stack - egui is a guest in this crate's renderer, never a second windowing
//! stack (plan MUST-NOT).
//!
//! egui-winit is deliberately absent (version dead end - see the crate
//! manifest note): [`super::input::InputState`] feeds the context from raw
//! winit 0.30 events instead. The same split makes the surface renderable
//! WITHOUT any window: [`render_offscreen`] drives one frame into a texture
//! and reads it back - the todo-36 QA path under the no-visible-windows
//! policy.

use egui::Context;
use flowshot_core::tokens::DesignTokens;

use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::render::read_texture_rgba;

use super::input::{ClipboardBridge, InputState};
use super::model::SettingsModel;
use super::tabs::{self, FrameAction, TabContext};
use super::theme::{self, ThemeMode};

/// The egui state of one window (or one offscreen frame).
pub struct SettingsSurface {
    ctx: Context,
    renderer: egui_wgpu::Renderer,
    input: InputState,
    color_format: wgpu::TextureFormat,
}

impl std::fmt::Debug for SettingsSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsSurface")
            .field("ctx", &self.ctx)
            .field("renderer", &"egui_wgpu::Renderer(..)")
            .field("input", &self.input)
            .field("color_format", &self.color_format)
            .finish()
    }
}

impl SettingsSurface {
    /// Builds the surface for `color_format` targets.
    ///
    /// Five independent initialization inputs (GPU handle, target format,
    /// initial geometry as points + scale, optional clipboard seam) with no
    /// cohesive subgroup worth a wrapper type; both callers (window spawn,
    /// offscreen render) supply them from different sources.
    #[must_use]
    pub fn new(
        gpu: &GpuContext,
        color_format: wgpu::TextureFormat,
        pixels_per_point: f32,
        screen_size_points: egui::Vec2,
        clipboard: Option<ClipboardBridge>,
    ) -> Self {
        let ctx = Context::default();
        ctx.set_fonts(theme::fonts());
        let renderer = egui_wgpu::Renderer::new(&gpu.device, color_format, None, 1);
        let input = InputState::new(
            pixels_per_point,
            screen_size_points,
            gpu.device.limits().max_texture_dimension_2d,
            clipboard,
        );
        Self {
            ctx,
            renderer,
            input,
            color_format,
        }
    }

    /// The egui context (style/font inspection, debug overlays).
    #[must_use]
    pub const fn context(&self) -> &Context {
        &self.ctx
    }

    /// The input accumulator; the window layer forwards every
    /// [`winit::event::WindowEvent`] here.
    pub(super) fn input_mut(&mut self) -> &mut InputState {
        &mut self.input
    }

    /// Runs one egui frame over the model; returns the frame output (for
    /// [`Self::paint`] + platform-output handling) and the window-level
    /// action the UI requested.
    pub fn frame(
        &mut self,
        model: &mut SettingsModel,
        context: &TabContext<'_>,
        style: egui::Style,
    ) -> (egui::FullOutput, FrameAction) {
        self.ctx.set_style(style);
        let input = self.input.take_raw_input();
        let mut action = FrameAction::None;
        let output = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = tabs::show(ui, model, context);
            });
        });
        self.input
            .push_copied_text(&output.platform_output.copied_text);
        (output, action)
    }

    /// Tessellates + uploads + renders `output` into `target` (clearing to
    /// the theme's panel fill). The caller submits the encoder.
    pub fn paint(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size_in_pixels: [u32; 2],
        output: &egui::FullOutput,
    ) {
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels,
            pixels_per_point: output.pixels_per_point,
        };
        for (id, delta) in &output.textures_delta.set {
            self.renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let paint_jobs = self
            .ctx
            .tessellate(output.shapes.clone(), screen.pixels_per_point);
        let callback_buffers =
            self.renderer
                .update_buffers(&gpu.device, &gpu.queue, encoder, &paint_jobs, &screen);
        let fill = self.ctx.style().visuals.panel_fill;
        // sRGB targets encode the clear value (linear components, the eframe
        // convention); gamma-space targets (the offscreen `Rgba8Unorm` QA
        // path) take the sRGB bytes directly, so readback lands in the same
        // byte space the theme specifies.
        let clear = if self.color_format.is_srgb() {
            let linear = egui::Rgba::from(fill).to_array();
            wgpu::Color {
                r: f64::from(linear[0]),
                g: f64::from(linear[1]),
                b: f64::from(linear[2]),
                a: 1.0,
            }
        } else {
            wgpu::Color {
                r: f64::from(fill.r()) / 255.0,
                g: f64::from(fill.g()) / 255.0,
                b: f64::from(fill.b()) / 255.0,
                a: 1.0,
            }
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("settings-frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        self.renderer.render(&mut pass, &paint_jobs, &screen);
        drop(pass);
        gpu.queue.submit(callback_buffers);
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}

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
    let mut surface = SettingsSurface::new(
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
    let style = theme::style(tokens, &model.config().ui, system_theme);
    let context = TabContext {
        system_theme,
        path_picker: None,
    };
    let (output, _action) = surface.frame(model, &context, style);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("settings-offscreen-encoder"),
        });
    surface.paint(gpu, &mut encoder, &view, [width, height], &output);
    gpu.queue.submit(Some(encoder.finish()));
    read_texture_rgba(&gpu.device, &gpu.queue, &texture, width, height)
}

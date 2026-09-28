//! The per-window egui state: context + renderer + input accumulator, with
//! the generic frame/paint pipeline both egui windows share.

use egui::{Context, Style, Ui};

use crate::gpu::GpuContext;

use super::input::{ClipboardBridge, InputState};

/// The egui state of one window (or one offscreen frame).
pub(crate) struct EguiSurface {
    ctx: Context,
    renderer: egui_wgpu::Renderer,
    input: InputState,
    color_format: wgpu::TextureFormat,
}

impl std::fmt::Debug for EguiSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EguiSurface")
            .field("ctx", &self.ctx)
            .field("renderer", &"egui_wgpu::Renderer(..)")
            .field("input", &self.input)
            .field("color_format", &self.color_format)
            .finish()
    }
}

impl EguiSurface {
    /// Builds the surface for `color_format` targets.
    ///
    /// Five independent initialization inputs (GPU handle, target format,
    /// initial geometry as points + scale, optional clipboard seam) with no
    /// cohesive subgroup worth a wrapper type; both window runtimes and the
    /// offscreen QA paths supply them from different sources.
    #[must_use]
    pub(crate) fn new(
        gpu: &GpuContext,
        color_format: wgpu::TextureFormat,
        pixels_per_point: f32,
        screen_size_points: egui::Vec2,
        clipboard: Option<ClipboardBridge>,
    ) -> Self {
        let ctx = Context::default();
        ctx.set_fonts(super::theme::fonts());
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

    /// The input accumulator; the window layer forwards every
    /// [`winit::event::WindowEvent`] here.
    pub(crate) fn input_mut(&mut self) -> &mut InputState {
        &mut self.input
    }

    /// Runs one egui frame: `panel` is the host frame the `show` closure
    /// renders into (the settings window passes a token-margined frame; the
    /// launcher passes the default), `style` is installed, the accumulated
    /// input is drained. Returns the frame output (for [`Self::paint`] +
    /// platform-output handling) and whatever `show` produced.
    /// `A::default()` (the "no action" value) covers the case where egui
    /// skips the panel body.
    pub(crate) fn frame_with<A: Default>(
        &mut self,
        panel: egui::CentralPanel,
        style: Style,
        show: impl FnOnce(&mut Ui) -> A,
    ) -> (egui::FullOutput, A) {
        self.ctx.set_style(style);
        let input = self.input.take_raw_input();
        let mut action = A::default();
        let output = self.ctx.run(input, |ctx| {
            action = panel.show(ctx, show).inner;
        });
        self.input
            .push_copied_text(&output.platform_output.copied_text);
        (output, action)
    }

    /// Tessellates + uploads + renders `output` into `target` (clearing to
    /// the theme's panel fill). The caller submits the encoder.
    pub(crate) fn paint(
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
            label: Some("egui-host-frame"),
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

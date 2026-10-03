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
        let renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            color_format,
            egui_wgpu::RendererOptions {
                // The pre-0.33 constructor's fixed behavior: no MSAA (egui
                // feathering is the antialiasing), no depth/stencil, and no
                // dithering - the offscreen QA contract pins exact token
                // colors on flat fills (dithering adds +/-1 LSB noise).
                msaa_samples: 1,
                depth_stencil_format: None,
                dithering: false,
                ..egui_wgpu::RendererOptions::default()
            },
        );
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
        // egui 0.36 stores one style per theme; this host resolves dark/light
        // itself (design tokens + system theme), so both themes receive the
        // same resolved style - the pre-0.36 single-`set_style` semantics.
        self.ctx.set_style_of(egui::Theme::Dark, style.clone());
        self.ctx.set_style_of(egui::Theme::Light, style);
        let input = self.input.take_raw_input();
        let mut action = A::default();
        // `run_ui` is `FnMut` (multi-pass on `request_discard`, which this
        // host's widgets never call); `take()` keeps the `FnOnce` inputs
        // consumed exactly once and any extra pass a no-op.
        let (mut panel, mut show) = (Some(panel), Some(show));
        let output = self.ctx.run_ui(input, |ui| {
            let (Some(panel), Some(show)) = (panel.take(), show.take()) else {
                return;
            };
            action = panel.show(ui, show).inner;
        });
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                self.input.push_copied_text(text);
            }
        }
        (output, action)
    }

    /// Tessellates + uploads + renders `output` into `target` (clearing to
    /// the theme's panel fill) and consumes `output`'s texture deltas (egui
    /// 0.36 panics when a `TexturesDelta` drops unapplied). The caller
    /// submits the encoder.
    pub(crate) fn paint(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size_in_pixels: [u32; 2],
        output: &mut egui::FullOutput,
    ) {
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels,
            pixels_per_point: output.pixels_per_point,
        };
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        let paint_jobs = self
            .ctx
            .tessellate(output.shapes.clone(), screen.pixels_per_point);
        let callback_buffers =
            self.renderer
                .update_buffers(&gpu.device, &gpu.queue, encoder, &paint_jobs, &screen);
        let fill = self.ctx.style_of(self.ctx.theme()).visuals.panel_fill;
        // sRGB targets (the debug-logged sRGB-only fallback) encode the clear
        // value from linear components (the eframe convention); gamma-space
        // targets - the live egui host surfaces and the offscreen QA path -
        // take the sRGB bytes directly, so pixels land in the same byte
        // space the theme specifies.
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
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui-host-frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // egui-wgpu's `render` takes a lifetime-erased pass; the encoder is
        // untouched until the pass drops (same ordering as before the erase).
        let mut pass = pass.forget_lifetime();
        self.renderer.render(&mut pass, &paint_jobs, &screen);
        drop(pass);
        gpu.queue.submit(callback_buffers);
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        output.textures_delta.clear();
    }
}

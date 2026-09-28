//! Offscreen export rendering.
//!
//! ONE export implementation shared by the live shell and the headless QA
//! harnesses: each prepared output renders through the real production
//! render path (frozen backdrop 1:1 + the pixel-effect quads + the
//! annotation scene via [`EditorState::paint_export_into`]) into an
//! offscreen target - no dim, no selection chrome, no crosshair, no
//! magnifier, no grid - reads back as upright RGBA, and
//! [`composite_selection`] merges the per-output crops physical-first
//! (#4871).
//!
//! Two entry points:
//! - `render_completion` for the live shell: renders through each
//!   window's EXISTING renderer (the backdrop's CPU pixels are drained on
//!   first upload, so the window renderers are the only texture holders)
//!   and normalizes the surface format's channel order (live surfaces
//!   prefer `Bgra8UnormSrgb`).
//! - [`render_export`] for headless harnesses: a fresh `Rgba8UnormSrgb`
//!   renderer with the frames uploaded from a not-yet-drained backdrop
//!   (the `frozen_backdrop --verify-offscreen` pattern).

use flowshot_core::geometry::{LogicalRect, OutputLayout};

use std::time::Instant;

use winit::event_loop::ActiveEventLoop;

use crate::app::WindowEntry;
use crate::backdrop::{Backdrop, BackdropOptions};
use crate::completion::{
    Completion, CompletionKind, ExportedImage, RenderedOutput, composite_selection,
};
use crate::editor::{EditorState, PixelEffect};
use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::input::Action;
use crate::render::{RenderTarget, Renderer, TextureId, read_texture_rgba};
use crate::router::{InputRouter, WindowSlot};

/// Renders ONE output's export frame through `renderer` and reads it back
/// as upright RGBA; `None` when the output has no prepared frozen frame.
///
/// # Errors
///
/// Texture validation, offscreen-target, render, or readback failures.
pub fn render_output_export(
    gpu: &GpuContext,
    renderer: &mut Renderer,
    backdrop: &Backdrop,
    editor: &EditorState,
    layout: &OutputLayout,
    output_index: usize,
    cursor_visible: bool,
) -> Result<Option<RenderedOutput>, UiError> {
    let Some((width, height)) = backdrop.texture_size(output_index) else {
        return Ok(None);
    };
    let options = BackdropOptions {
        dim: false,
        cursor_visible,
        selection: None,
    };
    let mut list = backdrop.commands(output_index, (width, height), &options);
    if let Some(output) = layout.outputs.get(output_index) {
        editor.paint_export_into(&mut list, output);
    }
    let render_started = std::time::Instant::now();
    let target = renderer.create_offscreen_target(&gpu.device, width, height)?;
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.render(
        &gpu.device,
        &gpu.queue,
        &RenderTarget {
            view: &view,
            width,
            height,
        },
        &list,
    )?;
    let mut rgba = read_texture_rgba(&gpu.device, &gpu.queue, &target, width, height)?;
    normalize_to_rgba(renderer.format(), &mut rgba);
    tracing::info!(
        target: "flowshot_perf",
        output_index,
        width,
        height,
        elapsed_us = u64::try_from(render_started.elapsed().as_micros()).unwrap_or(u64::MAX),
        "perf.output_render"
    );
    Ok(Some(RenderedOutput {
        output_index,
        width,
        height,
        rgba,
    }))
}

/// The live shell's completion render: every bound window's
/// renderer draws its output's export frame (effect textures synced first -
/// a keyboard-committed effect may not have reached this renderer yet),
/// then the crops composite physical-first. `None` when a cropped output
/// has no render (the composite contract).
pub(crate) fn render_completion(
    gpu: &GpuContext,
    backdrop: &Backdrop,
    editor: &EditorState,
    router: &InputRouter,
    selection: LogicalRect,
    cursor_visible: bool,
    windows: &mut [WindowEntry],
) -> Option<ExportedImage> {
    let mut crops = Vec::new();
    for (slot_index, entry) in windows.iter_mut().enumerate() {
        let slot = WindowSlot::new(slot_index);
        let Some(output_index) = router.output_index_for(slot) else {
            continue;
        };
        let Some(renderer) = entry.renderer.as_mut() else {
            continue;
        };
        if let Err(error) = sync_effect_textures(
            renderer,
            gpu,
            &mut entry.effect_textures,
            editor.pixel_effects(),
        ) {
            tracing::error!(%error, window = slot_index, "export effect texture sync failed");
        }
        match render_output_export(
            gpu,
            renderer,
            backdrop,
            editor,
            router.layout(),
            output_index,
            cursor_visible,
        ) {
            Ok(Some(output)) => crops.push(output),
            Ok(None) => tracing::warn!(window = slot_index, "no frozen frame; output skipped"),
            Err(error) => tracing::error!(%error, window = slot_index, "export render failed"),
        }
    }
    composite_selection(router.layout(), selection, &crops)
}

/// The headless one-call export (QA harnesses): a fresh `Rgba8UnormSrgb`
/// renderer, the backdrop's frames + cursor + the editor's effect textures
/// uploaded into it, every prepared output rendered, composited.
///
/// # Errors
///
/// Texture upload, render, or readback failures.
pub fn render_export(
    gpu: &GpuContext,
    backdrop: &mut Backdrop,
    editor: &EditorState,
    selection: LogicalRect,
    cursor_visible: bool,
) -> Result<Option<ExportedImage>, UiError> {
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8UnormSrgb);
    let mut uploaded: Vec<TextureId> = Vec::new();
    sync_effect_textures(&mut renderer, gpu, &mut uploaded, editor.pixel_effects())?;
    backdrop.upload_cursor(&mut renderer, gpu)?;
    let layout = backdrop.layout().clone();
    let mut crops = Vec::new();
    for index in 0..layout.outputs.len() {
        backdrop.upload_for(index, &mut renderer, gpu)?;
        if let Some(output) = render_output_export(
            gpu,
            &mut renderer,
            backdrop,
            editor,
            &layout,
            index,
            cursor_visible,
        )? {
            crops.push(output);
        }
    }
    Ok(composite_selection(&layout, selection, &crops))
}

/// Syncs one renderer's pixel-effect texture set to the editor's effect
/// layer: uploads new bakes, drops textures of undone/replaced
/// effects (each bake can be megabytes - retired ids must not linger).
/// A failed upload logs and keeps the id marked uploaded: the renderer's
/// magenta placeholder is the visible failure signal, retried never per
/// frame (log-spam guard).
pub(crate) fn sync_effect_textures(
    renderer: &mut Renderer,
    gpu: &GpuContext,
    uploaded: &mut Vec<TextureId>,
    effects: &[PixelEffect],
) -> Result<(), UiError> {
    for id in uploaded.iter().copied() {
        if !effects.iter().any(|effect| effect.texture_id() == id) {
            renderer.textures_mut().remove(id);
        }
    }
    for effect in effects {
        let id = effect.texture_id();
        if uploaded.contains(&id) {
            continue;
        }
        let image = crate::render::RgbaImage {
            width: effect.width(),
            height: effect.height(),
            data: effect.pixels(),
        };
        let result = renderer
            .textures_mut()
            .insert(&gpu.device, &gpu.queue, id, &image);
        uploaded.push(id);
        result?;
    }
    uploaded.retain(|id| effects.iter().any(|effect| effect.texture_id() == *id));
    Ok(())
}

/// Live overlay surfaces prefer `Bgra8UnormSrgb`, so their readback bytes
/// arrive BGRA-ordered; the export contract is RGBA. sRGB encoding is
/// untouched (the frozen frames and the target share the sRGB byte space).
fn normalize_to_rgba(format: wgpu::TextureFormat, bytes: &mut [u8]) {
    if matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    ) {
        for pixel in bytes.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    }
}

impl crate::app::OverlayApp {
    /// The capture-completion path: renders the export offscreen
    /// through the production render path, composites it physical-first,
    /// hands it to the installed [`CompletionSink`](crate::CompletionSink),
    /// and tears down. The binary layer owns encoding and post-capture
    /// actions; a missing sink or image is logged, never silent.
    pub(crate) fn complete(&mut self, target: &ActiveEventLoop, action: Action) {
        let started = Instant::now();
        let _span = tracing::info_span!("export.completion").entered();
        let kind = match action {
            Action::Accept => CompletionKind::Accept,
            Action::Copy => CompletionKind::Copy,
            Action::Save => CompletionKind::Save,
            Action::Pin => CompletionKind::Pin,
            Action::Upload => CompletionKind::Upload,
            Action::OpenWith => CompletionKind::OpenWith,
            Action::Redraw(_) | Action::Exit | Action::ColorWheel | Action::ColorPicked => return,
        };
        let Some(selection) = self.core.selection().rect() else {
            tracing::warn!("completion gesture without a selection; tearing down");
            target.exit();
            return;
        };
        let image = match (self.gpu.as_ref(), self.backdrop.as_ref()) {
            (Some(gpu), Some(backdrop)) => render_completion(
                gpu,
                backdrop,
                self.core.editor(),
                self.core.router(),
                selection,
                self.backdrop_options.cursor_visible,
                &mut self.windows,
            ),
            _ => None,
        };
        if let Some(image) = image {
            tracing::info!(
                target: "flowshot_ui::latency",
                width = image.width,
                height = image.height,
                elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
                "export composited"
            );
            if let Some(sink) = self.core.completion_sink() {
                sink(Completion {
                    kind,
                    selection,
                    image,
                });
            } else {
                tracing::warn!("no completion sink installed; export dropped");
            }
        } else {
            tracing::error!("export compositing produced no image; capture lost");
        }
        target.exit();
    }
}

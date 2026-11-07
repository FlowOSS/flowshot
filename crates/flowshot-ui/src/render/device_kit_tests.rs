//! The kit-sharing contract: windows that draw with one kit draw with the same
//! device resources, while the per-window half (the glyph atlas bind group) stays
//! distinct. Like the parity harness, this SKIPS with a message when no GPU
//! adapter is usable.

use crate::gpu::{GpuContext, new_instance};

use super::device_kit::DeviceKit;
use super::image::TextureStore;
use super::renderer::Renderer;
use super::text::TextStack;

const TEST_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn gpu_or_skip() -> Option<GpuContext> {
    let instance = new_instance();
    match GpuContext::new_headless(&instance) {
        Ok(ctx) => Some(ctx),
        Err(error) => {
            eprintln!("SKIP (no usable GPU adapter): {error}");
            None
        }
    }
}

#[test]
fn one_kit_is_reused_by_every_window_instead_of_rebuilt() {
    let Some(gpu) = gpu_or_skip() else {
        return;
    };
    let kit = DeviceKit::new(&gpu.device, &gpu.queue, TEST_FORMAT);
    let first = TextStack::new(&kit);
    let second = TextStack::new(&kit);
    assert_eq!(
        first.pipeline(),
        &kit.text_pipeline,
        "a window must draw text with the kit's pipeline, not one it built itself"
    );
    assert_eq!(
        first.pipeline(),
        second.pipeline(),
        "every window must draw text with the same pipeline"
    );
    assert_eq!(
        TextureStore::new(&kit).pipeline(),
        &kit.image_pipeline,
        "image quads must draw with the kit's pipeline"
    );
    let renderer = Renderer::with_kit(&kit);
    assert_eq!(
        renderer.textures().pipeline(),
        &kit.image_pipeline,
        "the renderer must draw image quads with the kit's pipeline"
    );
    assert_eq!(
        renderer.format(),
        kit.format,
        "the renderer reports the kit's attachment format"
    );
    assert_ne!(
        first.bind_group(),
        second.bind_group(),
        "the glyph atlas stays per window"
    );

    // The equality assertions above can genuinely fail: a second kit built on the
    // same device is a second set of device resources.
    let control = DeviceKit::new(&gpu.device, &gpu.queue, TEST_FORMAT);
    assert_ne!(
        kit.text_pipeline, control.text_pipeline,
        "control: a separately built kit is a different pipeline"
    );
    assert_ne!(
        kit.glyph_sampler, control.glyph_sampler,
        "control: a separately built kit is a different sampler"
    );
}

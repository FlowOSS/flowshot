//! Render-pass step dispatch: maps recorded [`Step`]s to pipeline, stencil
//! reference, bind group, and indexed draw.

use super::frame_build::{FlatMode, Step};
use super::image::TextureStore;
use super::shadow::ShadowPipeline;
use super::staging::UploadedBuffers;
use super::text::TextStack;
use super::vector::VectorPipelines;

/// The pipeline set a pass draws with, gathered for step dispatch.
pub(super) struct PipelineRefs<'a> {
    pub(super) vector: &'a VectorPipelines,
    pub(super) shadow: &'a ShadowPipeline,
    pub(super) text: &'a TextStack,
    pub(super) textures: &'a TextureStore,
}

pub(super) fn draw_step<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    step: &Step,
    uploaded: &UploadedBuffers<'pass>,
    pipelines: &PipelineRefs<'pass>,
) {
    let (pipeline, vertex, index, bind_group, indices, clip) = match step {
        Step::Flat {
            indices,
            clip,
            mode,
        } => {
            let pipeline = match mode {
                FlatMode::Content => &pipelines.vector.content,
                FlatMode::ClipPush => &pipelines.vector.clip_push,
                FlatMode::ClipPop => &pipelines.vector.clip_pop,
                FlatMode::Invert => &pipelines.vector.invert,
            };
            (
                pipeline,
                uploaded.flat_vertex,
                uploaded.flat_index,
                None,
                indices,
                *clip,
            )
        }
        Step::Image {
            indices,
            texture,
            clip,
        } => (
            pipelines.textures.pipeline(),
            uploaded.image_vertex,
            uploaded.image_index,
            Some(pipelines.textures.resolve(*texture)),
            indices,
            *clip,
        ),
        Step::Shadow { indices, clip } => (
            &pipelines.shadow.pipeline,
            uploaded.shadow_vertex,
            uploaded.shadow_index,
            None,
            indices,
            *clip,
        ),
        Step::Text { indices, clip } => (
            pipelines.text.pipeline(),
            uploaded.text_vertex,
            uploaded.text_index,
            Some(pipelines.text.bind_group()),
            indices,
            *clip,
        ),
    };
    let (Some(vertex), Some(index)) = (vertex, index) else {
        return;
    };
    pass.set_pipeline(pipeline);
    pass.set_stencil_reference(clip);
    if let Some(bind_group) = bind_group {
        pass.set_bind_group(0, bind_group, &[]);
    }
    pass.set_vertex_buffer(0, vertex.slice(..));
    pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(indices.clone(), 0, 0..1);
}

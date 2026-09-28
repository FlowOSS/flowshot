//! Display-list walk: converts commands into batched staging geometry and
//! the ordered [`Step`] list the pass replays. Owns the clip-stack state
//! machine (stencil reference bookkeeping, push/pop geometry, depth limit).

use std::ops::Range;

use lyon::tessellation::VertexBuffers;

use super::glyph::TextVertex;
use super::image::{ImageVertex, TextureStore, image_quad, uv_rect};
use super::list::{ClipCommand, Command, ImageCommand, Shape, TextureId};
use super::shadow::{ShadowVertex, shadow_quad};
use super::tess::{FlatVertex, Tessellator};
use super::text::TextStack;

/// Clip depths beyond this are ignored (stencil is 8-bit; overlay UIs nest
/// at most a few levels).
const MAX_CLIP_DEPTH: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FlatMode {
    Content,
    ClipPush,
    ClipPop,
    Invert,
}

/// The invert blend's source color: unit white, so `src * (1 - dst)` is the
/// exact per-channel complement (the blend state does the inversion, the
/// geometry only carries coverage).
const INVERT_SRC: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

#[derive(Debug, Clone)]
pub(super) enum Step {
    Flat {
        indices: Range<u32>,
        clip: u32,
        mode: FlatMode,
    },
    Image {
        indices: Range<u32>,
        texture: TextureId,
        clip: u32,
    },
    Shadow {
        indices: Range<u32>,
        clip: u32,
    },
    Text {
        indices: Range<u32>,
        clip: u32,
    },
}

/// Mutable walk state for one display list: the staging sinks, the clip
/// stack, and borrowed references to the subsystems commands need.
pub(super) struct FrameBuild<'a> {
    pub(super) queue: &'a wgpu::Queue,
    pub(super) steps: &'a mut Vec<Step>,
    pub(super) flat: &'a mut VertexBuffers<FlatVertex, u32>,
    pub(super) image_vertices: &'a mut Vec<ImageVertex>,
    pub(super) image_indices: &'a mut Vec<u32>,
    pub(super) shadow_vertices: &'a mut Vec<ShadowVertex>,
    pub(super) shadow_indices: &'a mut Vec<u32>,
    pub(super) text_vertices: &'a mut Vec<TextVertex>,
    pub(super) text_indices: &'a mut Vec<u32>,
    pub(super) tessellator: &'a mut Tessellator,
    pub(super) text: &'a mut TextStack,
    pub(super) textures: &'a TextureStore,
    pub(super) clip_depth: u32,
    pub(super) clip_stack: Vec<ClipCommand>,
}

impl FrameBuild<'_> {
    pub(super) fn command(&mut self, command: &Command) {
        match command {
            Command::Fill { shape, color } => {
                self.flat_content(shape, color.premultiplied_linear(), None);
            }
            Command::Stroke {
                shape,
                width,
                color,
            } => {
                self.flat_content(shape, color.premultiplied_linear(), Some(*width));
            }
            Command::Dim {
                bounds,
                cutouts,
                color,
            } => {
                let start = self.flat.indices.len();
                self.tessellator
                    .dim(*bounds, cutouts, color.premultiplied_linear(), self.flat);
                self.push_flat(start, self.clip_depth, FlatMode::Content);
            }
            Command::Invert { rect } => {
                let start = self.flat.indices.len();
                let shape = Shape::Rect {
                    rect: *rect,
                    radius: 0.0,
                };
                self.tessellator.fill_shape(&shape, INVERT_SRC, self.flat);
                self.push_flat(start, self.clip_depth, FlatMode::Invert);
            }
            Command::Image(command) => self.image(command),
            Command::Shadow { rect, radius, spec } => {
                let Some((quad, indices)) = shadow_quad(*rect, *radius, spec) else {
                    tracing::debug!("degenerate shadow skipped");
                    return;
                };
                append_quads(self.shadow_vertices, self.shadow_indices, &quad, &indices);
                self.steps.push(Step::Shadow {
                    indices: index_range(self.shadow_indices, indices.len()),
                    clip: self.clip_depth,
                });
            }
            Command::Text(command) => {
                let start = self.text_indices.len();
                self.text
                    .prepare(self.queue, command, self.text_vertices, self.text_indices);
                if self.text_indices.len() > start {
                    self.steps.push(Step::Text {
                        indices: u32_range(start, self.text_indices.len()),
                        clip: self.clip_depth,
                    });
                }
            }
            Command::PushClip(clip) => self.push_clip(*clip),
            Command::PopClip => self.pop_clip(),
        }
    }

    /// `width` selects stroking; `None` fills.
    fn flat_content(&mut self, shape: &Shape, color: [f32; 4], width: Option<f32>) {
        let start = self.flat.indices.len();
        match width {
            Some(width) => self
                .tessellator
                .stroke_shape(shape, width, color, self.flat),
            None => self.tessellator.fill_shape(shape, color, self.flat),
        }
        self.push_flat(start, self.clip_depth, FlatMode::Content);
    }

    fn image(&mut self, command: &ImageCommand) {
        let dimensions = self.textures.dimensions(command.texture).unwrap_or((1, 1));
        let uv = uv_rect(command.src, dimensions.0, dimensions.1);
        let (quad, indices) = image_quad(command.dst, uv, command.alpha);
        append_quads(self.image_vertices, self.image_indices, &quad, &indices);
        self.steps.push(Step::Image {
            indices: index_range(self.image_indices, indices.len()),
            texture: command.texture,
            clip: self.clip_depth,
        });
    }

    fn push_clip(&mut self, clip: ClipCommand) {
        self.clip_stack.push(clip);
        if u32::try_from(self.clip_stack.len()).unwrap_or(u32::MAX) > MAX_CLIP_DEPTH {
            tracing::warn!(
                limit = MAX_CLIP_DEPTH,
                "clip depth limit reached; deeper clips are no-ops"
            );
            return;
        }
        // Stencil-only geometry: the clip pipelines mask color writes, so
        // the vertex color is inert.
        self.clip_geometry(clip, FlatMode::ClipPush, self.clip_depth);
        self.clip_depth += 1;
    }

    fn pop_clip(&mut self) {
        let Some(clip) = self.clip_stack.pop() else {
            tracing::debug!("unbalanced clip pop ignored");
            return;
        };
        let remaining = u32::try_from(self.clip_stack.len()).unwrap_or(u32::MAX);
        if remaining >= MAX_CLIP_DEPTH {
            return;
        }
        self.clip_depth -= 1;
        // The pop restores the parent depth: test against the child depth
        // (clip_depth + 1) and decrement.
        self.clip_geometry(clip, FlatMode::ClipPop, self.clip_depth + 1);
    }

    fn clip_geometry(&mut self, clip: ClipCommand, mode: FlatMode, reference: u32) {
        let start = self.flat.indices.len();
        let shape = Shape::Rect {
            rect: clip.rect,
            radius: clip.radius,
        };
        self.tessellator.fill_shape(&shape, [0.0; 4], self.flat);
        self.push_flat(start, reference, mode);
    }

    fn push_flat(&mut self, start: usize, clip: u32, mode: FlatMode) {
        if self.flat.indices.len() > start {
            self.steps.push(Step::Flat {
                indices: u32_range(start, self.flat.indices.len()),
                clip,
                mode,
            });
        }
    }
}

fn append_quads<V: Copy>(
    vertices: &mut Vec<V>,
    indices: &mut Vec<u32>,
    quad: &[V],
    quad_indices: &[u32],
) {
    let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
    vertices.extend_from_slice(quad);
    indices.extend(quad_indices.iter().map(|index| index + base));
}

fn index_range(indices: &[u32], added: usize) -> Range<u32> {
    u32_range(indices.len() - added, indices.len())
}

fn u32_range(start: usize, end: usize) -> Range<u32> {
    let start = u32::try_from(start).unwrap_or(u32::MAX);
    let end = u32::try_from(end).unwrap_or(u32::MAX);
    start..end
}

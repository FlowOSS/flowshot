//! The batched 2D renderer: vector geometry, text, images,
//! and effects on wgpu - a pure draw-command consumer.
//!
//! # Architecture
//!
//! Consumers (overlay, widgets, editor) build a [`DisplayList`]
//! of [`Command`]s in **physical pixels** per frame; [`Renderer::render`]
//! executes it into any color attachment (the live overlay surface or an
//! offscreen texture). No scene or editor semantics live here.
//!
//! - **Vector** ([`Shape`], fills/strokes/dim): lyon tessellation into a
//!   flat premultiplied-color triangle mesh; antialiasing comes from the 4x
//!   multisampled frame target. Stroke widths are physical px.
//! - **Text** ([`TextCommand`]): cosmic-text shaping + swash rasterization
//!   through an in-crate texture atlas (atlas-first per Metis #17; no glyphon
//!   release pairs with the wgpu 0.20 pin - see the root manifest note).
//! - **Images** ([`super::render::TextureStore`]): frozen-frame textures,
//!   linear filtering, no mips, pixel sub-region sampling for the magnifier.
//! - **Effects**: even-odd dim cutout, single-pass SDF gaussian drop
//!   shadows, and a stencil-backed rounded-clip stack.
//!
//! # Color
//!
//! All colors arrive as design-token hex via [`Color`]; vertices carry
//! premultiplied **linear** light and every target is an sRGB format, so
//! opaque flat fills reproduce token bytes exactly.
//!
//! # Failure behavior
//!
//! Degenerate geometry is skipped with a tracing log; a missing image texture
//! draws the magenta placeholder and logs an error. The renderer never
//! panics - the crate-wide rule.

mod atlas;
mod color;
mod frame_build;
mod geom;
mod glyph;
mod image;
mod image_pipeline;
mod list;
mod pass;
mod readback;
mod renderer;
mod shadow;
mod staging;
mod target;
mod tess;
mod text;
mod text_pipeline;
mod vector;

pub use color::{Color, linear_to_srgb, srgb_to_linear};
pub use geom::{Point, Rect, Size};
pub(crate) use geom::{f32_from_f64, f32_from_i32, f32_from_u32};
pub use image::{RgbaImage, TextureStore};
pub use list::{
    ClipCommand, Command, DisplayList, ImageCommand, PathSegment, ShadowSpec, Shape, TextCommand,
    TextureId,
};
pub use readback::read_texture_rgba;
pub use renderer::{FrameStats, RenderTarget, Renderer};

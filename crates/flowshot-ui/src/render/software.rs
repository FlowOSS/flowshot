//! CPU raster path: the fallback renderer for GPU-less or memory-starved desktops.
//!
//! The GPU renderer's cost model is a GPU problem twice over: an 8x-multisampled
//! offscreen color target plus a `Stencil8` attachment at the window extent
//! (measured: ~230 MB at the 4480x1440 dual-monitor union) and a Vulkan-side
//! `encode+submit` that a software driver executes at ~63 ms/frame. Both are
//! what makes an OOM-starved desktop die at `request_device` or at the swapchain
//! allocation. This renderer keeps the same input — the [`DisplayList`]
//! vocabulary — and replaces only the rasterizer: it paints into a CPU-side
//! RGBA8 framebuffer and the caller presents that buffer as one textured quad.
//!
//! # Color contract
//!
//! The framebuffer is **premultiplied, sRGB-encoded** RGBA8 — the byte space the
//! GPU path writes into its sRGB target, so the two paths agree on opaque token
//! fills byte for byte. Antialiasing is coverage-based rather than
//! multisample-based, so translucent overlaps blend in the encoded space
//! instead of linear light: the difference is confined to antialiased edges,
//! the same band the cross-rasterizer parity policy masks.
//!
//! # Failure behavior
//!
//! Identical to the GPU renderer's contract: degenerate geometry is skipped with
//! a tracing log, a missing image texture draws the magenta placeholder, and
//! nothing here panics.
//!
//! # Speed
//!
//! Every operation is bounding-box limited and the clip mask is materialized
//! only while a clip is active, so frame cost tracks ink area rather than the
//! window extent.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tiny_skia::{FillRule, Transform};

use cosmic_text::{FontSystem, SwashCache};

mod blit;
mod buffer;
mod presenter;
pub use presenter::Presenter;
mod shadow;
mod text;
mod vector;

use super::list::{ClipCommand, Command, DisplayList, TextureId};
use super::software::blit::blit;
use super::software::buffer::{ClipMask, full_mask};
use super::software::shadow::shadow;
use super::software::text::text;
use super::software::vector::{clip_path, dim, fill, invert, stroke};
use super::stats::FrameStats;

/// One frame's CPU-side image source, keyed by the display list's
/// [`super::list::TextureId`]: raw RGBA8 bytes plus their extent.
///
/// The GPU path uploads these to textures once per capture; the CPU path keeps
/// the bytes and samples them directly, so no upload happens at all.
#[derive(Debug, Clone)]
pub struct ImageSource {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Row-major RGBA8 bytes, premultiplied, sRGB-encoded; exactly
    /// `width * height * 4` long.
    pub data: Vec<u8>,
}

impl ImageSource {
    /// The source extent in texels.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The source pixel bytes.
    #[must_use]
    pub const fn bytes(&self) -> &[u8] {
        self.data.as_slice()
    }
}

/// The CPU raster renderer: the same pure draw-command consumer as the GPU
/// [`super::renderer::Renderer`], rasterizing into a CPU framebuffer.
#[derive(Debug)]
pub struct SoftwareRenderer {
    width: u32,
    height: u32,
    buffer: buffer::SoftwareFramebuffer,
    clip: Vec<ClipMask>,
    fonts: (FontSystem, SwashCache),
    images: HashMap<TextureId, ImageSource>,
}

impl SoftwareRenderer {
    /// A renderer for `width` x `height` physical px frames.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Self {
            width,
            height,
            buffer: buffer::SoftwareFramebuffer::new(width, height),
            clip: Vec::new(),
            fonts: (FontSystem::new(), SwashCache::new()),
            images: HashMap::new(),
        }
    }

    /// Registers one image source for the [`super::list::TextureId`] handles the
    /// consumer issues. Re-registering the same handle replaces the bytes.
    pub fn upload(&mut self, texture: TextureId, source: ImageSource) {
        self.images.insert(texture, source);
    }

    /// Drops every registered image source (buffer reuse across captures).
    pub fn clear_images(&mut self) {
        self.images.clear();
    }

    /// The framebuffer extent in physical pixels.
    #[must_use]
    pub(crate) const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The premultiplied sRGB-encoded RGBA8 bytes of the last rendered frame.
    #[must_use]
    pub(crate) fn pixels(&self) -> &[u8] {
        self.buffer.data()
    }

    /// Renders one frame of `list` into the CPU framebuffer.
    #[must_use = "the frame stats are what the perf budget is checked against"]
    pub fn render(&mut self, list: &DisplayList) -> FrameStats {
        let started = Instant::now();
        self.buffer.clear();
        self.clip.clear();
        for command in list {
            match command {
                Command::Fill { shape, color } => fill(&mut self.buffer, &self.clip, shape, color),
                Command::Stroke {
                    shape,
                    width,
                    color,
                } => stroke(&mut self.buffer, &self.clip, shape, *width, color),
                Command::Dim {
                    bounds,
                    cutouts,
                    color,
                } => dim(&mut self.buffer, &self.clip, bounds, cutouts, color),
                Command::Invert { rect } => invert(&mut self.buffer, &self.clip, rect),
                Command::Image(image) => blit(&mut self.buffer, &self.clip, image, &self.images),
                Command::Shadow { rect, radius, spec } => {
                    shadow(&mut self.buffer, &self.clip, rect, *radius, spec);
                }
                Command::Text(command) => {
                    text(&mut self.buffer, &self.clip, &mut self.fonts, command);
                }
                Command::PushClip(clip) => self.push_clip(clip),
                Command::PopClip => {
                    self.clip.pop();
                }
            }
        }
        let walk = started.elapsed();
        FrameStats {
            commands: list.len(),
            vertices: 0,
            draws: list.len(),
            build_time: walk,
            cpu_time: walk,
            acquire_time: Duration::ZERO,
        }
    }

    /// Pushes a rounded-rect clip onto the clip stack.
    pub fn push_clip(&mut self, clip: &ClipCommand) {
        let Some(path) = clip_path(clip) else {
            return;
        };
        let mask = if self.clip.is_empty() {
            full_mask(self.width, self.height)
        } else {
            self.clip.last().cloned()
        };
        if let Some(mut mask) = mask {
            mask.intersect_path(&path, FillRule::Winding, true, Transform::identity());
            self.clip.push(mask);
        } else {
            tracing::error!("cpu renderer could not size a clip mask for the frame extent");
        }
    }

    /// Pops the most recent clip; a no-op when the stack is empty.
    pub fn pop_clip(&mut self) {
        self.clip.pop();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::color::Color;
    use crate::render::geom::{Point, Rect};
    use crate::render::list::Shape;

    #[test]
    fn opaque_token_fill_is_byte_exact() {
        let mut renderer = SoftwareRenderer::new(4, 4);
        let mut list = DisplayList::new();
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(0.0, 0.0, 4.0, 4.0),
                radius: 0.0,
            },
            Color::from_rgba8(255, 0, 0, 255),
        );
        let _stats = renderer.render(&list);
        let pixels = renderer.pixels();
        for offset in 0..16 {
            let at = offset * 4;
            assert_eq!(
                [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]],
                [255, 0, 0, 255]
            );
        }
    }

    #[test]
    fn degenerate_geometry_paints_nothing_and_never_panics() {
        let mut renderer = SoftwareRenderer::new(2, 2);
        let mut list = DisplayList::new();
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(0.0, 0.0, 0.0, 4.0),
                radius: 0.0,
            },
            Color::from_rgba8(255, 0, 0, 255),
        );
        list.stroke(
            Shape::Line {
                from: Point::new(0.0, 0.0),
                to: Point::new(0.0, 0.0),
            },
            0.0,
            Color::from_rgba8(0, 255, 0, 255),
        );
        let _stats = renderer.render(&list);
        assert_eq!(renderer.pixels(), vec![0u8; 16]);
    }

    #[test]
    fn clip_punches_only_the_clipped_region() {
        let mut renderer = SoftwareRenderer::new(4, 4);
        let mut list = DisplayList::new();
        list.push_clip(Rect::from_parts(0.0, 0.0, 2.0, 2.0), 0.0);
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(0.0, 0.0, 4.0, 4.0),
                radius: 0.0,
            },
            Color::from_rgba8(0, 0, 255, 255),
        );
        list.pop_clip();
        let _stats = renderer.render(&list);
        let pixels = renderer.pixels();
        for offset in 0..16 {
            let (x, y) = (offset % 4, offset / 4);
            let at = offset * 4;
            if x < 2 && y < 2 {
                assert_eq!(
                    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]],
                    [0, 0, 255, 255]
                );
            } else {
                assert_eq!(
                    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]],
                    [0, 0, 0, 0]
                );
            }
        }
    }

    #[test]
    fn invert_complements_painted_pixels() {
        let mut renderer = SoftwareRenderer::new(2, 2);
        let mut list = DisplayList::new();
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(0.0, 0.0, 2.0, 2.0),
                radius: 0.0,
            },
            Color::from_rgba8(255, 0, 0, 255),
        );
        list.invert(Rect::from_parts(0.0, 0.0, 2.0, 2.0));
        let _stats = renderer.render(&list);
        let pixels = renderer.pixels();
        for offset in 0..4 {
            let at = offset * 4;
            assert_eq!(
                [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]],
                [0, 255, 255, 255]
            );
        }
    }

    #[test]
    fn a_1_to_1_blit_copies_the_source_bytes() {
        let source = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let mut renderer = SoftwareRenderer::new(2, 2);
        renderer.upload(
            TextureId::new(1),
            ImageSource {
                width: 2,
                height: 2,
                data: source.to_vec(),
            },
        );
        let mut list = DisplayList::new();
        list.image(
            TextureId::new(1),
            Rect::from_parts(0.0, 0.0, 2.0, 2.0),
            None,
        );
        let _stats = renderer.render(&list);
        let pixels = renderer.pixels();
        for offset in 0..4 {
            let at = offset * 4;
            assert_eq!(
                [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]],
                [source[at], source[at + 1], source[at + 2], source[at + 3]]
            );
        }
    }

    #[test]
    fn a_missing_texture_draws_the_magenta_placeholder() {
        let mut renderer = SoftwareRenderer::new(2, 2);
        let mut list = DisplayList::new();
        list.image(
            TextureId::new(9),
            Rect::from_parts(0.0, 0.0, 2.0, 2.0),
            None,
        );
        let _stats = renderer.render(&list);
        let pixels = renderer.pixels();
        for offset in 0..4 {
            let at = offset * 4;
            assert_eq!(
                [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]],
                [255, 0, 255, 255]
            );
        }
    }
}

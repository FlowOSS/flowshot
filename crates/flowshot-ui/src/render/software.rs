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
//! The framebuffer is **premultiplied, linear-light** RGBA8 — the space the GPU
//! path blends in. The presenter stores it into an sRGB target, whose store
//! re-encodes, so the two paths agree byte for byte in the target and opaque
//! token fills round-trip through that encode exactly. Antialiasing is
//! coverage-based rather than multisample-based, so edge coverage quantizes
//! differently from MSAA — the band the cross-rasterizer parity policy masks.
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
mod regions;
use super::software::regions::{
    Region, command_bounds, intersect_rect, region_box, region_rect, union_rect, union_region,
};
pub(crate) use buffer::unit_to_byte;
mod shadow;
mod text;
mod vector;

use super::color::Color;
use super::geom::{Rect, Size};
use super::list::{ClipCommand, Command, DisplayList, ImageCommand, TextureId};
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

/// A registered image source: the decoded bytes plus the one property the
/// blit's row-copy fast path needs (every texel opaque), computed once at
/// registration rather than per frame.
#[derive(Debug, Clone)]
struct ImageEntry {
    width: u32,
    height: u32,
    data: Vec<u8>,
    opaque: bool,
}

impl ImageEntry {
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

    /// Whether every texel is fully opaque.
    #[must_use]
    pub const fn opaque(&self) -> bool {
        self.opaque
    }
}

/// The CPU raster renderer: the same pure draw-command consumer as the GPU
/// [`super::renderer::Renderer`], rasterizing into a CPU framebuffer.
/// The backdrop pair's identity: the texture, the wash's bounds, holes, and
/// color. A frame whose head matches this key reuses the cached base.
#[derive(Debug, Clone, PartialEq)]
struct WashKey {
    texture: TextureId,
    bounds: Rect,
    cutouts: Vec<Rect>,
    color: Color,
}

/// One window's software fallback renderer: paints into a CPU-side
/// premultiplied linear-light RGBA8 framebuffer.
#[derive(Debug)]
pub struct SoftwareRenderer {
    width: u32,
    height: u32,
    buffer: buffer::SoftwareFramebuffer,
    base: buffer::SoftwareFramebuffer,
    wash: Option<WashKey>,
    wash_region: Option<Region>,
    prev_chrome: Option<Vec<Command>>,
    last_damage: Option<Region>,
    scene_key: Option<Vec<Command>>,
    clip: Vec<ClipMask>,
    fonts: (FontSystem, SwashCache),
    images: HashMap<TextureId, ImageEntry>,
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
            base: buffer::SoftwareFramebuffer::new(width, height),
            wash: None,
            wash_region: None,
            prev_chrome: None,
            last_damage: None,
            scene_key: None,
            clip: Vec::new(),
            fonts: (FontSystem::new(), SwashCache::new()),
            images: HashMap::new(),
        }
    }

    /// Registers one image source for the [`super::list::TextureId`] handles the
    /// consumer issues. Re-registering the same handle replaces the bytes.
    ///
    /// The bytes arrive in the capture's space (premultiplied, sRGB-encoded) and
    /// are decoded **once** here into the framebuffer's premultiplied linear-light
    /// space, so a blit never crosses the transfer function per texel per frame —
    /// that decode is what put a full-window backdrop blit at hundreds of
    /// milliseconds rather than the copy's own few.
    pub fn upload(&mut self, texture: TextureId, source: ImageSource) {
        let mut source = source;
        let table = crate::render::software::blit::linear_table();
        for texel in source.data.as_chunks_mut::<4>().0 {
            for channel in 0..3 {
                texel[channel] = table[usize::from(texel[channel])];
            }
        }
        let opaque = source
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .all(|texel| texel[3] == 255);
        self.images.insert(
            texture,
            ImageEntry {
                width: source.width,
                height: source.height,
                data: source.data,
                opaque,
            },
        );
    }

    /// Drops every registered image source (buffer reuse across captures).
    pub fn clear_images(&mut self) {
        self.images.clear();
    }

    /// Reallocates the framebuffer when the window extent changes, so the
    /// presenter's 1:1 copy never stretches a stale-extent frame onto a new
    /// swapchain. Unchanged extents are a no-op; a resize drops any clip
    /// stack left over from the abandoned frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.size() == (width.max(1), height.max(1)) {
            return;
        }
        self.width = width.max(1);
        self.height = height.max(1);
        self.buffer = buffer::SoftwareFramebuffer::new(self.width, self.height);
        self.base = buffer::SoftwareFramebuffer::new(self.width, self.height);
        self.wash = None;
        self.wash_region = None;
        self.prev_chrome = None;
        self.last_damage = None;
        self.scene_key = None;
        self.clip.clear();
    }

    /// The framebuffer extent in physical pixels.
    #[must_use]
    pub(crate) const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The sub-region damaged in the most recent frame, or `None` when the
    /// whole frame was repainted from scratch.
    #[must_use]
    pub fn last_damage(&self) -> Option<Region> {
        self.last_damage
    }

    /// The framebuffer pixel bytes.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        self.buffer.data()
    }

    /// Renders one frame of `list` into the CPU framebuffer.
    #[must_use = "the frame stats are what the perf budget is checked against"]
    pub fn render(&mut self, list: &DisplayList) -> FrameStats {
        let started = Instant::now();
        self.clip.clear();
        if let Some(damage) = self.cached_frame(list) {
            self.last_damage = Some(damage);
            self.buffer.copy_region_from(&self.base, damage);
            let scene_end = list.scene_end().max(2).min(list.len());
            for command in &list.as_slice()[scene_end..] {
                self.paint(command);
            }
        } else {
            self.last_damage = None;
            self.buffer.clear();
            for command in list {
                self.paint(command);
            }
        }
        let walk = started.elapsed();
        FrameStats {
            commands: list.len(),
            vertices: 0,
            draws: 0,
            build_time: walk,
            cpu_time: walk,
            acquire_time: Duration::ZERO,
        }
    }

    #[expect(
        clippy::too_many_lines,
        clippy::manual_let_else,
        clippy::question_mark,
        reason = "cached frame sequence pipeline"
    )]
    #[must_use]
    fn cached_frame(&mut self, list: &DisplayList) -> Option<Region> {
        let extent = Size::new(
            crate::render::geom::f32_from_u32(self.width),
            crate::render::geom::f32_from_u32(self.height),
        );
        let window = ((0, 0), (self.width, self.height));
        let image = match list.iter().next() {
            Some(Command::Image(image)) => image,
            _ => return None,
        };
        let (bounds, cutouts, color) = match list.iter().nth(1) {
            Some(Command::Dim {
                bounds,
                cutouts,
                color,
            }) => (bounds, cutouts, color),
            _ => return None,
        };
        let entry = match self.images.get(&image.texture) {
            Some(entry) => entry,
            None => return None,
        };
        let (tex_w, tex_h) = entry.size();
        let src_region = image.src.unwrap_or(Rect::from_parts(
            0.0,
            0.0,
            crate::render::geom::f32_from_u32(tex_w),
            crate::render::geom::f32_from_u32(tex_h),
        ));
        let scale_x = src_region.size.width / image.dst.size.width;
        let scale_y = src_region.size.height / image.dst.size.height;
        #[expect(
            clippy::float_cmp,
            reason = "both derive from integer physical extents, so 1.0 is exact"
        )]
        let one_to_one = scale_x == 1.0 && scale_y == 1.0;
        if !self.clip.is_empty() || image.alpha < 1.0 || !entry.opaque() || !one_to_one {
            return None;
        }
        let wash_box = match region_box(*bounds, extent) {
            Some(area) => area,
            None => return None,
        };
        let key = WashKey {
            texture: image.texture,
            bounds: *bounds,
            cutouts: cutouts.clone(),
            color: *color,
        };
        let contained = match intersect_rect(region_rect(wash_box), *bounds) {
            Some(area) => area,
            None => *bounds,
        };
        if contained != *bounds {
            return None;
        }

        let scene_end = list.scene_end().max(2).min(list.len());
        let scene_slice = &list.as_slice()[0..scene_end];

        let changed = match self.wash {
            Some(ref stored) => *stored != key,
            None => true,
        };
        let scene_changed = match self.scene_key.as_deref() {
            Some(stored) => stored != scene_slice,
            None => true,
        };
        let is_extension = match self.scene_key.as_deref() {
            Some(stored) => scene_slice.starts_with(stored),
            None => false,
        };

        let mut damage: Option<Region> = None;

        if is_extension && !changed && scene_changed {
            // Incremental commit fast-path: backdrop and wash are identical!
            // Paint only the newly added scene commands onto self.base without full-screen re-rasterization.
            let old_len = self.scene_key.as_ref().map_or(2, Vec::len);
            let mut base_clip = Vec::new();
            let mut new_bounds: Option<Rect> = None;
            for cmd in &list.as_slice()[old_len..scene_end] {
                if let Some(b) = command_bounds(cmd) {
                    new_bounds = union_rect(new_bounds, b);
                }
                Self::paint_to(
                    &mut self.base,
                    &mut base_clip,
                    &mut self.fonts,
                    &self.images,
                    cmd,
                );
            }
            self.scene_key = Some(scene_slice.to_vec());
            if let Some(nb) = new_bounds.and_then(|r| region_box(r, extent)) {
                damage = Some(nb);
            }
        } else if changed || scene_changed {
            // Full re-render of base: initial frame, selection moved/resized, or undo/redo.
            let region = if scene_changed {
                window
            } else {
                match self.wash {
                    Some(ref stored)
                        if stored.bounds == *bounds
                            && stored.color == *color
                            && stored.texture == image.texture =>
                    {
                        let area = stored
                            .cutouts
                            .iter()
                            .fold(None, |acc, hole| union_rect(acc, *hole));
                        let area = cutouts
                            .iter()
                            .fold(area, |acc, hole| union_rect(acc, *hole));
                        let holes = match area {
                            Some(area) => region_box(area, extent),
                            None => None,
                        };
                        match holes {
                            Some(area) => area,
                            None => wash_box,
                        }
                    }
                    _ => match self.wash_region {
                        Some(previous) => union_region(previous, wash_box),
                        None => window,
                    },
                }
            };
            let clipped = ImageCommand {
                texture: image.texture,
                dst: region_rect(region),
                src: Some(region_rect(region)),
                alpha: 1.0,
            };
            crate::render::software::blit::blit(&mut self.base, &[], &clipped, &self.images);
            dim(&mut self.base, &[], &region_rect(region), cutouts, color);
            let mut base_clip = Vec::new();
            for cmd in &list.as_slice()[2..scene_end] {
                Self::paint_to(
                    &mut self.base,
                    &mut base_clip,
                    &mut self.fonts,
                    &self.images,
                    cmd,
                );
            }
            self.wash = Some(key);
            self.wash_region = Some(region);
            self.scene_key = Some(scene_slice.to_vec());
            damage = Some(region);
        }

        // Fine-grained chrome damage: diff the commands in chrome between frames
        let prev_chrome = self.prev_chrome.as_deref().unwrap_or(&[]);
        let curr_chrome = &list.as_slice()[scene_end..];
        let mut chrome_dirty: Option<Rect> = None;

        for (i, cmd) in curr_chrome.iter().enumerate() {
            if prev_chrome.get(i) != Some(cmd)
                && let Some(b) = command_bounds(cmd)
            {
                chrome_dirty = union_rect(chrome_dirty, b);
            }
        }
        for (i, cmd) in prev_chrome.iter().enumerate() {
            if curr_chrome.get(i) != Some(cmd)
                && let Some(b) = command_bounds(cmd)
            {
                chrome_dirty = union_rect(chrome_dirty, b);
            }
        }
        self.prev_chrome = Some(curr_chrome.to_vec());
        let chrome_region = chrome_dirty.and_then(|r| region_box(r, extent));

        match (damage, chrome_region) {
            (Some(d), Some(c)) => Some(union_region(d, c)),
            (Some(d), None) => Some(d),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        }
    }

    /// Paints one command into the framebuffer under the active clip stack.
    fn paint(&mut self, command: &Command) {
        Self::paint_to(
            &mut self.buffer,
            &mut self.clip,
            &mut self.fonts,
            &self.images,
            command,
        );
    }

    /// Paints one command into an arbitrary framebuffer under a given clip stack.
    fn paint_to(
        fb: &mut buffer::SoftwareFramebuffer,
        clip: &mut Vec<ClipMask>,
        fonts: &mut (FontSystem, SwashCache),
        images: &HashMap<TextureId, ImageEntry>,
        command: &Command,
    ) {
        match command {
            Command::Fill { shape, color } => fill(fb, clip, shape, color),
            Command::Stroke {
                shape,
                width,
                color,
            } => stroke(fb, clip, shape, *width, color),
            Command::Dim {
                bounds,
                cutouts,
                color,
            } => dim(fb, clip, bounds, cutouts, color),
            Command::Invert { rect } => invert(fb, clip, rect),
            Command::Image(image) => crate::render::software::blit::blit(fb, clip, image, images),
            Command::Shadow { rect, radius, spec } => {
                shadow(fb, clip, rect, *radius, spec);
            }
            Command::Text(command) => {
                text(fb, clip, fonts, command);
            }
            Command::PushClip(clip_cmd) => {
                if let Some(path) = clip_path(clip_cmd)
                    && let Some(mask) = full_mask(fb.size().0, fb.size().1)
                {
                    let mut mask = if clip.is_empty() {
                        mask
                    } else {
                        clip.last().cloned().unwrap_or(mask)
                    };
                    mask.intersect_path(
                        &path,
                        tiny_skia::FillRule::Winding,
                        true,
                        tiny_skia::Transform::identity(),
                    );
                    clip.push(mask);
                }
            }
            Command::PopClip => {
                clip.pop();
            }
        }
    }

    /// Pushes a rounded-rect clip onto the active clip stack.
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
    use super::buffer::unit_to_byte;
    use super::*;
    use crate::render::color::Color;
    use crate::render::geom::{Point, Rect};
    use crate::render::list::Shape;

    #[test]
    fn the_opaque_fill_lands_in_linear_light() {
        // Given an opaque token fill, when the frame is rasterized, then the
        // framebuffer holds that color's premultiplied LINEAR bytes — the space
        // the GPU path blends in — so the presenter's sRGB store re-encodes to
        // the token bytes.
        let mut renderer = SoftwareRenderer::new(4, 4);
        let mut list = DisplayList::new();
        let color = Color::from_rgba8(99, 102, 241, 255);
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(0.0, 0.0, 4.0, 4.0),
                radius: 0.0,
            },
            color,
        );
        let _stats = renderer.render(&list);
        let pixels = renderer.pixels();
        let [r, g, b, a] = color.premultiplied_linear();
        let want = [r, g, b, a].map(unit_to_byte);
        for offset in 0..16 {
            let at = offset * 4;
            assert_eq!(
                [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]],
                want,
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

    #[test]
    fn scene_baking_preserves_pixel_equality_across_commits_and_undo() {
        let width = 64;
        let height = 48;
        let texture = TextureId::new(1);
        let data = vec![200u8; (width * height * 4) as usize];
        let mut cached = SoftwareRenderer::new(width, height);
        cached.upload(
            texture,
            ImageSource {
                width,
                height,
                data: data.clone(),
            },
        );

        let wash = Color::from_rgba8(15, 23, 42, 128);
        let rect1_color = Color::from_rgba8(255, 0, 0, 255);
        let rect2_color = Color::from_rgba8(0, 255, 0, 255);

        let make_list = |with_rect1: bool, with_rect2: bool| {
            let mut list = DisplayList::new();
            list.image(texture, Rect::from_parts(0.0, 0.0, 64.0, 48.0), None);
            list.dim(
                Rect::from_parts(0.0, 0.0, 64.0, 48.0),
                vec![Rect::from_parts(10.0, 10.0, 20.0, 20.0)],
                wash,
            );
            if with_rect1 {
                list.fill(
                    Shape::Rect {
                        rect: Rect::from_parts(12.0, 12.0, 8.0, 8.0),
                        radius: 0.0,
                    },
                    rect1_color,
                );
            }
            if with_rect2 {
                list.fill(
                    Shape::Rect {
                        rect: Rect::from_parts(22.0, 12.0, 8.0, 8.0),
                        radius: 0.0,
                    },
                    rect2_color,
                );
            }
            list.mark_scene_end();
            list.fill(
                Shape::Rect {
                    rect: Rect::from_parts(0.0, 0.0, 4.0, 4.0),
                    radius: 0.0,
                },
                Color::from_rgba8(255, 255, 255, 255),
            );
            list
        };

        // Step 1: Base scene only
        let list1 = make_list(false, false);
        let _ = cached.render(&list1);
        let mut fresh1 = SoftwareRenderer::new(width, height);
        fresh1.upload(
            texture,
            ImageSource {
                width,
                height,
                data: data.clone(),
            },
        );
        let _ = fresh1.render(&list1);
        assert_eq!(cached.pixels(), fresh1.pixels(), "step 1 match");

        // Step 2: Commit Shape 1
        let list2 = make_list(true, false);
        let _ = cached.render(&list2);
        let mut fresh2 = SoftwareRenderer::new(width, height);
        fresh2.upload(
            texture,
            ImageSource {
                width,
                height,
                data: data.clone(),
            },
        );
        let _ = fresh2.render(&list2);
        assert_eq!(cached.pixels(), fresh2.pixels(), "step 2 match");

        // Step 3: Commit Shape 2
        let list3 = make_list(true, true);
        let _ = cached.render(&list3);
        let mut fresh3 = SoftwareRenderer::new(width, height);
        fresh3.upload(
            texture,
            ImageSource {
                width,
                height,
                data: data.clone(),
            },
        );
        let _ = fresh3.render(&list3);
        assert_eq!(cached.pixels(), fresh3.pixels(), "step 3 match");

        // Step 4: Undo Shape 2 (Ctrl+Z)
        let list4 = make_list(true, false);
        let _ = cached.render(&list4);
        let mut fresh4 = SoftwareRenderer::new(width, height);
        fresh4.upload(
            texture,
            ImageSource {
                width,
                height,
                data: data.clone(),
            },
        );
        let _ = fresh4.render(&list4);
        assert_eq!(cached.pixels(), fresh4.pixels(), "step 4 match");

        // Step 5: Redo Shape 2
        let list5 = make_list(true, true);
        let _ = cached.render(&list5);
        let mut fresh5 = SoftwareRenderer::new(width, height);
        fresh5.upload(
            texture,
            ImageSource {
                width,
                height,
                data: data.clone(),
            },
        );
        let _ = fresh5.render(&list5);
        assert_eq!(cached.pixels(), fresh5.pixels(), "step 5 match");
    }
}

//! Text on the CPU path: cosmic-text shaping plus swash glyph masks composited
//! directly, the same mask source the GPU path uploads into its glyph atlas.
//!
//! The GPU path pays one atlas upload plus one batched draw; this path pays one
//! mask composite per inked glyph, which keeps the cost proportional to the
//! glyph's ink box rather than to the frame.

#![expect(
    clippy::cast_possible_truncation,
    reason = "glyph ink-box index math, clamped into range before each cast"
)]

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight,
};

use crate::render::geom::Point;
use crate::render::list::{TextAnchor, TextCommand};
use crate::render::software::buffer::{ClipMask, SoftwareFramebuffer, premultiplied};

/// Draws one shaped text run.
pub(crate) fn text(
    framebuffer: &mut SoftwareFramebuffer,
    clip: &[ClipMask],
    fonts: &mut (FontSystem, SwashCache),
    command: &TextCommand,
) {
    if command.font_size <= 0.0 || command.line_height <= 0.0 {
        return;
    }
    let (font_system, cache) = fonts;
    let mut buffer = Buffer::new(
        font_system,
        Metrics::new(command.font_size, command.line_height),
    );
    if let Some(width) = command.max_width {
        buffer.set_size(Some(width.max(0.0)), None);
    }
    let mut attrs = Attrs::new();
    if let Some(family) = &command.family {
        attrs = attrs.family(Family::Name(family.as_str()));
    }
    if command.bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    let shaping = if command.text.is_ascii() {
        Shaping::Basic
    } else {
        Shaping::Advanced
    };
    buffer.set_text(&command.text, &attrs, shaping, None);
    buffer.shape_until_scroll(font_system, false);

    let origin = match command.anchor {
        TextAnchor::TopLeft => command.position,
        TextAnchor::Center => {
            let (width, height) = extent(&buffer);
            Point::new(
                command.position.x - width * 0.5,
                command.position.y - height * 0.5,
            )
        }
    };
    let tint = premultiplied(&command.color);
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let physical = glyph.physical((origin.x, origin.y + run.line_y), 1.0);
            let Some(image) = cache.get_image(font_system, physical.cache_key).as_ref() else {
                continue;
            };
            let (ink_w, ink_h) = (image.placement.width, image.placement.height);
            if ink_w == 0 || ink_h == 0 {
                continue;
            }
            let origin_x = physical.x + image.placement.left;
            let origin_y = physical.y - image.placement.top;
            for dy in 0..ink_h {
                for dx in 0..ink_w {
                    let (x, y) = (
                        origin_x + i32::try_from(dx).unwrap_or(0),
                        origin_y + i32::try_from(dy).unwrap_or(0),
                    );
                    let coverage = framebuffer.coverage(clip, x, y);
                    if coverage == 0 {
                        continue;
                    }
                    let cell = dy as usize * ink_w as usize + dx as usize;
                    match image.content {
                        SwashContent::Mask => {
                            let mask = image.data.get(cell).copied().unwrap_or(0);
                            let scaled = (u32::from(mask) * u32::from(coverage)) / 255;
                            framebuffer.blend_covered(framebuffer.offset(x, y), scaled as u8, tint);
                        }
                        SwashContent::Color => {
                            let offset = cell * 4;
                            let alpha = image.data.get(offset + 3).copied().unwrap_or(0);
                            let scaled = (u32::from(alpha) * u32::from(coverage)) / 255;
                            let channel = |index: usize| -> u8 {
                                let byte = image.data.get(offset + index).copied().unwrap_or(0);
                                u8::try_from((u32::from(byte) * u32::from(tint[3])) / 255)
                                    .unwrap_or(255)
                            };
                            framebuffer.blend_covered(
                                framebuffer.offset(x, y),
                                scaled as u8,
                                [channel(0), channel(1), channel(2), alpha],
                            );
                        }
                        SwashContent::SubpixelMask => {}
                    }
                }
            }
        }
    }
}

/// The laid-out extent of a shaped block, for the centered anchor.
#[must_use]
pub(crate) fn extent(buffer: &Buffer) -> (f32, f32) {
    let (width, height) = (
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max),
        buffer
            .layout_runs()
            .map(|run| run.line_top + run.line_height)
            .fold(0.0, f32::max),
    );
    (width, height)
}

//! The dialog's brand mark: `assets/logo.svg` rasterized at build time
//! (resvg, the logo-wiring pattern) and embedded as raw premultiplied RGBA
//! bytes - no runtime SVG stack, no 410 KB PNG. The texture is loaded into
//! the egui context exactly once per surface (memoized in the context's
//! data map), so the window, the offscreen QA path, and the headless tests
//! all render the mark with zero plumbing through their frame loops.

use egui::{ColorImage, Context, Id, TextureHandle, TextureOptions, Ui, Vec2};

/// The logo raster: 128x128 premultiplied RGBA (tiny-skia's native form -
/// the build script's `raster_logo` output; pairs with
/// [`ColorImage::from_rgba_premultiplied`]).
const LOGO_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/logo_128.rgba"));
/// The raster's edge length in pixels.
const LOGO_PIXELS: usize = 128;
/// The context-data key memoizing the loaded texture.
const LOGO_TEXTURE_KEY: &str = "consent-logo-texture";

/// The logo texture for `ctx`, loading it on first use. The stored handle
/// keeps the texture alive for the context's lifetime (an egui
/// [`TextureHandle`] frees the texture when its last clone drops).
pub(super) fn texture(ctx: &Context) -> TextureHandle {
    let key = Id::new(LOGO_TEXTURE_KEY);
    if let Some(handle) = ctx.data(|data| data.get_temp::<TextureHandle>(key)) {
        return handle;
    }
    let image = ColorImage::from_rgba_premultiplied([LOGO_PIXELS; 2], LOGO_RGBA);
    let handle = ctx.load_texture("flowshot-logo", image, TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(key, handle.clone()));
    handle
}

/// The centered brand mark at `edge` x `edge` points (a modest mark, not a
/// banner; the caller's layout owns the placement).
pub(super) fn mark(ui: &mut Ui, edge: f32) {
    let handle = texture(ui.ctx());
    let image = egui::Image::new(&handle).fit_to_exact_size(Vec2::splat(edge));
    ui.add(image);
}

/// The mark's display edge as a multiple of the base typography size
/// (token-derived sizing: 3.5em = 49pt at the default base of 14).
#[must_use]
pub(super) fn mark_edge(base_size: f32) -> f32 {
    base_size * 3.5
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_raster_is_exactly_one_128px_rgba_image() {
        // Given/When: the build script's embedded logo bytes
        // Then: one pixel per raster texel, 4 bytes each (the
        //     from_rgba_premultiplied contract)
        assert_eq!(
            super::LOGO_RGBA.len(),
            super::LOGO_PIXELS * super::LOGO_PIXELS * 4
        );
    }
}

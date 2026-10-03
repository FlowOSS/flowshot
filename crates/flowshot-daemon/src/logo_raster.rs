//! Shared build-time logo rasterizer: `assets/logo.svg` -> the SNI
//! `IconPixmap` ARGB32 wire bytes at one size.
//!
//! Compiled in exactly two contexts - `build.rs` (which embeds the bytes
//! in the binary) and the lib's test build (the tray-icon determinism
//! test re-rasterizes the source and compares) - so the artifact and its
//! verification cannot drift apart. Never part of the shipped library:
//! resvg is a build/dev dependency only.

/// Pixmap sizes offered to the SNI host (it picks the closest and scales):
/// the tray convention 16/22/24/32/48.
pub const SIZES: [u32; 5] = [16, 22, 24, 32, 48];

/// Renders the logo SVG at `size` x `size` and returns the SNI wire bytes:
/// ARGB32 big-endian (A,R,G,B per pixel, row-major, premultiplied alpha -
/// tiny-skia's native form, and what SNI hosts composite).
///
/// Deterministic: same SVG bytes + same resvg version -> same output.
///
/// # Panics
///
/// When the SVG does not parse or the pixmap does not allocate - a broken
/// brand asset must fail the build, never ship an invisible tray icon.
#[must_use]
pub fn raster_argb(svg: &[u8], size: u32) -> Vec<u8> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .unwrap_or_else(|error| panic!("the logo SVG must parse with resvg: {error}"));
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)
        .unwrap_or_else(|| panic!("a {size}x{size} pixmap must allocate"));
    #[expect(
        clippy::cast_precision_loss,
        reason = "tray pixmap sizes are <= 48, exactly representable in f32"
    )]
    let scale = size as f32 / tree.size().width();
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let rgba = pixmap.data();
    let mut argb = Vec::with_capacity(rgba.len());
    for pixel in rgba.as_chunks::<4>().0 {
        argb.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
    }
    argb
}

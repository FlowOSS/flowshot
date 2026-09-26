//! The pinned pixel buffer: rotation transforms and opacity premultiply.
//!
//! The pin keeps the ORIGINAL upright RGBA buffer and derives the upload
//! buffer on demand (rotate, then premultiply by opacity). Copy/save
//! snapshots use the rotated buffer at FULL opacity - Flameshot parity:
//! `copyToClipboard` copies `m_pixmap` (rotated), while `setWindowOpacity`
//! is a window-level effect that never touches the pixmap.
//!
//! Opacity premultiplies in LINEAR light (decode -> multiply -> re-encode)
//! because the image pipeline samples `Rgba8UnormSrgb` and blends
//! `PREMULTIPLIED_ALPHA_BLENDING` (the todo-15 cursor-sprite lesson:
//! half-alpha white stores as 188, not 128).

use flowshot_core::geometry::Transform;

use crate::error::UiError;
use crate::render::{linear_to_srgb, srgb_to_linear};

/// Cumulative pin rotation in clockwise quarter turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    /// Upright (0 degrees).
    #[default]
    Up0,
    /// Rotated 90 degrees clockwise.
    Cw90,
    /// Rotated 180 degrees.
    Up180,
    /// Rotated 270 degrees clockwise (90 counter-clockwise).
    Cw270,
}

impl Rotation {
    /// The rotation one clockwise 90-degree step further.
    #[must_use]
    pub const fn clockwise(self) -> Self {
        match self {
            Self::Up0 => Self::Cw90,
            Self::Cw90 => Self::Up180,
            Self::Up180 => Self::Cw270,
            Self::Cw270 => Self::Up0,
        }
    }

    /// The rotation one counter-clockwise 90-degree step further.
    #[must_use]
    pub const fn counter_clockwise(self) -> Self {
        match self {
            Self::Up0 => Self::Cw270,
            Self::Cw90 => Self::Up0,
            Self::Up180 => Self::Cw90,
            Self::Cw270 => Self::Up180,
        }
    }

    /// Whether this rotation swaps width and height.
    #[must_use]
    pub const fn swaps_dimensions(self) -> bool {
        matches!(self, Self::Cw90 | Self::Cw270)
    }

    /// The core buffer transform that produces this rotation.
    ///
    /// Core `Transform::Rot90` is COUNTER-clockwise (`wl_output` convention),
    /// so a clockwise quarter turn maps to `Rot270`.
    #[must_use]
    pub const fn transform(self) -> Transform {
        match self {
            Self::Up0 => Transform::Normal,
            Self::Cw90 => Transform::Rot270,
            Self::Up180 => Transform::Rot180,
            Self::Cw270 => Transform::Rot90,
        }
    }
}

/// The pinned image: upright original RGBA8 pixels (row-major, top row
/// first, 4 bytes/px) plus its dimensions in physical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinImage {
    /// Width in physical pixels (upright orientation).
    pub width: u32,
    /// Height in physical pixels (upright orientation).
    pub height: u32,
    /// Pixel bytes; exactly `width * height * 4`.
    pub rgba: Vec<u8>,
}

impl PinImage {
    /// Wraps an upright RGBA buffer.
    ///
    /// # Errors
    ///
    /// [`UiError::PinImageInvalid`] when either dimension is zero, the
    /// declared size overflows `usize`, or `rgba` is shorter than
    /// `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, UiError> {
        let expected = usize::try_from(width)
            .ok()
            .zip(usize::try_from(height).ok())
            .and_then(|(w, h)| w.checked_mul(h))
            .and_then(|pixels| pixels.checked_mul(4))
            .unwrap_or(usize::MAX);
        if width == 0 || height == 0 || rgba.len() < expected {
            return Err(UiError::PinImageInvalid {
                width,
                height,
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// The dimensions after `rotation` (swapped for quarter turns).
    #[must_use]
    pub fn rotated_size(&self, rotation: Rotation) -> (u32, u32) {
        if rotation.swaps_dimensions() {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    /// The upload/snapshot buffer: `rotation` applied, then premultiplied by
    /// `opacity` (0.0..=1.0). At full opacity and upright rotation this
    /// clones the original bytes untouched (no re-encode round-trip loss).
    ///
    /// # Errors
    ///
    /// [`UiError::Geometry`] propagates [`Transform::remap_buffer`]
    /// validation failures.
    pub fn composed(&self, rotation: Rotation, opacity: f32) -> Result<Vec<u8>, UiError> {
        let rotated = self.rotated(rotation)?;
        Ok(premultiply_linear(&rotated, opacity.clamp(0.0, 1.0)))
    }

    /// The rotated buffer at full opacity (the copy/save snapshot payload).
    ///
    /// # Errors
    ///
    /// [`UiError::Geometry`] propagates [`Transform::remap_buffer`]
    /// validation failures.
    pub fn rotated(&self, rotation: Rotation) -> Result<Vec<u8>, UiError> {
        if rotation == Rotation::Up0 {
            return Ok(self.rgba.clone());
        }
        let mut dst = vec![0u8; self.rgba.len()];
        rotation.transform().remap_buffer(
            &self.rgba,
            &mut dst,
            usize::try_from(self.width).unwrap_or(usize::MAX),
            usize::try_from(self.height).unwrap_or(usize::MAX),
            4,
        )?;
        Ok(dst)
    }
}

/// Premultiplies straight-alpha RGBA pixels by `opacity` in linear light:
/// `store = encode(linear(c) * opacity)`, `alpha = opacity * 255`. A fully
/// opaque buffer at full opacity returns the input unchanged (identity
/// fast path - no re-encode round-trip, no per-pixel pow); any translucent
/// texel (an alpha-bearing PNG pinned via todo 35) goes through the
/// normalization so the premultiplied image pipeline never misreads
/// straight-alpha content as premultiplied.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "every cast value is a rounded product of [0, 1] factors, provably inside 0..=255"
)]
fn premultiply_linear(rgba: &[u8], opacity: f32) -> Vec<u8> {
    if opacity >= 1.0 && rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 255) {
        return rgba.to_vec();
    }
    let opacity = opacity.clamp(0.0, 1.0);
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|px| {
            // Effective alpha = the texel's own alpha x the window opacity
            // (opaque captures: texel alpha 1 -> the opacity alone).
            let alpha = (f32::from(px[3]) / 255.0) * opacity;
            let a = (f64::from(alpha) * 255.0).round() as u8;
            let premul = |c: u8| -> u8 {
                let encoded = f32::from(c) / 255.0;
                let scaled = srgb_to_linear(encoded) * alpha;
                (f64::from(linear_to_srgb(scaled)) * 255.0).round() as u8
            };
            [premul(px[0]), premul(px[1]), premul(px[2]), a]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_possible_truncation)]

    use super::*;

    /// 2x1 fixture: left pixel opaque red, right pixel opaque white.
    fn fixture() -> PinImage {
        PinImage::new(2, 1, vec![255, 0, 0, 255, 255, 255, 255, 255])
            .unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    #[test]
    fn rotation_cycles_clockwise_and_back() {
        let mut r = Rotation::Up0;
        for expected in [
            Rotation::Cw90,
            Rotation::Up180,
            Rotation::Cw270,
            Rotation::Up0,
        ] {
            r = r.clockwise();
            assert_eq!(r, expected);
        }
        assert_eq!(Rotation::Up0.counter_clockwise(), Rotation::Cw270);
        assert_eq!(Rotation::Cw90.counter_clockwise(), Rotation::Up0);
        assert!(Rotation::Cw90.swaps_dimensions());
        assert!(!Rotation::Up180.swaps_dimensions());
    }

    #[test]
    fn clockwise_rotation_moves_top_left_to_top_right() {
        // 2x1 (red, white) rotated CW90 -> 1x2 with red ON TOP (top-left
        // corner travels to the top-right of a landscape source = top row
        // of the portrait destination).
        let rotated = fixture()
            .rotated(Rotation::Cw90)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rotated, vec![255, 0, 0, 255, 255, 255, 255, 255]);
        assert_eq!(fixture().rotated_size(Rotation::Cw90), (1, 2));
    }

    #[test]
    fn half_opacity_premultiplies_in_linear_light() {
        // The todo-15 verified constant: half-alpha white stores as 188.
        let composed = fixture()
            .composed(Rotation::Up0, 0.5)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(&composed[4..8], &[188, 188, 188, 128]);
        // Full opacity is the identity fast path (exact original bytes).
        let full = fixture()
            .composed(Rotation::Up0, 1.0)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(full, fixture().rgba);
    }

    #[test]
    fn translucent_input_normalizes_to_premultiplied_at_full_opacity() {
        // Straight-alpha white at 50% (a pinned alpha PNG) must upload as
        // the premultiplied equivalent (the todo-15 constant: 188).
        let image = PinImage::new(1, 1, vec![255, 255, 255, 128]).unwrap_or_else(|e| panic!("{e}"));
        let composed = image
            .composed(Rotation::Up0, 1.0)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(composed[0], 188);
        assert_eq!(composed[3], 128);
    }

    #[test]
    fn zero_opacity_stores_transparent_black() {
        let composed = fixture()
            .composed(Rotation::Up0, 0.0)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(composed, vec![0; 8]);
    }

    #[test]
    fn new_rejects_bad_buffers() {
        assert!(matches!(
            PinImage::new(2, 2, vec![0; 15]),
            Err(UiError::PinImageInvalid { .. })
        ));
        assert!(matches!(
            PinImage::new(0, 2, vec![]),
            Err(UiError::PinImageInvalid { .. })
        ));
    }
}

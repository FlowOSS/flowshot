//! Color values for the 2D renderer.
//!
//! Colors are parsed from design tokens (`#RRGGBB` / `#RRGGBBAA` hex strings)
//! and stored **sRGB-encoded** (the token byte divided by 255). Vertex
//! construction converts to linear light via [`Color::premultiplied_linear`]:
//! every render target is an sRGB texture format, so the hardware re-encodes
//! on store and opaque flat fills round-trip to the exact token bytes.
//! Blending therefore happens in linear space (tiny-skia, the dev-only parity
//! reference, blends in encoded space - the difference is confined to
//! antialiased edges, which the parity policy masks).

use flowshot_core::tokens::{Palette, Shadow};

/// An sRGB-encoded RGBA color with normalized `f32` channels in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red channel, sRGB-encoded.
    pub r: f32,
    /// Green channel, sRGB-encoded.
    pub g: f32,
    /// Blue channel, sRGB-encoded.
    pub b: f32,
    /// Alpha channel (linear by convention; alpha is never gamma-encoded).
    pub a: f32,
}

impl Color {
    /// Parses a design-token hex color: `#RRGGBB` (opaque) or `#RRGGBBAA`
    /// (shadow tokens carry alpha in the last byte).
    #[must_use]
    pub fn from_hex_token(hex: &str) -> Option<Self> {
        let digits = hex.strip_prefix('#')?.as_bytes();
        if !matches!(digits.len(), 6 | 8) {
            return None;
        }
        let mut channels = [0u8; 4];
        channels[3] = 255;
        for (index, channel) in channels.iter_mut().enumerate() {
            // A 6-digit token has no alpha pair; the loop ends with the
            // opaque default already in place.
            let Some(pair) = digits.get(index * 2..index * 2 + 2) else {
                break;
            };
            let hi = from_hex_digit(*pair.first()?)?;
            let lo = from_hex_digit(*pair.get(1)?)?;
            *channel = hi * 16 + lo;
        }
        let [r, g, b, a] = channels;
        Some(Self::from_rgba8(r, g, b, a))
    }

    /// Builds a color from 8-bit sRGB channels.
    #[must_use]
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: byte_to_unit(r),
            g: byte_to_unit(g),
            b: byte_to_unit(b),
            a: byte_to_unit(a),
        }
    }

    /// The dim-layer color from palette tokens: `contrast` at `dim_opacity`
    /// alpha (plan todo 14(d) - the contrastOpacity token, zero hardcoded
    /// visual constants).
    ///
    /// # Errors
    ///
    /// `None` when the token's `contrast` hex string is malformed.
    #[must_use]
    pub fn dim_from_palette(palette: &Palette) -> Option<Self> {
        Self::from_hex_token(&palette.contrast).map(|color| color.with_alpha8(palette.dim_opacity))
    }

    /// The shadow color from a shadow token (`#RRGGBBAA`).
    ///
    /// # Errors
    ///
    /// `None` when the token's hex string is malformed.
    #[must_use]
    pub fn from_shadow_token(shadow: &Shadow) -> Option<Self> {
        Self::from_hex_token(&shadow.color)
    }

    /// The same color with alpha replaced by an 8-bit opacity token value.
    #[must_use]
    pub fn with_alpha8(self, alpha: u8) -> Self {
        Self {
            a: byte_to_unit(alpha),
            ..self
        }
    }

    /// The same color with alpha replaced, clamped to `[0, 1]`.
    #[must_use]
    pub fn with_alpha(self, alpha: f32) -> Self {
        Self {
            a: alpha.clamp(0.0, 1.0),
            ..self
        }
    }

    /// Weighted-luma darkness test on the sRGB-encoded channels (the
    /// Flameshot `ColorUtils::colorIsDark` parity convention).
    #[must_use]
    pub fn is_dark(&self) -> bool {
        0.299 * self.r + 0.587 * self.g + 0.114 * self.b < 0.5
    }

    /// Black-or-white text ink readable on THIS color as a background (the
    /// single source of the HUD/menu/panel text convention; todo-41 polish:
    /// light contrast tokens get dark ink instead of hardcoded white).
    #[must_use]
    pub fn readable_ink(&self) -> Self {
        if self.is_dark() {
            Self::from_rgba8(255, 255, 255, 255)
        } else {
            Self::from_rgba8(0, 0, 0, 255)
        }
    }

    /// Premultiplied **linear-light** RGBA, ready for vertex data rendered
    /// into sRGB attachments with premultiplied-alpha blending.
    #[must_use]
    pub fn premultiplied_linear(self) -> [f32; 4] {
        let a = self.a;
        [
            srgb_to_linear(self.r) * a,
            srgb_to_linear(self.g) * a,
            srgb_to_linear(self.b) * a,
            a,
        ]
    }
}

/// The sRGB EOTF: one encoded channel in `[0, 1]` to linear light.
#[must_use]
pub fn srgb_to_linear(encoded: f32) -> f32 {
    let c = encoded.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse sRGB EOTF: linear light in `[0, 1]` to an encoded channel.
#[must_use]
pub fn linear_to_srgb(linear: f32) -> f32 {
    let c = linear.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn byte_to_unit(byte: u8) -> f32 {
    f32::from(byte) / 255.0
}

fn from_hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn parses_token_accent_rgb() {
        let color = Color::from_hex_token("#6366F1").unwrap_or_else(|| panic!("parse"));
        assert_eq!(color.r, f32::from(0x63u8) / 255.0);
        assert_eq!(color.g, f32::from(0x66u8) / 255.0);
        assert_eq!(color.b, f32::from(0xF1u8) / 255.0);
        assert_eq!(color.a, 1.0);
        assert!(Color::from_hex_token("#6366f1").is_some());
    }

    #[test]
    fn parses_shadow_token_with_alpha() {
        let color = Color::from_hex_token("#0000001F").unwrap_or_else(|| panic!("parse"));
        assert_eq!(color.r, 0.0);
        assert_eq!(color.a, f32::from(0x1Fu8) / 255.0);
    }

    #[test]
    fn rejects_malformed_hex() {
        for bad in [
            "6366F1",
            "#GGG",
            "#2AA19",
            "#6366F18",
            "",
            "#12345g",
            "#123456789",
        ] {
            assert!(Color::from_hex_token(bad).is_none(), "{bad} parsed");
        }
    }

    #[test]
    fn dim_color_comes_from_palette_tokens() {
        let palette = Palette::default();
        let dim = Color::dim_from_palette(&palette).unwrap_or_else(|| panic!("parse"));
        let contrast = Color::from_hex_token(&palette.contrast).unwrap_or_else(|| panic!("parse"));
        assert_eq!((dim.r, dim.g, dim.b), (contrast.r, contrast.g, contrast.b));
        assert_eq!(dim.a, f32::from(palette.dim_opacity) / 255.0);
    }

    #[test]
    fn srgb_roundtrips_every_byte_within_one_unit() {
        for byte in 0..=255u8 {
            let encoded = f32::from(byte) / 255.0;
            let restored = linear_to_srgb(srgb_to_linear(encoded)) * 255.0;
            assert!(
                (restored - f32::from(byte)).abs() <= 1.0,
                "byte {byte} roundtripped to {restored}"
            );
        }
    }

    #[test]
    fn srgb_endpoints_are_exact() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert_eq!(srgb_to_linear(1.0), 1.0);
        assert_eq!(linear_to_srgb(0.0), 0.0);
        // powf in f32 lands one ulp under 1.0; the byte round-trip test above
        // is the exactness contract that matters.
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn premultiplied_linear_scales_by_alpha() {
        let color = Color::from_rgba8(255, 255, 255, 128);
        let [r, g, b, a] = color.premultiplied_linear();
        assert!((r - g).abs() < 1e-6 && (g - b).abs() < 1e-6);
        assert!((a - 128.0 / 255.0).abs() < 1e-6);
        assert!((r - a).abs() < 1e-6, "white premultiplies to alpha");
    }
}

//! Procedural tray icon: the `FlowShot` selection-frame glyph rendered
//! straight into the SNI `IconPixmap` ARGB32 wire format.
//!
//! Asset decision (recorded): the vendored icon set is editor-tool SVGs
//! rasterized into a `flowshot-ui` build-time atlas - it carries no app
//! logo, and consuming the atlas would drag the GPU stack plus a PNG
//! decoder into the daemon. The glyph below is drawn from the brand
//! accent token instead: zero deps, deterministic, unit-testable. When
//! packaging lands an installed themed icon can take over
//! via the `IconName` property (hosts prefer it over the pixmap).

use super::spec::IconWire;

/// Pixmap sizes offered to the host (it picks the closest and scales).
pub const SIZES: [u32; 5] = [16, 22, 24, 32, 48];

/// Attention-state glyph color (red 600): the core palette has no danger
/// token yet (the settings pass may add one - recorded).
const ATTENTION_RGB: (u8, u8, u8) = (220, 38, 38);

/// Fallback when the configured accent does not parse.
const DEFAULT_ACCENT_RGB: (u8, u8, u8) = (0x63, 0x66, 0xF1);

/// The idle and attention pixmap sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconSet {
    /// `IconPixmap` entries (idle).
    pub idle: Vec<IconWire>,
    /// `AttentionIconPixmap` entries.
    pub attention: Vec<IconWire>,
}

/// Builds both pixmap sets from a `#RRGGBB` accent (the `[ui].accent_color`
/// config value; unparseable input falls back to the brand token).
#[must_use]
pub fn icon_set(accent_hex: &str) -> IconSet {
    let accent = parse_hex_color(accent_hex).unwrap_or(DEFAULT_ACCENT_RGB);
    IconSet {
        idle: pixmap_set(accent),
        attention: pixmap_set(ATTENTION_RGB),
    }
}

/// Parses `#RRGGBB` (case-insensitive); anything else is `None`.
#[must_use]
pub fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |offset: usize| u8::from_str_radix(digits.get(offset..offset + 2)?, 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

fn pixmap_set(rgb: (u8, u8, u8)) -> Vec<IconWire> {
    SIZES
        .iter()
        .map(|&size| IconWire {
            width: i32::try_from(size).unwrap_or(i32::MAX),
            height: i32::try_from(size).unwrap_or(i32::MAX),
            data: glyph(size, rgb),
        })
        .collect()
}

/// Renders the selection-frame glyph: four L-shaped corner brackets (the
/// universal capture-region symbol) in `rgb` on a transparent ground, as
/// ARGB32 big-endian bytes (A first - the SNI spec's pixel order).
///
/// Brackets are fold-symmetric: a pixel is painted when its distance-fold
/// `(min(x, size-1-x), min(y, size-1-y))` lands in the top-left bracket's
/// bars. `size` is bounded by [`SIZES`] in practice (the ratios below keep
/// the four brackets disjoint for any `size >= 8`).
#[must_use]
pub fn glyph(size: u32, rgb: (u8, u8, u8)) -> Vec<u8> {
    let margin = (size / 8).max(1);
    let arm = margin + (size / 4).max(2);
    let thickness = margin + (size / 16).max(2);
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let fold_y = y.min(size.saturating_sub(1) - y);
        for x in 0..size {
            let fold_x = x.min(size.saturating_sub(1) - x);
            let in_thick_x = (margin..thickness).contains(&fold_x);
            let in_thick_y = (margin..thickness).contains(&fold_y);
            let in_arm_x = (margin..arm).contains(&fold_x);
            let in_arm_y = (margin..arm).contains(&fold_y);
            let painted = (in_thick_y && in_arm_x) || (in_thick_x && in_arm_y);
            let pixel = if painted {
                [255, rgb.0, rgb.1, rgb.2]
            } else {
                [0, 0, 0, 0]
            };
            data.extend_from_slice(&pixel);
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Type;

    #[test]
    fn hex_parsing_accepts_token_form_and_rejects_junk() {
        assert_eq!(parse_hex_color("#6366F1"), Some((0x63, 0x66, 0xF1)));
        assert_eq!(parse_hex_color("#0f172a"), Some((0x0F, 0x17, 0x2A)));
        assert_eq!(parse_hex_color("6366F1"), None);
        assert_eq!(parse_hex_color("#6366F"), None);
        assert_eq!(parse_hex_color("#GGGGGG"), None);
        assert_eq!(parse_hex_color(""), None);
    }

    #[test]
    fn glyph_has_exact_argb_wire_geometry() {
        for size in SIZES {
            let data = glyph(size, (1, 2, 3));
            assert_eq!(
                data.len(),
                (size * size * 4) as usize,
                "size {size} must fill the ARGB32 plane"
            );
        }
    }

    #[test]
    fn glyph_paints_brackets_opaque_and_ground_transparent() {
        let size = 32;
        let data = glyph(size, (0x63, 0x66, 0xF1));
        let pixel = |x: u32, y: u32| {
            let offset = ((y * size + x) * 4) as usize;
            (
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            )
        };
        let margin = size / 8;
        assert_eq!(pixel(0, 0), (0, 0, 0, 0), "ground stays transparent");
        assert_eq!(pixel(size / 2, size / 2), (0, 0, 0, 0));
        assert_eq!(
            pixel(margin, margin),
            (255, 0x63, 0x66, 0xF1),
            "the top-left bracket corner is opaque accent, A-first byte order"
        );
        assert_eq!(pixel(size - 1 - margin, margin), (255, 0x63, 0x66, 0xF1));
        assert_eq!(pixel(margin, size - 1 - margin), (255, 0x63, 0x66, 0xF1));
        assert_eq!(
            pixel(size - 1 - margin, size - 1 - margin),
            (255, 0x63, 0x66, 0xF1)
        );
    }

    #[test]
    fn icon_set_offers_every_size_and_a_distinct_attention_state() {
        let set = icon_set("#6366F1");
        assert_eq!(set.idle.len(), SIZES.len());
        assert_eq!(set.attention.len(), SIZES.len());
        for (entry, &size) in set.idle.iter().zip(SIZES.iter()) {
            assert_eq!(entry.width, i32::try_from(size).unwrap_or(-1));
            assert_eq!(entry.height, entry.width);
            assert_eq!(IconWire::signature().as_str(), "(iiay)");
        }
        assert_ne!(set.idle, set.attention);
    }

    #[test]
    fn unparseable_accent_falls_back_to_the_brand_token() {
        let broken = icon_set("not-a-color");
        let branded = icon_set("#6366F1");
        assert_eq!(broken.idle, branded.idle);
    }
}

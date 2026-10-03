//! Real-logo tray icon: the `FlowShot` mark (`assets/logo.svg`) rendered
//! straight into the SNI `IconPixmap` ARGB32 wire format.
//!
//! Asset pipeline (recorded): `build.rs` rasterizes the SVG with the
//! workspace resvg pin at exactly the [`SIZES`] the tray serves and embeds
//! the raw wire bytes (~18 KB total) - the binary carries neither the
//! 2048px PNG export nor a runtime SVG stack. The previous procedural
//! bracket glyph predates the brand mark and is gone.
//!
//! The attention state keeps the recorded convention (the procedural glyph
//! recolored to red 600): the multicolor logo takes it as a red-600 ring
//! over the badge's outer band, so the mark stays recognizable and the two
//! states are never confused at 16px. When packaging lands an installed
//! themed icon can take over via the `IconName` property (hosts prefer it
//! over the pixmap).

use super::spec::IconWire;

include!(concat!(env!("OUT_DIR"), "/logo_icons.rs"));

/// Attention-state ring color (red 600): the core palette has no danger
/// token yet (the settings pass may add one - recorded).
const ATTENTION_RGB: (u8, u8, u8) = (220, 38, 38);

/// The idle and attention pixmap sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconSet {
    /// `IconPixmap` entries (idle).
    pub idle: Vec<IconWire>,
    /// `AttentionIconPixmap` entries.
    pub attention: Vec<IconWire>,
}

/// Builds both pixmap sets from the embedded logo rasters: idle = the mark
/// as authored, attention = the mark with the red-600 ring.
#[must_use]
pub fn icon_set() -> IconSet {
    IconSet {
        idle: pixmap_set(|_, logo| logo.to_vec()),
        attention: pixmap_set(attention_ring),
    }
}

fn pixmap_set(derive: impl Fn(u32, &[u8]) -> Vec<u8>) -> Vec<IconWire> {
    SIZES
        .into_iter()
        .zip(LOGO_ARGB)
        .map(|(size, logo)| {
            let side = i32::try_from(size).unwrap_or(i32::MAX);
            IconWire {
                width: side,
                height: side,
                data: derive(size, logo),
            }
        })
        .collect()
}

/// Derives the attention pixmap: the red-600 ring over the badge's outer
/// band (thickness `size / 16`, floor 2px), painted only where the logo is
/// already solid (alpha >= 128) so the silhouette is unchanged. Pure
/// doubled-integer distance math against the badge circle (the mark fills
/// its viewBox, so the doubled radius is `size`) - deterministic and
/// unit-testable.
fn attention_ring(size: u32, logo: &[u8]) -> Vec<u8> {
    let mut out = logo.to_vec();
    let thickness = i64::from((size / 16).max(2));
    let center2 = i64::from(size) - 1;
    let inner2 = i64::from(size) - 2 * thickness;
    let inner_sq = inner2 * inner2;
    for y in 0..size {
        let dy = 2 * i64::from(y) - center2;
        for x in 0..size {
            let dx = 2 * i64::from(x) - center2;
            let offset = ((y * size + x) * 4) as usize;
            let solid = out[offset] >= 128;
            let in_band = dx * dx + dy * dy >= inner_sq;
            if solid && in_band {
                out[offset..offset + 4].copy_from_slice(&[
                    255,
                    ATTENTION_RGB.0,
                    ATTENTION_RGB.1,
                    ATTENTION_RGB.2,
                ]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logo_raster;
    use std::collections::HashSet;
    use zbus::zvariant::Type;

    fn svg_source() -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/logo.svg");
        std::fs::read(path).unwrap_or_else(|error| panic!("{path} must be readable: {error}"))
    }

    #[test]
    fn icon_set_serves_every_size_with_nonempty_nonflat_pixmaps() {
        let set = icon_set();
        assert_eq!(set.idle.len(), SIZES.len());
        assert_eq!(set.attention.len(), SIZES.len());
        for (entry, &size) in set.idle.iter().zip(SIZES.iter()) {
            assert_eq!(entry.width, i32::try_from(size).unwrap_or(-1));
            assert_eq!(entry.height, entry.width);
            assert_eq!(IconWire::SIGNATURE.to_string(), "(iiay)");
            assert_eq!(
                entry.data.len(),
                (size * size * 4) as usize,
                "size {size} must fill the ARGB32 plane"
            );
            let plane = (size * size) as usize;
            let painted = entry
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[0] > 0)
                .count();
            assert!(
                painted * 2 > plane,
                "size {size}: the badge must paint most of the plane ({painted}/{plane})"
            );
            let distinct = entry
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .collect::<HashSet<_>>()
                .len();
            assert!(
                distinct > 8,
                "size {size}: the gradient mark must not be flat ({distinct} distinct pixels)"
            );
            assert_eq!(
                &entry.data[..4],
                &[0, 0, 0, 0],
                "the ground outside the circular badge stays transparent"
            );
        }
    }

    #[test]
    fn embedded_pixmaps_reproduce_from_the_svg_source() {
        // Determinism AND freshness: the shared build-time raster function
        // over the current source SVG reproduces every embedded byte (a
        // stale artifact or a nondeterministic rasterizer fails here).
        let svg = svg_source();
        assert_eq!(
            SIZES,
            logo_raster::SIZES,
            "the generated size list must come from the shared raster module"
        );
        for (&size, embedded) in SIZES.iter().zip(LOGO_ARGB) {
            assert_eq!(
                logo_raster::raster_argb(&svg, size),
                embedded,
                "size {size}: the embedded pixmap must equal a fresh raster of assets/logo.svg"
            );
        }
    }

    #[test]
    fn attention_rings_the_badge_without_touching_its_shape() {
        let set = icon_set();
        assert_ne!(set.idle, set.attention);
        for (idle, attention) in set.idle.iter().zip(&set.attention) {
            let size = u32::try_from(idle.width).unwrap_or(0);
            assert_eq!(idle.data.len(), attention.data.len());
            assert_eq!(&attention.data[..4], &[0, 0, 0, 0], "corner ground");
            let last = attention.data.len() - 4;
            assert_eq!(&attention.data[last..], &[0, 0, 0, 0]);
            let center = (size / 2) as usize;
            let center_offset = (center * size as usize + center) * 4;
            assert_eq!(
                &attention.data[center_offset..center_offset + 4],
                &idle.data[center_offset..center_offset + 4],
                "the center dot keeps the idle logo"
            );
            let center2 = i64::from(size) - 1;
            let peripheral = (i64::from(size) / 4).pow(2);
            let mut ring_pixels = 0usize;
            for y in 0..size {
                for x in 0..size {
                    let offset = ((y * size + x) * 4) as usize;
                    let (before, after) = (
                        &idle.data[offset..offset + 4],
                        &attention.data[offset..offset + 4],
                    );
                    if before == after {
                        continue;
                    }
                    ring_pixels += 1;
                    assert_eq!(
                        after,
                        &[255, ATTENTION_RGB.0, ATTENTION_RGB.1, ATTENTION_RGB.2],
                        "changed pixels are opaque red-600, A-first"
                    );
                    assert!(
                        before[0] >= 128,
                        "the ring only paints over solid logo pixels"
                    );
                    let dx = 2 * i64::from(x) - center2;
                    let dy = 2 * i64::from(y) - center2;
                    assert!(
                        dx * dx + dy * dy >= peripheral,
                        "the ring stays in the badge's outer band"
                    );
                }
            }
            assert!(
                ring_pixels > 0,
                "size {size}: the attention state must be visible"
            );
        }
    }
}

//! Shelf packer for the glyph texture atlas (pure, GPU-free).
//!
//! The text stack rasterizes glyphs once into a fixed-size atlas texture and
//! draws instances by UV (the atlas-first policy - no SDF). The
//! allocator is a shelf packer: glyphs fill a row until the next one does not
//! fit, then a new shelf starts below. Callers add their own padding to the
//! requested size so linear filtering never bleeds across glyphs. When the
//! atlas fills, the caller resets it and re-rasterizes on demand (glyph
//! working sets are tiny for an overlay UI).

/// One allocated atlas region, in texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AtlasSlot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A fixed-size shelf packer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShelfAtlas {
    width: u32,
    height: u32,
    cursor_x: u32,
    cursor_y: u32,
    shelf_height: u32,
}

impl ShelfAtlas {
    pub(crate) const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            cursor_x: 0,
            cursor_y: 0,
            shelf_height: 0,
        }
    }

    /// Allocates a `width` x `height` region; `None` when it no longer fits
    /// (the caller resets and retries). Zero-sized requests always fail.
    pub(crate) fn allocate(&mut self, width: u32, height: u32) -> Option<AtlasSlot> {
        if width == 0 || height == 0 || width > self.width || height > self.height {
            return None;
        }
        if self.cursor_x + width > self.width {
            self.cursor_x = 0;
            self.cursor_y += self.shelf_height;
            self.shelf_height = 0;
        }
        if self.cursor_y + height > self.height {
            return None;
        }
        let slot = AtlasSlot {
            x: self.cursor_x,
            y: self.cursor_y,
            width,
            height,
        };
        self.cursor_x += width;
        self.shelf_height = self.shelf_height.max(height);
        Some(slot)
    }

    /// Empties the atlas (every outstanding slot becomes invalid).
    pub(crate) fn reset(&mut self) {
        *self = Self::new(self.width, self.height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_left_to_right_then_wraps_shelves() {
        let mut atlas = ShelfAtlas::new(64, 64);
        let a = atlas.allocate(30, 10).unwrap_or_else(|| panic!("slot"));
        let b = atlas.allocate(30, 12).unwrap_or_else(|| panic!("slot"));
        assert_eq!(
            a,
            AtlasSlot {
                x: 0,
                y: 0,
                width: 30,
                height: 10
            }
        );
        assert_eq!(
            b,
            AtlasSlot {
                x: 30,
                y: 0,
                width: 30,
                height: 12
            }
        );
        // 30 + 30 > 64: wraps to a new shelf whose y is the tallest so far.
        let c = atlas.allocate(30, 8).unwrap_or_else(|| panic!("slot"));
        assert_eq!((c.x, c.y), (0, 12));
    }

    #[test]
    fn reports_full_instead_of_overflowing() {
        let mut atlas = ShelfAtlas::new(16, 16);
        assert!(atlas.allocate(16, 16).is_some());
        assert!(atlas.allocate(1, 1).is_none());
        atlas.reset();
        assert!(atlas.allocate(16, 16).is_some());
    }

    #[test]
    fn rejects_degenerate_and_oversized_requests() {
        let mut atlas = ShelfAtlas::new(16, 16);
        assert!(atlas.allocate(0, 4).is_none());
        assert!(atlas.allocate(4, 0).is_none());
        assert!(atlas.allocate(17, 4).is_none());
        // Failed allocations leave the packer state untouched: the full-size
        // request still fits afterwards.
        assert!(atlas.allocate(16, 16).is_some());
    }
}

//! Renderer geometry: physical-pixel `f32` primitives.
//!
//! The display list speaks **physical pixels** (f32, origin top-left, y down)
//! so subpixel antialiasing survives to the rasterizer; consumers convert
//! logical layout coordinates via `flowshot_core::geometry` before building
//! commands. These types deliberately do not reuse the core newtype spaces
//! (`PhysicalPx(i32)` / `Logical(f64)`): the renderer needs subpixel `f32`
//! precision, and stroke widths are physical by contract.

/// A point in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Horizontal coordinate, physical px.
    pub x: f32,
    /// Vertical coordinate, physical px (y down).
    pub y: f32,
}

impl Point {
    /// Builds a point.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A size in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// Width, physical px.
    pub width: f32,
    /// Height, physical px.
    pub height: f32,
}

impl Size {
    /// Builds a size.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Whether both dimensions are finite and non-negative.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width >= 0.0 && self.height >= 0.0
    }
}

/// An axis-aligned rectangle in physical pixels: `origin` plus `size`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Top-left corner.
    pub origin: Point,
    /// Extent.
    pub size: Size,
}

impl Rect {
    /// Builds a rect from a corner and a size.
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// Builds a rect from raw components.
    #[must_use]
    pub const fn from_parts(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size::new(width, height),
        }
    }

    /// Returns true if the point is inside the rectangle.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.origin.x
            && point.x < self.origin.x + self.size.width
            && point.y >= self.origin.y
            && point.y < self.origin.y + self.size.height
    }

    /// Right edge (`x + width`).
    #[must_use]
    pub fn right(self) -> f32 {
        self.origin.x + self.size.width
    }

    /// Bottom edge (`y + height`).
    #[must_use]
    pub fn bottom(self) -> f32 {
        self.origin.y + self.size.height
    }

    /// The center point.
    #[must_use]
    pub fn center(self) -> Point {
        Point::new(
            self.origin.x + self.size.width * 0.5,
            self.origin.y + self.size.height * 0.5,
        )
    }

    /// Whether two rects overlap (edge-touching rects do NOT intersect -
    /// the chrome layout's abutment rule: a widget flush against another
    /// is not covering it).
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.origin.x < other.right()
            && other.origin.x < self.right()
            && self.origin.y < other.bottom()
            && other.origin.y < self.bottom()
    }

    /// The rect grown by `amount` on every side (negative shrinks).
    #[must_use]
    pub fn expand(self, amount: f32) -> Self {
        Self::from_parts(
            self.origin.x - amount,
            self.origin.y - amount,
            self.size.width + amount * 2.0,
            self.size.height + amount * 2.0,
        )
    }

    /// The rect translated by an offset.
    #[must_use]
    pub fn translate(self, offset: Point) -> Self {
        Self::new(
            Point::new(self.origin.x + offset.x, self.origin.y + offset.y),
            self.size,
        )
    }

    /// Whether every component is finite and the size is non-negative.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.origin.x.is_finite() && self.origin.y.is_finite() && self.size.is_valid()
    }
}

/// `u32` texture/atlas dimensions and pixel counts convert losslessly for
/// every value the renderer handles (device texture limits cap far below
/// f32's exact-integer range of 2^24).
#[allow(clippy::cast_precision_loss)]
pub(crate) fn f32_from_u32(value: u32) -> f32 {
    value as f32
}

/// `i32` physical-pixel coordinates convert losslessly for every value the
/// renderer handles (surface extents cap far below f32's exact-integer range
/// of 2^24).
#[allow(clippy::cast_precision_loss)]
pub(crate) fn f32_from_i32(value: i32) -> f32 {
    value as f32
}

/// `f64` logical coordinates/lengths convert to the renderer's `f32` pixel
/// space; layout-scale values lose only sub-ulp precision, and an overflow
/// saturates to ±inf (the renderer skips non-finite geometry with a log).
#[expect(
    clippy::cast_possible_truncation,
    reason = "f32 IS the renderer's pixel contract; f64 layout values are far inside its range"
)]
pub(crate) fn f32_from_f64(value: f64) -> f32 {
    value as f32
}

/// The viewport transform applied to every staged vertex family once per
/// frame: positions arrive in physical px (y down) and map to clip-space NDC
/// (x,y in [-1,1], y up). Generic over the fixed-array vertex types, which all
/// carry position at indices 0,1 (`FlatVertex`/`ImageVertex`/`ShadowVertex`/
/// `TextVertex`); the shaders pass positions through unchanged.
pub(crate) fn ndc_transform<const N: usize>(vertices: &mut [[f32; N]], width: f32, height: f32) {
    for vertex in vertices {
        vertex[0] = 2.0 * vertex[0] / width - 1.0;
        vertex[1] = 1.0 - 2.0 * vertex[1] / height;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn rect_edges_and_center() {
        let rect = Rect::from_parts(10.0, 20.0, 30.0, 40.0);
        assert_eq!(rect.right(), 40.0);
        assert_eq!(rect.bottom(), 60.0);
        assert_eq!(rect.center(), Point::new(25.0, 40.0));
    }

    #[test]
    fn rect_expand_grows_on_every_side() {
        let rect = Rect::from_parts(10.0, 10.0, 10.0, 10.0).expand(2.0);
        assert_eq!(rect, Rect::from_parts(8.0, 8.0, 14.0, 14.0));
    }

    #[test]
    fn rect_intersects_overlaps_only() {
        let a = Rect::from_parts(10.0, 10.0, 20.0, 20.0);
        assert!(a.intersects(&Rect::from_parts(20.0, 20.0, 20.0, 20.0)));
        assert!(a.intersects(&Rect::from_parts(0.0, 0.0, 100.0, 100.0)));
        // Edge-touching and disjoint rects do not intersect.
        assert!(!a.intersects(&Rect::from_parts(30.0, 10.0, 20.0, 20.0)));
        assert!(!a.intersects(&Rect::from_parts(10.0, 30.0, 20.0, 20.0)));
        assert!(!a.intersects(&Rect::from_parts(100.0, 100.0, 5.0, 5.0)));
    }

    #[test]
    fn validity_rejects_nan_and_negative_size() {
        assert!(Rect::from_parts(0.0, 0.0, 1.0, 1.0).is_valid());
        assert!(!Rect::from_parts(f32::NAN, 0.0, 1.0, 1.0).is_valid());
        assert!(!Rect::from_parts(0.0, 0.0, -1.0, 1.0).is_valid());
        assert!(!Size::new(1.0, f32::INFINITY).is_valid());
    }
}

//! Renderer geometry: physical-pixel `f32` primitives.
//!
//! The display list speaks **physical pixels** (f32, origin top-left, y down)
//! so subpixel antialiasing survives to the rasterizer; consumers convert
//! logical layout coordinates via `flowshot_core::geometry` before building
//! commands. These types deliberately do not reuse the core newtype spaces
//! (`PhysicalPx(i32)` / `Logical(f64)`): the renderer needs subpixel `f32`
//! precision, and stroke widths are physical by contract (plan todo 14(a)).

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
    fn validity_rejects_nan_and_negative_size() {
        assert!(Rect::from_parts(0.0, 0.0, 1.0, 1.0).is_valid());
        assert!(!Rect::from_parts(f32::NAN, 0.0, 1.0, 1.0).is_valid());
        assert!(!Rect::from_parts(0.0, 0.0, -1.0, 1.0).is_valid());
        assert!(!Size::new(1.0, f32::INFINITY).is_valid());
    }
}

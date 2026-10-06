//! Physical-first geometry algebra.
//!
//! # Design rules
//!
//! 1. **Physical pixels are the source of truth.** Outputs report a native
//!    buffer size in physical pixels; logical geometry is *derived* from it via
//!    the per-output scale factor. Crops are always computed per-output in
//!    physical pixels - never by rescaling an averaged factor across outputs.
//! 2. **Double scaling is unrepresentable.** The [`PhysicalPx`] and [`Logical`]
//!    newtypes are distinct: [`ToLogical::to_logical`] is only implemented for
//!    physical-space types and [`ToPhysical::to_physical`] only for
//!    logical-space types. There is exactly one conversion function per
//!    direction, so a value that has already been scaled cannot be scaled
//!    again by accident - the compiler rejects it.
//! 3. **Purity.** This module depends only on `std`, `serde`, and `thiserror`.
//!    No windowing or protocol crates are imported.
//!
//! # Spaces
//!
//! Geometry primitives ([`Point`], [`Size`], [`Rect`]) are generic over their
//! coordinate type. The aliases [`PhysicalPoint`], [`PhysicalSize`],
//! [`PhysicalRect`] carry [`PhysicalPx`] (`i32`) coordinates; [`LogicalPoint`],
//! [`LogicalSize`], [`LogicalRect`] carry [`Logical`] (`f64`) coordinates.
//!
//! # Example
//!
//! ```
//! use flowshot_core::geometry::{Logical, PhysicalPx, ToLogical, ToPhysical};
//!
//! let px = PhysicalPx(3840);
//! let logical = px.to_logical(2.0);
//! assert_eq!(logical, Logical(1920.0));
//! assert_eq!(logical.to_physical(2.0), px);
//! ```

use std::fmt::Debug;
use std::ops::{Add, Sub};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors produced by geometry operations.
#[derive(Error, Debug, Clone, Copy, PartialEq)]
pub enum GeometryError {
    /// A scale factor was not finite and positive.
    #[error("scale factor must be finite and positive, got {0}")]
    InvalidScale(f64),

    /// A logical coordinate was not finite.
    #[error("logical coordinate must be finite, got {0}")]
    NonFiniteLogical(f64),

    /// A physical dimension was negative.
    #[error("physical dimension must be non-negative, got {0:?}")]
    NegativePhysical(PhysicalPx),

    /// An integer did not map to a known transform.
    #[error("invalid transform value {0}: expected 0..=7")]
    InvalidTransform(u32),

    /// A buffer was too small for the requested remap.
    #[error("buffer too small: need {needed} elements, got {actual}")]
    BufferTooSmall {
        /// Number of elements required.
        needed: usize,
        /// Number of elements actually available.
        actual: usize,
    },

    /// Buffer dimensions overflowed `usize` when multiplied.
    #[error("buffer dimensions overflow: {width}x{height}x{channels}")]
    DimensionsOverflow {
        /// Buffer width in pixels.
        width: usize,
        /// Buffer height in pixels.
        height: usize,
        /// Elements per pixel.
        channels: usize,
    },
}

/// Returns `scale` when it is finite and positive, otherwise `1.0`.
///
/// Conversions are total functions: instead of panicking on a nonsensical
/// scale they fall back to `1.0`. Validated construction happens in
/// [`OutputInfo::new`], which rejects invalid scales with a
/// [`GeometryError`].
fn effective_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// Rounds an `f64` to the nearest `i32`, saturating at the `i32` bounds.
///
/// `NaN` maps to `0` (Rust's defined float-to-int cast semantics).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn round_to_i32(value: f64) -> i32 {
    value.round() as i32
}

/// A scalar in physical pixel space.
///
/// Together with [`Logical`], this newtype makes double scaling
/// unrepresentable: only physical values can be converted *to* logical, and
/// only logical values can be converted *to* physical.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct PhysicalPx(pub i32);

/// A scalar in logical (scale-adjusted) space.
///
/// Logical coordinates are fractional; see [`PhysicalPx`] for the space
/// separation contract.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
pub struct Logical(pub f64);

impl Add for PhysicalPx {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }
}

impl Sub for PhysicalPx {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl Add for Logical {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Logical {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

/// A coordinate scalar usable in geometry algebra.
pub trait Coord:
    Copy + Default + Debug + PartialEq + PartialOrd + Add<Output = Self> + Sub<Output = Self>
{
    /// The additive identity (`0`).
    fn zero() -> Self;

    /// The smaller of two coordinates.
    #[must_use]
    fn min(self, other: Self) -> Self;

    /// The larger of two coordinates.
    #[must_use]
    fn max(self, other: Self) -> Self;
}

impl Coord for PhysicalPx {
    fn zero() -> Self {
        Self(0)
    }

    fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }
}

impl Coord for Logical {
    fn zero() -> Self {
        Self(0.0)
    }

    fn min(self, other: Self) -> Self {
        // f64::min ignores a NaN operand; NaN never reaches here for validated
        // layouts (see `OutputInfo::new`).
        Self(self.0.min(other.0))
    }

    fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }
}

/// The one and only conversion from physical to logical space.
///
/// Implemented exclusively for physical-space types; there is deliberately no
/// `to_logical` on anything already logical, which makes double scaling a
/// type error.
pub trait ToLogical {
    /// The logical-space counterpart type.
    type Output;

    /// Divides by `scale` to obtain logical coordinates.
    ///
    /// A `scale` that is not finite and positive falls back to `1.0` so the
    /// conversion stays total; use [`OutputInfo::new`] for validated
    /// construction.
    #[must_use]
    fn to_logical(self, scale: f64) -> Self::Output;
}

/// The one and only conversion from logical to physical space.
///
/// Implemented exclusively for logical-space types; there is deliberately no
/// `to_physical` on anything already physical, which makes double scaling a
/// type error.
pub trait ToPhysical {
    /// The physical-space counterpart type.
    type Output;

    /// Multiplies by `scale` and rounds half-away-from-zero to obtain physical
    /// pixel coordinates. Out-of-range values saturate at the `i32` bounds.
    ///
    /// A `scale` that is not finite and positive falls back to `1.0` so the
    /// conversion stays total.
    #[must_use]
    fn to_physical(self, scale: f64) -> Self::Output;
}

impl ToLogical for PhysicalPx {
    type Output = Logical;

    fn to_logical(self, scale: f64) -> Logical {
        Logical(f64::from(self.0) / effective_scale(scale))
    }
}

impl ToPhysical for Logical {
    type Output = PhysicalPx;

    fn to_physical(self, scale: f64) -> PhysicalPx {
        PhysicalPx(round_to_i32(self.0 * effective_scale(scale)))
    }
}

/// A point in a coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Point<C> {
    /// Horizontal coordinate.
    pub x: C,
    /// Vertical coordinate.
    pub y: C,
}

/// A size in a coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Size<C> {
    /// Width.
    pub width: C,
    /// Height.
    pub height: C,
}

/// An axis-aligned rectangle in a coordinate space.
///
/// Rectangles are half-open: they include the top/left edges (`x`, `y`) and
/// exclude the bottom/right edges (`x + width`, `y + height`). A rectangle
/// with a non-positive width or height is empty and contains nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Rect<C> {
    /// Horizontal coordinate of the top-left corner.
    pub x: C,
    /// Vertical coordinate of the top-left corner.
    pub y: C,
    /// Width.
    pub width: C,
    /// Height.
    pub height: C,
}

/// A point in physical pixel space.
pub type PhysicalPoint = Point<PhysicalPx>;
/// A point in logical space.
pub type LogicalPoint = Point<Logical>;
/// A size in physical pixel space.
pub type PhysicalSize = Size<PhysicalPx>;
/// A size in logical space.
pub type LogicalSize = Size<Logical>;
/// A rectangle in physical pixel space.
pub type PhysicalRect = Rect<PhysicalPx>;
/// A rectangle in logical space.
pub type LogicalRect = Rect<Logical>;

impl<C: Coord> Point<C> {
    /// Creates a point from coordinates.
    #[must_use]
    pub const fn new(x: C, y: C) -> Self {
        Self { x, y }
    }

    /// The origin point (`0, 0`).
    #[must_use]
    pub fn zero() -> Self {
        Self {
            x: C::zero(),
            y: C::zero(),
        }
    }
}

impl<C: Coord> Size<C> {
    /// Creates a size from dimensions.
    #[must_use]
    pub const fn new(width: C, height: C) -> Self {
        Self { width, height }
    }

    /// The zero size (`0 x 0`).
    #[must_use]
    pub fn zero() -> Self {
        Self {
            width: C::zero(),
            height: C::zero(),
        }
    }

    /// Returns `true` when either dimension is non-positive.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width <= C::zero() || self.height <= C::zero()
    }
}

impl<C: Coord> Rect<C> {
    /// Creates a rectangle from origin and dimensions.
    #[must_use]
    pub const fn new(x: C, y: C, width: C, height: C) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates a rectangle from an origin point and a size.
    #[must_use]
    pub const fn from_parts(origin: Point<C>, size: Size<C>) -> Self {
        Self {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        }
    }

    /// The top-left corner.
    #[must_use]
    pub const fn origin(&self) -> Point<C> {
        Point {
            x: self.x,
            y: self.y,
        }
    }

    /// The dimensions.
    #[must_use]
    pub const fn size(&self) -> Size<C> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    /// The exclusive right edge (`x + width`).
    #[must_use]
    pub fn right(&self) -> C {
        self.x + self.width
    }

    /// The exclusive bottom edge (`y + height`).
    #[must_use]
    pub fn bottom(&self) -> C {
        self.y + self.height
    }

    /// Returns `true` when the rectangle covers no area.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width <= C::zero() || self.height <= C::zero()
    }

    /// Returns `true` when `point` lies inside the half-open rectangle.
    #[must_use]
    pub fn contains_point(&self, point: Point<C>) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }

    /// Returns `true` when `other` is fully inside `self`.
    ///
    /// Empty rectangles are contained by every rectangle.
    #[must_use]
    pub fn contains_rect(&self, other: &Self) -> bool {
        other.is_empty()
            || (other.x >= self.x
                && other.y >= self.y
                && other.right() <= self.right()
                && other.bottom() <= self.bottom())
    }

    /// The overlap of two rectangles, or `None` when they do not overlap.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right > x && bottom > y {
            Some(Self::new(x, y, right - x, bottom - y))
        } else {
            None
        }
    }

    /// Returns `true` when the rectangles overlap with positive area.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.intersection(other).is_some()
    }

    /// The smallest rectangle containing both rectangles.
    ///
    /// Empty operands are ignored: the union with an empty rectangle is the
    /// other operand.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(x, y, right - x, bottom - y)
    }
}

impl PhysicalPoint {
    /// Creates a physical point from raw `i32` values.
    #[must_use]
    pub const fn from_raw(x: i32, y: i32) -> Self {
        Self {
            x: PhysicalPx(x),
            y: PhysicalPx(y),
        }
    }
}

impl LogicalPoint {
    /// Creates a logical point from raw `f64` values.
    #[must_use]
    pub const fn from_raw(x: f64, y: f64) -> Self {
        Self {
            x: Logical(x),
            y: Logical(y),
        }
    }
}

impl PhysicalSize {
    /// Creates a physical size from raw `i32` values.
    #[must_use]
    pub const fn from_raw(width: i32, height: i32) -> Self {
        Self {
            width: PhysicalPx(width),
            height: PhysicalPx(height),
        }
    }

    /// The area in physical pixels, as `i64` to avoid overflow.
    #[must_use]
    pub fn area(self) -> i64 {
        i64::from(self.width.0) * i64::from(self.height.0)
    }
}

impl LogicalSize {
    /// Creates a logical size from raw `f64` values.
    #[must_use]
    pub const fn from_raw(width: f64, height: f64) -> Self {
        Self {
            width: Logical(width),
            height: Logical(height),
        }
    }
}

impl PhysicalRect {
    /// Creates a physical rectangle from raw `i32` values.
    #[must_use]
    pub const fn from_raw(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x: PhysicalPx(x),
            y: PhysicalPx(y),
            width: PhysicalPx(width),
            height: PhysicalPx(height),
        }
    }
}

impl LogicalRect {
    /// Creates a logical rectangle from raw `f64` values.
    #[must_use]
    pub const fn from_raw(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x: Logical(x),
            y: Logical(y),
            width: Logical(width),
            height: Logical(height),
        }
    }

    /// Returns `true` when all coordinates are finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.x.0.is_finite()
            && self.y.0.is_finite()
            && self.width.0.is_finite()
            && self.height.0.is_finite()
    }
}

impl ToLogical for Point<PhysicalPx> {
    type Output = Point<Logical>;

    fn to_logical(self, scale: f64) -> Point<Logical> {
        Point {
            x: self.x.to_logical(scale),
            y: self.y.to_logical(scale),
        }
    }
}

impl ToLogical for Size<PhysicalPx> {
    type Output = Size<Logical>;

    fn to_logical(self, scale: f64) -> Size<Logical> {
        Size {
            width: self.width.to_logical(scale),
            height: self.height.to_logical(scale),
        }
    }
}

impl ToLogical for Rect<PhysicalPx> {
    type Output = Rect<Logical>;

    fn to_logical(self, scale: f64) -> Rect<Logical> {
        Rect {
            x: self.x.to_logical(scale),
            y: self.y.to_logical(scale),
            width: self.width.to_logical(scale),
            height: self.height.to_logical(scale),
        }
    }
}

impl ToPhysical for Point<Logical> {
    type Output = Point<PhysicalPx>;

    fn to_physical(self, scale: f64) -> Point<PhysicalPx> {
        Point {
            x: self.x.to_physical(scale),
            y: self.y.to_physical(scale),
        }
    }
}

impl ToPhysical for Size<Logical> {
    type Output = Size<PhysicalPx>;

    fn to_physical(self, scale: f64) -> Size<PhysicalPx> {
        Size {
            width: self.width.to_physical(scale),
            height: self.height.to_physical(scale),
        }
    }
}

impl ToPhysical for Rect<Logical> {
    type Output = Rect<PhysicalPx>;

    fn to_physical(self, scale: f64) -> Rect<PhysicalPx> {
        Rect {
            x: self.x.to_physical(scale),
            y: self.y.to_physical(scale),
            width: self.width.to_physical(scale),
            height: self.height.to_physical(scale),
        }
    }
}

/// An output transform, matching the eight `wl_output` transform values.
///
/// Flipped variants flip around the vertical axis first, then rotate
/// counter-clockwise, per the Wayland output convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Transform {
    /// No transform.
    #[default]
    Normal,
    /// Rotated 90 degrees counter-clockwise.
    Rot90,
    /// Rotated 180 degrees.
    Rot180,
    /// Rotated 270 degrees counter-clockwise.
    Rot270,
    /// Flipped around the vertical axis.
    Flipped,
    /// Flipped, then rotated 90 degrees counter-clockwise.
    Flipped90,
    /// Flipped, then rotated 180 degrees.
    Flipped180,
    /// Flipped, then rotated 270 degrees counter-clockwise.
    Flipped270,
}

impl Transform {
    /// All transforms, ordered by their conventional wire values (`0..=7`).
    pub const ALL: [Self; 8] = [
        Self::Normal,
        Self::Rot90,
        Self::Rot180,
        Self::Rot270,
        Self::Flipped,
        Self::Flipped90,
        Self::Flipped180,
        Self::Flipped270,
    ];

    /// Returns `true` when the transform swaps width and height.
    #[must_use]
    pub const fn swaps_dimensions(self) -> bool {
        matches!(
            self,
            Self::Rot90 | Self::Rot270 | Self::Flipped90 | Self::Flipped270
        )
    }

    /// The size of a buffer after this transform is applied.
    #[must_use]
    pub fn apply_to_size<C: Coord>(self, size: Size<C>) -> Size<C> {
        if self.swaps_dimensions() {
            Size::new(size.height, size.width)
        } else {
            size
        }
    }

    /// The transform that undoes this one.
    ///
    /// All flipped variants are reflections (involutions); rotations invert by
    /// rotating the opposite way.
    #[must_use]
    pub const fn inverse(self) -> Self {
        match self {
            Self::Normal => Self::Normal,
            Self::Rot90 => Self::Rot270,
            Self::Rot180 => Self::Rot180,
            Self::Rot270 => Self::Rot90,
            Self::Flipped => Self::Flipped,
            Self::Flipped90 => Self::Flipped90,
            Self::Flipped180 => Self::Flipped180,
            Self::Flipped270 => Self::Flipped270,
        }
    }

    /// Maps a source pixel `(x, y)` to its destination in the transformed
    /// buffer of a `src_width` x `src_height` source.
    ///
    /// Precondition: `x < src_width` and `y < src_height`. Out-of-range inputs
    /// saturate at the edges instead of panicking.
    #[must_use]
    pub fn map_point(
        self,
        x: usize,
        y: usize,
        src_width: usize,
        src_height: usize,
    ) -> (usize, usize) {
        let last_x = src_width.saturating_sub(1);
        let last_y = src_height.saturating_sub(1);
        match self {
            Self::Normal => (x, y),
            Self::Rot90 => (y, last_x.saturating_sub(x)),
            Self::Rot180 => (last_x.saturating_sub(x), last_y.saturating_sub(y)),
            Self::Rot270 => (last_y.saturating_sub(y), x),
            Self::Flipped => (last_x.saturating_sub(x), y),
            Self::Flipped90 => (y, x),
            Self::Flipped180 => (x, last_y.saturating_sub(y)),
            Self::Flipped270 => (last_y.saturating_sub(y), last_x.saturating_sub(x)),
        }
    }

    /// Remaps a pixel buffer through this transform.
    ///
    /// `src` holds a `width` x `height` image with `channels` elements per
    /// pixel (e.g. `4` for RGBA8). `dst` receives the transformed image and
    /// must have room for the same number of elements; its dimensions are
    /// [`Transform::apply_to_size`] of the source dimensions. `src` and `dst`
    /// must not overlap (the borrow checker enforces this).
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::DimensionsOverflow`] when
    /// `width * height * channels` overflows `usize`, and
    /// [`GeometryError::BufferTooSmall`] when either buffer holds fewer than
    /// `width * height * channels` elements.
    pub fn remap_buffer<T: Copy>(
        self,
        src: &[T],
        dst: &mut [T],
        width: usize,
        height: usize,
        channels: usize,
    ) -> Result<(), GeometryError> {
        let pixels = width
            .checked_mul(height)
            .ok_or(GeometryError::DimensionsOverflow {
                width,
                height,
                channels,
            })?;
        let needed = pixels
            .checked_mul(channels)
            .ok_or(GeometryError::DimensionsOverflow {
                width,
                height,
                channels,
            })?;
        if src.len() < needed {
            return Err(GeometryError::BufferTooSmall {
                needed,
                actual: src.len(),
            });
        }
        if dst.len() < needed {
            return Err(GeometryError::BufferTooSmall {
                needed,
                actual: dst.len(),
            });
        }
        let dst_width = if self.swaps_dimensions() {
            height
        } else {
            width
        };
        for sy in 0..height {
            for sx in 0..width {
                let (dx, dy) = self.map_point(sx, sy, width, height);
                let src_offset = (sy * width + sx) * channels;
                let dst_offset = (dy * dst_width + dx) * channels;
                dst[dst_offset..dst_offset + channels]
                    .copy_from_slice(&src[src_offset..src_offset + channels]);
            }
        }
        Ok(())
    }
}

impl TryFrom<u32> for Transform {
    type Error = GeometryError;

    /// Decodes the conventional wire ordering (`0` = normal, `1` = 90 degrees
    /// counter-clockwise, ..., `4` = flipped, ...).
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::InvalidTransform`] for values outside `0..=7`.
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match usize::try_from(value) {
            Ok(index) => Self::ALL
                .get(index)
                .copied()
                .ok_or(GeometryError::InvalidTransform(value)),
            Err(_) => Err(GeometryError::InvalidTransform(value)),
        }
    }
}

/// Describes a single output in the desktop's coordinate space.
///
/// `physical_size` is the output's native buffer size *before* `transform` is
/// applied; [`OutputInfo::buffer_size`] gives the post-transform size that
/// crops index into. `logical_rect` positions the output in the global
/// logical space shared by all outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    /// Connector name (e.g. `DP-1`).
    pub connector: String,
    /// Human-readable output name (e.g. the monitor model).
    pub name: String,
    /// Position and size in global logical space.
    pub logical_rect: LogicalRect,
    /// Native buffer size in physical pixels, before `transform`.
    pub physical_size: PhysicalSize,
    /// Scale factor converting physical pixels to logical coordinates.
    pub scale: f64,
    /// Output transform (rotation / flip).
    pub transform: Transform,
}

impl OutputInfo {
    /// Creates an output description, validating its numbers.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::InvalidScale`] when `scale` is not finite and
    /// positive, [`GeometryError::NonFiniteLogical`] when any `logical_rect`
    /// coordinate is not finite, and [`GeometryError::NegativePhysical`] when
    /// a `physical_size` dimension is negative.
    pub fn new(
        connector: impl Into<String>,
        name: impl Into<String>,
        logical_rect: LogicalRect,
        physical_size: PhysicalSize,
        scale: f64,
        transform: Transform,
    ) -> Result<Self, GeometryError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(GeometryError::InvalidScale(scale));
        }
        if !logical_rect.is_finite() {
            for value in [
                logical_rect.x.0,
                logical_rect.y.0,
                logical_rect.width.0,
                logical_rect.height.0,
            ] {
                if !value.is_finite() {
                    return Err(GeometryError::NonFiniteLogical(value));
                }
            }
        }
        if physical_size.width.0 < 0 {
            return Err(GeometryError::NegativePhysical(physical_size.width));
        }
        if physical_size.height.0 < 0 {
            return Err(GeometryError::NegativePhysical(physical_size.height));
        }
        Ok(Self {
            connector: connector.into(),
            name: name.into(),
            logical_rect,
            physical_size,
            scale,
            transform,
        })
    }

    /// The physical size of the output's buffer after `transform` is applied
    /// (90/270 degree rotations swap width and height).
    #[must_use]
    pub fn buffer_size(&self) -> PhysicalSize {
        self.transform.apply_to_size(self.physical_size)
    }

    /// The logical size implied by [`OutputInfo::buffer_size`] and `scale`.
    #[must_use]
    pub fn implied_logical_size(&self) -> LogicalSize {
        self.buffer_size().to_logical(self.scale)
    }

    /// The crop rectangle, in this output's post-transform physical buffer,
    /// covering the part of `region` that falls inside this output.
    ///
    /// The crop is computed with *this output's* scale factor only - never an
    /// averaged factor across outputs. Returns `None` when `region` does not
    /// overlap this output, or when the overlap rounds to zero physical
    /// pixels.
    #[must_use]
    pub fn physical_crop(&self, region: LogicalRect) -> Option<PhysicalRect> {
        let intersection = self.logical_rect.intersection(&region)?;
        self.crop_of_intersection(intersection)
    }

    /// Computes the physical crop for an intersection that is already known
    /// to lie within `logical_rect`.
    fn crop_of_intersection(&self, intersection: LogicalRect) -> Option<PhysicalRect> {
        let buffer = self.buffer_size();
        let origin_x = self.logical_rect.x;
        let origin_y = self.logical_rect.y;
        // Convert edges (not widths) so adjacent crops share exact boundaries.
        let x0 = (intersection.x - origin_x)
            .to_physical(self.scale)
            .0
            .clamp(0, buffer.width.0);
        let x1 = (intersection.right() - origin_x)
            .to_physical(self.scale)
            .0
            .clamp(0, buffer.width.0);
        let y0 = (intersection.y - origin_y)
            .to_physical(self.scale)
            .0
            .clamp(0, buffer.height.0);
        let y1 = (intersection.bottom() - origin_y)
            .to_physical(self.scale)
            .0
            .clamp(0, buffer.height.0);
        (x1 > x0 && y1 > y0).then(|| PhysicalRect::from_raw(x0, y0, x1 - x0, y1 - y0))
    }
}

/// One output's share of a captured region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputCrop<'a> {
    /// The output this crop belongs to.
    pub output: &'a OutputInfo,
    /// The intersection of the region with the output, in global logical
    /// space.
    pub logical: LogicalRect,
    /// The crop rectangle in the output's post-transform physical buffer.
    pub physical: PhysicalRect,
}

/// The full output arrangement of a desktop.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OutputLayout {
    /// All known outputs.
    pub outputs: Vec<OutputInfo>,
}

impl OutputLayout {
    /// Wraps already-validated outputs into a layout.
    #[must_use]
    pub fn new(outputs: Vec<OutputInfo>) -> Self {
        Self { outputs }
    }

    /// The smallest logical rectangle containing every output, or `None` for
    /// an empty layout.
    #[must_use]
    pub fn union_bounds(&self) -> Option<LogicalRect> {
        self.outputs
            .iter()
            .map(|output| output.logical_rect)
            .reduce(|acc, rect| acc.union(&rect))
    }

    /// The output containing `point`, or `None` when the point lies outside
    /// every output.
    ///
    /// Rectangles are half-open, so a point on a shared boundary belongs to
    /// the output starting there. When outputs overlap, the first one in
    /// [`OutputLayout::outputs`] order wins.
    #[must_use]
    pub fn output_at(&self, point: LogicalPoint) -> Option<&OutputInfo> {
        self.outputs
            .iter()
            .find(|output| output.logical_rect.contains_point(point))
    }

    /// Clamps `region` to the layout's union bounds by intersection.
    ///
    /// Returns `None` when the layout is empty or the region does not overlap
    /// it at all. Clamping is idempotent: clamping an already-clamped region
    /// returns the same region.
    #[must_use]
    pub fn clamp_region_to_layout(&self, region: LogicalRect) -> Option<LogicalRect> {
        let bounds = self.union_bounds()?;
        bounds.intersection(&region)
    }

    /// Intersects `region` with every output and computes each output's
    /// physical crop rectangle.
    ///
    /// Each crop uses its own output's scale factor - a region spanning
    /// outputs with different scales is *never* rescaled by an averaged
    /// factor. Outputs that the region misses (or whose overlap rounds to
    /// zero physical pixels) are omitted.
    #[must_use]
    pub fn crop_rects(&self, region: LogicalRect) -> Vec<OutputCrop<'_>> {
        self.outputs
            .iter()
            .filter_map(|output| {
                let logical = output.logical_rect.intersection(&region)?;
                let physical = output.crop_of_intersection(logical)?;
                Some(OutputCrop {
                    output,
                    logical,
                    physical,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::similar_names,
        clippy::many_single_char_names,
        clippy::too_many_lines
    )]

    use super::*;
    use proptest::prelude::*;

    // ---------- helpers ----------

    fn rect_approx_eq(a: LogicalRect, b: LogicalRect, tol: f64) -> bool {
        (a.x.0 - b.x.0).abs() <= tol
            && (a.y.0 - b.y.0).abs() <= tol
            && (a.width.0 - b.width.0).abs() <= tol
            && (a.height.0 - b.height.0).abs() <= tol
    }

    fn remap_ok(t: Transform, src: &[u8], w: usize, h: usize, channels: usize) -> Vec<u8> {
        let (dw, dh) = if t.swaps_dimensions() { (h, w) } else { (w, h) };
        let mut dst = vec![0u8; dw * dh * channels];
        match t.remap_buffer(src, &mut dst, w, h, channels) {
            Ok(()) => dst,
            Err(e) => panic!("remap_buffer failed: {e}"),
        }
    }

    fn output(
        connector: &str,
        rect: LogicalRect,
        size: PhysicalSize,
        scale: f64,
        transform: Transform,
    ) -> OutputInfo {
        OutputInfo::new(connector, connector, rect, size, scale, transform)
            .unwrap_or_else(|e| panic!("test output must be valid: {e}"))
    }

    // ---------- strategies ----------

    fn scales() -> impl Strategy<Value = f64> {
        prop_oneof![
            Just(1.0),
            Just(1.25),
            Just(1.5),
            Just(2.0),
            Just(2.5),
            Just(3.0),
            0.5f64..4.0,
        ]
    }

    fn integral_scales() -> impl Strategy<Value = f64> {
        prop_oneof![Just(1.0), Just(2.0)]
    }

    fn transforms() -> impl Strategy<Value = Transform> {
        (0..Transform::ALL.len()).prop_map(|i| Transform::ALL[i])
    }

    fn involutions() -> impl Strategy<Value = Transform> {
        prop_oneof![
            Just(Transform::Normal),
            Just(Transform::Rot180),
            Just(Transform::Flipped),
            Just(Transform::Flipped180),
        ]
    }

    fn logical_rects_any() -> impl Strategy<Value = LogicalRect> {
        (
            -10_000.0f64..10_000.0,
            -10_000.0f64..10_000.0,
            0.0f64..10_000.0,
            0.0f64..10_000.0,
        )
            .prop_map(|(x, y, w, h)| LogicalRect::from_raw(x, y, w, h))
    }

    fn logical_rects_nonempty_any() -> impl Strategy<Value = LogicalRect> {
        (
            -10_000.0f64..10_000.0,
            -10_000.0f64..10_000.0,
            1.0f64..10_000.0,
            1.0f64..10_000.0,
        )
            .prop_map(|(x, y, w, h)| LogicalRect::from_raw(x, y, w, h))
    }

    fn logical_rects_integral() -> impl Strategy<Value = LogicalRect> {
        (-1000i32..1000, -1000i32..1000, 0i32..1000, 0i32..1000).prop_map(|(x, y, w, h)| {
            LogicalRect::from_raw(f64::from(x), f64::from(y), f64::from(w), f64::from(h))
        })
    }

    fn logical_rects_nonempty_integral() -> impl Strategy<Value = LogicalRect> {
        (-1000i32..1000, -1000i32..1000, 1i32..1000, 1i32..1000).prop_map(|(x, y, w, h)| {
            LogicalRect::from_raw(f64::from(x), f64::from(y), f64::from(w), f64::from(h))
        })
    }

    fn physical_rects_any() -> impl Strategy<Value = PhysicalRect> {
        (any::<i32>(), any::<i32>(), any::<i32>(), any::<i32>())
            .prop_map(|(x, y, w, h)| PhysicalRect::from_raw(x, y, w, h))
    }

    type OutputSpec = (i32, i32, f64, Transform);

    fn general_specs() -> impl Strategy<Value = Vec<OutputSpec>> {
        prop::collection::vec((1i32..2048, 1i32..2048, scales(), transforms()), 1..4)
    }

    fn integral_specs() -> impl Strategy<Value = Vec<OutputSpec>> {
        // Even physical dimensions with scale 1 or 2 give integral logical sizes.
        prop::collection::vec(
            (
                (1i32..1024).prop_map(|k| 2 * k),
                (1i32..1024).prop_map(|k| 2 * k),
                integral_scales(),
                transforms(),
            ),
            1..4,
        )
    }

    /// Builds a left-to-right row of outputs at y = 0, each consistent with
    /// its own scale and transform (logical size == buffer size / scale).
    fn build_layout(specs: Vec<OutputSpec>) -> OutputLayout {
        let mut cursor_x = 0.0f64;
        let mut outputs = Vec::with_capacity(specs.len());
        for (index, (pw, ph, scale, transform)) in specs.into_iter().enumerate() {
            let physical_size = PhysicalSize::from_raw(pw, ph);
            let logical_size = transform.apply_to_size(physical_size).to_logical(scale);
            let rect = LogicalRect::new(
                Logical(cursor_x),
                Logical(0.0),
                logical_size.width,
                logical_size.height,
            );
            cursor_x += logical_size.width.0;
            outputs.push(output(
                &format!("DP-{index}"),
                rect,
                physical_size,
                scale,
                transform,
            ));
        }
        OutputLayout::new(outputs)
    }

    fn general_layouts() -> impl Strategy<Value = OutputLayout> {
        general_specs().prop_map(build_layout)
    }

    fn general_layout_and_region() -> impl Strategy<Value = (OutputLayout, LogicalRect)> {
        general_specs().prop_flat_map(|specs| {
            let layout = build_layout(specs);
            let bounds = layout
                .union_bounds()
                .expect("layout has at least one output");
            let span_w = bounds.width.0;
            let span_h = bounds.height.0;
            (
                -100.0f64..(span_w + 100.0),
                -100.0f64..(span_h + 100.0),
                0.0f64..(span_w + 200.0),
                0.0f64..(span_h + 200.0),
            )
                .prop_map(move |(x, y, w, h)| (layout.clone(), LogicalRect::from_raw(x, y, w, h)))
        })
    }

    fn integral_layout_and_region() -> impl Strategy<Value = (OutputLayout, LogicalRect)> {
        integral_specs().prop_flat_map(|specs| {
            let layout = build_layout(specs);
            let bounds = layout
                .union_bounds()
                .expect("layout has at least one output");
            let span_w = bounds.width.0 as i32;
            let span_h = bounds.height.0 as i32;
            (
                -50i32..(span_w + 50),
                -50i32..(span_h + 50),
                0i32..(span_w + 100),
                0i32..(span_h + 100),
            )
                .prop_map(move |(x, y, w, h)| {
                    (
                        layout.clone(),
                        LogicalRect::from_raw(
                            f64::from(x),
                            f64::from(y),
                            f64::from(w),
                            f64::from(h),
                        ),
                    )
                })
        })
    }

    fn integral_layout_and_inner_region() -> impl Strategy<Value = (OutputLayout, LogicalRect)> {
        integral_specs().prop_flat_map(|specs| {
            let layout = build_layout(specs);
            let bounds = layout
                .union_bounds()
                .expect("layout has at least one output");
            let span_w = bounds.width.0 as i32;
            let span_h = bounds.height.0 as i32;
            (0i32..span_w).prop_flat_map({
                let layout_x = layout.clone();
                move |x| {
                    (1i32..=(span_w - x)).prop_flat_map({
                        let layout_w = layout_x.clone();
                        move |w| {
                            (0i32..span_h).prop_flat_map({
                                let layout_y = layout_w.clone();
                                move |y| {
                                    let layout_h = layout_y.clone();
                                    (1i32..=(span_h - y)).prop_map(move |h| {
                                        (
                                            layout_h.clone(),
                                            LogicalRect::from_raw(
                                                f64::from(x),
                                                f64::from(y),
                                                f64::from(w),
                                                f64::from(h),
                                            ),
                                        )
                                    })
                                }
                            })
                        }
                    })
                }
            })
        })
    }

    fn arb_buffer() -> impl Strategy<Value = (usize, usize, usize, Vec<u8>)> {
        (
            1usize..10,
            1usize..10,
            prop_oneof![Just(1usize), Just(3), Just(4)],
        )
            .prop_flat_map(|(w, h, channels)| {
                prop::collection::vec(any::<u8>(), w * h * channels)
                    .prop_map(move |data| (w, h, channels, data))
            })
    }

    fn transform_and_buffer() -> impl Strategy<Value = (Transform, (usize, usize, usize, Vec<u8>))>
    {
        transforms().prop_flat_map(|t| arb_buffer().prop_map(move |b| (t, b)))
    }

    fn involution_and_buffer() -> impl Strategy<Value = (Transform, (usize, usize, usize, Vec<u8>))>
    {
        involutions().prop_flat_map(|t| arb_buffer().prop_map(move |b| (t, b)))
    }

    // ---------- property tests ----------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        // --- conversions ---

        #[test]
        fn prop_physical_to_logical_to_physical_exact(v in any::<i32>(), s in scales()) {
            prop_assert_eq!(PhysicalPx(v).to_logical(s).to_physical(s), PhysicalPx(v));
        }

        #[test]
        fn prop_logical_grid_roundtrip_within_1e9(k in -1_000_000i32..1_000_000, s in scales()) {
            let l = Logical(f64::from(k) / s);
            let back = l.to_physical(s).to_logical(s);
            prop_assert!(
                (back.0 - l.0).abs() <= 1e-9,
                "roundtrip drifted: {l:?} -> {back:?} at scale {s}"
            );
        }

        #[test]
        fn prop_to_logical_monotonic(a in any::<i32>(), b in any::<i32>(), s in scales()) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(PhysicalPx(lo).to_logical(s).0 <= PhysicalPx(hi).to_logical(s).0);
        }

        #[test]
        fn prop_to_physical_monotonic(a in -1e9f64..1e9, b in -1e9f64..1e9, s in scales()) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(Logical(lo).to_physical(s).0 <= Logical(hi).to_physical(s).0);
        }

        #[test]
        fn prop_to_physical_rounds_within_half_pixel(l in -1e6f64..1e6, s in scales()) {
            let p = Logical(l).to_physical(s);
            let back = f64::from(p.0) / s;
            prop_assert!(
                (back - l).abs() <= 0.5 / s + 1e-9,
                "l={l} s={s} p={p:?} back={back}"
            );
        }

        #[test]
        fn prop_rect_conversion_roundtrip_exact(r in physical_rects_any(), s in scales()) {
            prop_assert_eq!(r.to_logical(s).to_physical(s), r);
        }

        // --- rect algebra ---

        #[test]
        fn prop_intersection_commutative(a in logical_rects_any(), b in logical_rects_any()) {
            prop_assert_eq!(a.intersection(&b), b.intersection(&a));
        }

        #[test]
        fn prop_intersection_subset_of_both(a in logical_rects_integral(), b in logical_rects_integral()) {
            if let Some(i) = a.intersection(&b) {
                prop_assert!(a.contains_rect(&i), "{i:?} not inside {a:?}");
                prop_assert!(b.contains_rect(&i), "{i:?} not inside {b:?}");
            }
        }

        #[test]
        fn prop_self_intersection_exact_integral(r in logical_rects_integral()) {
            let expected = if r.is_empty() { None } else { Some(r) };
            prop_assert_eq!(r.intersection(&r), expected);
        }

        #[test]
        fn prop_union_commutative(a in logical_rects_nonempty_any(), b in logical_rects_nonempty_any()) {
            prop_assert_eq!(a.union(&b), b.union(&a));
        }

        #[test]
        fn prop_union_contains_both(a in logical_rects_integral(), b in logical_rects_integral()) {
            let u = a.union(&b);
            prop_assert!(u.contains_rect(&a), "{a:?} not inside {u:?}");
            prop_assert!(u.contains_rect(&b), "{b:?} not inside {u:?}");
        }

        #[test]
        fn prop_union_self_exact_integral(r in logical_rects_nonempty_integral()) {
            prop_assert_eq!(r.union(&r), r);
        }

        #[test]
        fn prop_intersection_point_consistency_integral(
            a in logical_rects_integral(),
            b in logical_rects_integral()
        ) {
            if let Some(i) = a.intersection(&b) {
                let p = LogicalPoint::from_raw(
                    i.x.0 + i.width.0 / 2.0,
                    i.y.0 + i.height.0 / 2.0,
                );
                prop_assert!(i.contains_point(p));
                prop_assert!(a.contains_point(p));
                prop_assert!(b.contains_point(p));
            }
        }

        // --- layout ---

        #[test]
        fn prop_union_bounds_contains_all_outputs(layout in general_layouts()) {
            let bounds = layout.union_bounds().expect("layout is non-empty");
            for o in &layout.outputs {
                prop_assert!(
                    bounds.contains_rect(&o.logical_rect),
                    "{:?} not inside bounds {:?}",
                    o.logical_rect,
                    bounds
                );
            }
        }

        #[test]
        fn prop_output_at_finds_owner(layout in general_layouts()) {
            for o in &layout.outputs {
                let center = LogicalPoint::from_raw(
                    o.logical_rect.x.0 + o.logical_rect.width.0 / 2.0,
                    o.logical_rect.y.0 + o.logical_rect.height.0 / 2.0,
                );
                let found = layout.output_at(center).expect("center lies inside its own output");
                prop_assert_eq!(found.connector.as_str(), o.connector.as_str());
            }
        }

        #[test]
        fn prop_clamp_idempotent_exact_integral((layout, region) in integral_layout_and_region()) {
            if let Some(c1) = layout.clamp_region_to_layout(region) {
                prop_assert_eq!(layout.clamp_region_to_layout(c1), Some(c1));
            }
        }

        #[test]
        fn prop_clamp_idempotent_approx_general((layout, region) in general_layout_and_region()) {
            if let Some(c1) = layout.clamp_region_to_layout(region) {
                match layout.clamp_region_to_layout(c1) {
                    Some(c2) => prop_assert!(
                        rect_approx_eq(c1, c2, 1e-9),
                        "clamp not idempotent: {c1:?} vs {c2:?}"
                    ),
                    // A re-clamp may only vanish when the first clamp left a
                    // sliver thinner than floating-point resolution.
                    None => prop_assert!(
                        c1.width.0 <= 1e-6 || c1.height.0 <= 1e-6,
                        "wide region lost on re-clamp: {c1:?}"
                    ),
                }
            }
        }

        #[test]
        fn prop_clamp_within_bounds_integral((layout, region) in integral_layout_and_region()) {
            let bounds = layout.union_bounds().expect("layout is non-empty");
            if let Some(c) = layout.clamp_region_to_layout(region) {
                prop_assert!(bounds.contains_rect(&c), "{c:?} escapes {bounds:?}");
            }
        }

        #[test]
        fn prop_clamp_preserves_contained_integral(
            (layout, region) in integral_layout_and_inner_region()
        ) {
            prop_assert_eq!(layout.clamp_region_to_layout(region), Some(region));
        }

        // --- crops ---

        #[test]
        fn prop_crop_within_buffer_bounds((layout, region) in general_layout_and_region()) {
            for crop in layout.crop_rects(region) {
                let buffer = crop.output.buffer_size();
                prop_assert!(crop.physical.x.0 >= 0 && crop.physical.y.0 >= 0);
                prop_assert!(crop.physical.right().0 <= buffer.width.0);
                prop_assert!(crop.physical.bottom().0 <= buffer.height.0);
                prop_assert!(!crop.physical.is_empty());
            }
        }

        #[test]
        fn prop_spanning_crop_matches_per_output((layout, region) in general_layout_and_region()) {
            let crops = layout.crop_rects(region);
            for crop in &crops {
                let o = crop.output;
                let s = o.scale;
                // The logical part is exactly this output's intersection.
                prop_assert_eq!(Some(crop.logical), o.logical_rect.intersection(&region));
                // The physical part is that intersection scaled by THIS
                // output's factor (never an averaged one): each edge lands
                // within half a physical pixel of the exact product.
                let lx0 = (crop.logical.x - o.logical_rect.x).0;
                let ly0 = (crop.logical.y - o.logical_rect.y).0;
                let lx1 = (crop.logical.right() - o.logical_rect.x).0;
                let ly1 = (crop.logical.bottom() - o.logical_rect.y).0;
                let tol = 0.5 / s + 1e-9;
                prop_assert!((f64::from(crop.physical.x.0) / s - lx0).abs() <= tol, "x0 edge off for {}", o.connector);
                prop_assert!((f64::from(crop.physical.y.0) / s - ly0).abs() <= tol, "y0 edge off for {}", o.connector);
                prop_assert!((f64::from(crop.physical.right().0) / s - lx1).abs() <= tol, "x1 edge off for {}", o.connector);
                prop_assert!((f64::from(crop.physical.bottom().0) / s - ly1).abs() <= tol, "y1 edge off for {}", o.connector);
            }
            // Every output with a physically meaningful overlap gets a crop.
            for o in &layout.outputs {
                if let Some(inter) = o.logical_rect.intersection(&region)
                    && inter.width.0 * o.scale >= 1.5
                    && inter.height.0 * o.scale >= 1.5
                {
                    prop_assert!(
                        crops.iter().any(|c| c.output.connector == o.connector),
                        "missing crop for {}",
                        o.connector
                    );
                }
            }
        }

        #[test]
        fn prop_crop_area_matches_logical_area((layout, region) in general_layout_and_region()) {
            for crop in layout.crop_rects(region) {
                let s = crop.output.scale;
                let wl = crop.logical.width.0 * s;
                let hl = crop.logical.height.0 * s;
                let area_phys =
                    f64::from(crop.physical.width.0) * f64::from(crop.physical.height.0);
                let tol = wl + hl + 1.0 + 1e-6;
                prop_assert!(
                    (area_phys - wl * hl).abs() <= tol,
                    "area {area_phys} vs {} (tol {tol}) for {}",
                    wl * hl,
                    crop.output.connector
                );
            }
        }

        // --- transforms ---

        #[test]
        fn prop_transform_inverse_roundtrip((t, (w, h, channels, data)) in transform_and_buffer()) {
            let fwd = remap_ok(t, &data, w, h, channels);
            let (fw, fh) = if t.swaps_dimensions() { (h, w) } else { (w, h) };
            let back = remap_ok(t.inverse(), &fwd, fw, fh, channels);
            prop_assert_eq!(back, data);
        }

        #[test]
        fn prop_transform_double_apply_identity((t, (w, h, channels, data)) in involution_and_buffer()) {
            let once = remap_ok(t, &data, w, h, channels);
            let twice = remap_ok(t, &once, w, h, channels);
            prop_assert_eq!(twice, data);
        }

        #[test]
        fn prop_transform_quadruple_apply_identity((t, (w, h, channels, data)) in transform_and_buffer()) {
            let mut cur = data.clone();
            let (mut cw, mut ch) = (w, h);
            for _ in 0..4 {
                cur = remap_ok(t, &cur, cw, ch, channels);
                if t.swaps_dimensions() {
                    std::mem::swap(&mut cw, &mut ch);
                }
            }
            prop_assert_eq!((cw, ch), (w, h));
            prop_assert_eq!(cur, data);
        }

        #[test]
        fn prop_transform_size_apply_twice_identity(
            t in transforms(),
            w in 0i32..4096,
            h in 0i32..4096
        ) {
            let size = PhysicalSize::from_raw(w, h);
            prop_assert_eq!(t.apply_to_size(t.apply_to_size(size)), size);
        }

        #[test]
        fn prop_transform_inverse_is_involutive(t in transforms()) {
            prop_assert_eq!(t.inverse().inverse(), t);
        }

        #[test]
        fn prop_remap_preserves_pixel_multiset((t, (w, h, channels, data)) in transform_and_buffer()) {
            let mut got = remap_ok(t, &data, w, h, channels);
            let mut expected = data.clone();
            got.sort_unstable();
            expected.sort_unstable();
            prop_assert_eq!(got, expected);
        }
    }

    // ---------- unit tests ----------

    #[test]
    fn mixed_scales_crop_per_output_not_averaged() {
        // A: 100x100 physical @ 1.0 covering logical (0,0,100,100).
        // B: 200x200 physical @ 2.0 covering logical (100,0,100,100).
        let layout = OutputLayout::new(vec![
            output(
                "DP-1",
                LogicalRect::from_raw(0.0, 0.0, 100.0, 100.0),
                PhysicalSize::from_raw(100, 100),
                1.0,
                Transform::Normal,
            ),
            output(
                "DP-2",
                LogicalRect::from_raw(100.0, 0.0, 100.0, 100.0),
                PhysicalSize::from_raw(200, 200),
                2.0,
                Transform::Normal,
            ),
        ]);
        let region = LogicalRect::from_raw(50.0, 0.0, 100.0, 100.0);
        let crops = layout.crop_rects(region);
        assert_eq!(crops.len(), 2);
        // A: logical x 50..100 at scale 1 -> physical (50, 0, 50, 100).
        assert_eq!(crops[0].physical, PhysicalRect::from_raw(50, 0, 50, 100));
        // B: logical x 0..50 at scale 2 -> physical (0, 0, 100, 200).
        assert_eq!(crops[1].physical, PhysicalRect::from_raw(0, 0, 100, 200));
        // An averaged factor of 1.5 would have produced this for A - wrong.
        assert_ne!(crops[0].physical, PhysicalRect::from_raw(75, 0, 75, 150));
    }

    #[test]
    fn buffer_size_accounts_for_transform() {
        let o = output(
            "DP-1",
            LogicalRect::from_raw(0.0, 0.0, 540.0, 960.0),
            PhysicalSize::from_raw(1920, 1080),
            2.0,
            Transform::Rot90,
        );
        assert_eq!(o.buffer_size(), PhysicalSize::from_raw(1080, 1920));
        assert_eq!(
            o.implied_logical_size(),
            LogicalSize::from_raw(540.0, 960.0)
        );
        let crop = o
            .physical_crop(LogicalRect::from_raw(0.0, 0.0, 540.0, 960.0))
            .expect("full overlap");
        assert_eq!(crop, PhysicalRect::from_raw(0, 0, 1080, 1920));
    }

    #[test]
    fn transform_map_point_known_vectors() {
        // Source is 3 wide, 2 high.
        assert_eq!(Transform::Normal.map_point(1, 0, 3, 2), (1, 0));
        assert_eq!(Transform::Rot90.map_point(0, 0, 3, 2), (0, 2));
        assert_eq!(Transform::Rot90.map_point(2, 1, 3, 2), (1, 0));
        assert_eq!(Transform::Rot180.map_point(0, 0, 3, 2), (2, 1));
        assert_eq!(Transform::Rot270.map_point(0, 0, 3, 2), (1, 0));
        assert_eq!(Transform::Flipped.map_point(0, 0, 3, 2), (2, 0));
        assert_eq!(Transform::Flipped90.map_point(1, 0, 3, 2), (0, 1));
        assert_eq!(Transform::Flipped180.map_point(0, 0, 3, 2), (0, 1));
        assert_eq!(Transform::Flipped270.map_point(0, 0, 3, 2), (1, 2));
    }

    #[test]
    fn transform_remap_rot90_pixels() {
        // 2x1 source [1, 2] rotated 90 CCW becomes a 1x2 buffer [2, 1].
        let src = [1u8, 2];
        let mut dst = [0u8; 2];
        Transform::Rot90
            .remap_buffer(&src, &mut dst, 2, 1, 1)
            .expect("sizes match");
        assert_eq!(dst, [2, 1]);
    }

    #[test]
    fn output_at_half_open_boundaries() {
        let layout = OutputLayout::new(vec![
            output(
                "DP-1",
                LogicalRect::from_raw(0.0, 0.0, 10.0, 10.0),
                PhysicalSize::from_raw(10, 10),
                1.0,
                Transform::Normal,
            ),
            output(
                "DP-2",
                LogicalRect::from_raw(10.0, 0.0, 10.0, 10.0),
                PhysicalSize::from_raw(10, 10),
                1.0,
                Transform::Normal,
            ),
        ]);
        let at = |x, y| {
            layout
                .output_at(LogicalPoint::from_raw(x, y))
                .map(|o| o.connector.as_str())
        };
        // A shared boundary belongs to the output starting there.
        assert_eq!(at(10.0, 5.0), Some("DP-2"));
        assert_eq!(at(9.999, 5.0), Some("DP-1"));
        // Far edges are exclusive.
        assert_eq!(at(20.0, 5.0), None);
        assert_eq!(at(-0.001, 5.0), None);
    }

    #[test]
    fn empty_layout_has_no_bounds_or_crops() {
        let layout = OutputLayout::new(Vec::new());
        let region = LogicalRect::from_raw(0.0, 0.0, 10.0, 10.0);
        assert_eq!(layout.union_bounds(), None);
        assert_eq!(layout.clamp_region_to_layout(region), None);
        assert_eq!(
            layout.crop_rects(region),
            [] as [crate::geometry::OutputCrop<'_>; 0]
        );
        assert!(layout.output_at(LogicalPoint::from_raw(0.0, 0.0)).is_none());
    }

    #[test]
    fn output_info_rejects_invalid_input() {
        let rect = LogicalRect::from_raw(0.0, 0.0, 10.0, 10.0);
        let size = PhysicalSize::from_raw(10, 10);
        assert_eq!(
            OutputInfo::new("C", "N", rect, size, 0.0, Transform::Normal).unwrap_err(),
            GeometryError::InvalidScale(0.0)
        );
        assert!(matches!(
            OutputInfo::new("C", "N", rect, size, f64::NAN, Transform::Normal),
            Err(GeometryError::InvalidScale(v)) if v.is_nan()
        ));
        assert!(matches!(
            OutputInfo::new(
                "C",
                "N",
                LogicalRect::from_raw(f64::INFINITY, 0.0, 1.0, 1.0),
                size,
                1.0,
                Transform::Normal
            ),
            Err(GeometryError::NonFiniteLogical(_))
        ));
        assert!(matches!(
            OutputInfo::new(
                "C",
                "N",
                rect,
                PhysicalSize::from_raw(-1, 10),
                1.0,
                Transform::Normal
            ),
            Err(GeometryError::NegativePhysical(_))
        ));
    }

    #[test]
    fn conversions_fall_back_to_unit_scale_when_invalid() {
        assert_eq!(PhysicalPx(5).to_logical(0.0), Logical(5.0));
        assert_eq!(PhysicalPx(5).to_logical(f64::NAN), Logical(5.0));
        assert_eq!(Logical(7.0).to_physical(-1.0), PhysicalPx(7));
    }

    #[test]
    fn to_physical_saturates_at_i32_bounds() {
        assert_eq!(Logical(1e30).to_physical(1.0), PhysicalPx(i32::MAX));
        assert_eq!(Logical(-1e30).to_physical(1.0), PhysicalPx(i32::MIN));
        assert_eq!(Logical(f64::NAN).to_physical(1.0), PhysicalPx(0));
    }

    #[test]
    fn remap_buffer_reports_small_buffers() {
        let src = [0u8; 4];
        let mut dst = [0u8; 3];
        assert_eq!(
            Transform::Normal.remap_buffer(&src, &mut dst, 2, 2, 1),
            Err(GeometryError::BufferTooSmall {
                needed: 4,
                actual: 3
            })
        );
        let mut ok = [0u8; 4];
        assert_eq!(
            Transform::Normal.remap_buffer(&src, &mut ok, 2, 2, 1),
            Ok(())
        );
        // A zero-sized remap is a no-op.
        assert_eq!(
            Transform::Rot90.remap_buffer::<u8>(&[], &mut [], 0, 0, 4),
            Ok(())
        );
    }

    #[test]
    fn transform_wire_values_round_trip() {
        for (value, transform) in Transform::ALL.iter().enumerate() {
            let decoded = Transform::try_from(u32::try_from(value).expect("0..8 fits in u32"));
            assert_eq!(decoded, Ok(*transform));
        }
        assert!(matches!(
            Transform::try_from(8),
            Err(GeometryError::InvalidTransform(8))
        ));
    }

    #[test]
    fn rect_basics() {
        let r = PhysicalRect::from_raw(1, 2, 3, 4);
        assert_eq!(r.right(), PhysicalPx(4));
        assert_eq!(r.bottom(), PhysicalPx(6));
        assert_eq!(r.origin(), PhysicalPoint::from_raw(1, 2));
        assert_eq!(r.size(), PhysicalSize::from_raw(3, 4));
        assert!(r.contains_point(PhysicalPoint::from_raw(1, 2)));
        assert!(!r.contains_point(PhysicalPoint::from_raw(4, 2)));
        assert!(PhysicalRect::from_raw(0, 0, 0, 5).is_empty());
        assert_eq!(PhysicalSize::from_raw(3, 4).area(), 12);
    }

    #[test]
    fn single_conversion_per_direction() {
        // `to_logical` exists only on physical types and `to_physical` only on
        // logical types, so a second scaling in the same direction is a
        // compile error (e.g. `Logical(1.0).to_logical(2.0)` does not exist).
        let p = PhysicalPx(100);
        let l = p.to_logical(2.0);
        assert_eq!(l, Logical(50.0));
        assert_eq!(l.to_physical(2.0), p);
    }
}

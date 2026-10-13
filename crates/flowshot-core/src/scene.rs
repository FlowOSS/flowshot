//! Scene graph, snapshot undo, and tool objects for annotation editing.
//!
//! Design overview:
//!
//! - [`Scene`] owns annotation objects in a dense arena (`Vec<Box<dyn ToolObject>>`).
//!   Object ids are arena indices; they are compacted on removal, so ids are
//!   invalidated by [`Scene::remove_object`] and must be re-queried afterwards.
//!   [`Scene::z_order`] is the paint order (bottom-most first, top-most last)
//!   and is always a permutation of `0..object_count`.
//! - [`ToolObject`] is the renderer-agnostic tool trait. Painting goes through
//!   the abstract [`PaintSink`], so the core crate never depends on a GPU or
//!   UI backend. Persistence uses the serde tagged enum [`ToolObjectData`].
//! - [`UndoStack`] stores FULL before/after [`Scene`] snapshots per
//!   modification, bounded by a configurable limit (default
//!   [`DEFAULT_UNDO_LIMIT`]). Undo/redo past the ends are silent no-ops.
//! - Counter renumbering is owned by [`Scene`] (never duplicated in editors):
//!   removing a counter decrements every higher count, and counters added with
//!   an unassigned count (`0`) or re-added via [`Scene::restore_object`] are
//!   numbered with the max+1 rule.

mod arrow;
mod counter;
mod objects;

#[cfg(test)]
pub(crate) mod test_support;

pub use arrow::{ARROW_HEAD_HEIGHT, ARROW_HEAD_WIDTH, ArrowObject};
pub use counter::{
    COUNTER_PADDING, COUNTER_THICKNESS_OFFSET, CounterObject, anti_contrast_color, color_is_dark,
    contrast_color, label_font_size,
};
pub use objects::{InvertObject, LineObject, MarkerObject, PencilPath};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Default bound for the number of undo snapshots kept by [`UndoStack`].
///
/// Configuration should feed its `undo_limit` value into
/// [`UndoStack::with_limit`]; this constant is the fallback default.
pub const DEFAULT_UNDO_LIMIT: usize = 100;

// ---------------------------------------------------------------------------
// Geometry and color
// ---------------------------------------------------------------------------

/// A 2D point in scene coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

impl Point {
    /// Creates a new point.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in scene coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width (non-negative by construction when built via the constructors).
    pub width: f32,
    /// Height (non-negative by construction when built via the constructors).
    pub height: f32,
}

impl Rect {
    /// Creates a rectangle from its origin and size.
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates the normalized rectangle spanning two arbitrary corner points.
    #[must_use]
    pub const fn from_points(a: Point, b: Point) -> Self {
        let x = if a.x < b.x { a.x } else { b.x };
        let y = if a.y < b.y { a.y } else { b.y };
        let max_x = if a.x > b.x { a.x } else { b.x };
        let max_y = if a.y > b.y { a.y } else { b.y };
        Self {
            x,
            y,
            width: max_x - x,
            height: max_y - y,
        }
    }

    /// Returns the geometric center of the rectangle.
    #[must_use]
    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        }
    }

    /// Returns `true` when `point` lies inside (or on the edge of) the rectangle.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }
}

/// An RGBA color with premultiplied-free plain components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel (`255` = opaque).
    pub a: u8,
}

impl Color {
    /// Creates a color from RGBA components.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Returns this color with the alpha channel replaced.
    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self {
            r: self.r,
            g: self.g,
            b: self.b,
            a,
        }
    }
}

// ---------------------------------------------------------------------------
// Paint sink (renderer-agnostic)
// ---------------------------------------------------------------------------

/// Abstract drawing surface consumed by [`ToolObject::paint`].
///
/// This trait is the only "rendering" surface in the core crate: it carries no
/// GPU, font, or UI dependencies. Backends (wgpu, software raster, test mocks)
/// implement it to receive draw primitives.
pub trait PaintSink {
    /// Fills an axis-aligned rectangle.
    fn fill_rect(&mut self, rect: Rect, color: Color);
    /// Strokes the outline of an axis-aligned rectangle with `width` thickness.
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);
    /// Strokes a rounded-corner rectangle outline (`radius = 0` is sharp;
    /// backends clamp the radius to half the smaller side).
    fn stroke_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color, width: f32);
    /// Fills the ellipse inscribed in `rect`.
    fn fill_ellipse(&mut self, rect: Rect, color: Color);
    /// Strokes the ellipse inscribed in `rect` with `width` thickness.
    fn stroke_ellipse(&mut self, rect: Rect, color: Color, width: f32);
    /// Draws a straight line segment with `width` thickness.
    fn draw_line(&mut self, from: Point, to: Point, color: Color, width: f32);
    /// Strokes an open polyline through `points` with `width` thickness
    /// (joined as ONE path - freehand strokes).
    fn stroke_polyline(&mut self, points: &[Point], color: Color, width: f32);
    /// Fills the polygon spanned by `points` (arrow heads, chisel-cap marker
    /// quads - fewer than 3 points paint nothing).
    fn fill_polygon(&mut self, points: &[Point], color: Color);
    /// Inverts the colors of everything painted below `rect` (the
    /// non-destructive region filter; backends without an inversion
    /// capability log and skip).
    fn invert_region(&mut self, rect: Rect);
    /// Draws `text` with its layout box anchored at `position` (top-left).
    fn draw_text(&mut self, position: Point, text: &str, font_size: f32, color: Color);
    /// Draws `text` centered on `center` (backends with font metrics center
    /// the shaped block exactly; the counter digit uses this).
    fn draw_text_centered(&mut self, center: Point, text: &str, style: LabelStyle);
}

/// The style of a centered text label ([`PaintSink::draw_text_centered`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelStyle {
    /// Font size in scene units (logical px).
    pub font_size: f32,
    /// Ink color.
    pub color: Color,
    /// Bold weight (Flameshot's counter digits are bold).
    pub bold: bool,
}

// ---------------------------------------------------------------------------
// Tool objects
// ---------------------------------------------------------------------------

/// Helper supertrait enabling `clone_box` on [`ToolObject`].
///
/// Blanket-implemented for every `Clone + ToolObject + 'static` type, so
/// concrete objects get object-safe cloning for free.
pub trait ToolObjectClone {
    /// Clones this object into a new heap-allocated trait object.
    fn clone_box(&self) -> Box<dyn ToolObject>;
}

impl<T: ToolObject + Clone + 'static> ToolObjectClone for T {
    fn clone_box(&self) -> Box<dyn ToolObject> {
        Box::new(self.clone())
    }
}

/// A user-placed annotation object in the [`Scene`].
///
/// Implementations are plain data plus drawing dispatch; they must stay free
/// of renderer and UI dependencies. Persistence is bridged through
/// [`ToolObject::to_data`] and [`ToolObjectData::into_object`].
pub trait ToolObject: ToolObjectClone + std::fmt::Debug + Send + Sync {
    /// Stable string identifying the tool type (matches the serde tag).
    fn type_id(&self) -> &'static str;

    /// Axis-aligned bounding box of the object in scene coordinates.
    fn bounding_rect(&self) -> Rect;

    /// Emits draw primitives for this object into `sink`.
    fn paint(&self, sink: &mut dyn PaintSink);

    /// Current counter value, or `None` for non-counter objects.
    ///
    /// A value of `0` means "unassigned": [`Scene::add_object`] will number it
    /// with the max+1 rule.
    fn count(&self) -> Option<u32> {
        None
    }

    /// Sets the counter value; no-op for non-counter objects.
    fn set_count(&mut self, _count: u32) {}

    /// Converts this object into its serializable representation.
    fn to_data(&self) -> ToolObjectData;
}

/// Serializable tagged-enum form of every concrete [`ToolObject`].
///
/// The `type` tag matches [`ToolObject::type_id`] for each variant payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolObjectData {
    /// A [`RectObject`].
    Rectangle(RectObject),
    /// An [`EllipseObject`].
    Ellipse(EllipseObject),
    /// An [`ArrowObject`].
    Arrow(ArrowObject),
    /// A [`TextObject`].
    Text(TextObject),
    /// A [`CounterObject`].
    Counter(CounterObject),
    /// A [`PencilPath`].
    Pencil(PencilPath),
    /// A [`LineObject`].
    Line(LineObject),
    /// A [`MarkerObject`].
    Marker(MarkerObject),
    /// An [`InvertObject`].
    Invert(InvertObject),
}

impl ToolObjectData {
    /// Rebuilds a heap-allocated trait object from serializable data.
    #[must_use]
    pub fn into_object(self) -> Box<dyn ToolObject> {
        match self {
            Self::Rectangle(object) => Box::new(object),
            Self::Ellipse(object) => Box::new(object),
            Self::Arrow(object) => Box::new(object),
            Self::Text(object) => Box::new(object),
            Self::Counter(object) => Box::new(object),
            Self::Pencil(object) => Box::new(object),
            Self::Line(object) => Box::new(object),
            Self::Marker(object) => Box::new(object),
            Self::Invert(object) => Box::new(object),
        }
    }
}

/// A rectangle annotation (filled or stroked outline).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RectObject {
    /// Bounding rectangle of the shape.
    pub rect: Rect,
    /// Fill or stroke color.
    pub color: Color,
    /// Stroke thickness (used when `filled` is `false`).
    pub stroke_width: f32,
    /// Whether the shape is filled instead of stroked.
    pub filled: bool,
    /// Corner radius for the stroked outline (`[tools.rectangle].corner_radius`
    /// via the rect tool; `0` = sharp corners, backends clamp to half the
    /// smaller side). Ignored when `filled`.
    #[serde(default)]
    pub corner_radius: f32,
}

impl RectObject {
    /// Creates a stroked rectangle annotation with sharp corners.
    #[must_use]
    pub fn new(rect: Rect, color: Color, stroke_width: f32, filled: bool) -> Self {
        Self {
            rect,
            color,
            stroke_width,
            filled,
            corner_radius: 0.0,
        }
    }

    /// Sets the stroke corner radius (builder).
    #[must_use]
    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }
}

impl ToolObject for RectObject {
    fn type_id(&self) -> &'static str {
        "rectangle"
    }

    fn bounding_rect(&self) -> Rect {
        self.rect
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        if self.filled {
            sink.fill_rect(self.rect, self.color);
        } else {
            sink.stroke_rounded_rect(self.rect, self.corner_radius, self.color, self.stroke_width);
        }
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Rectangle(self.clone())
    }
}

/// An ellipse annotation inscribed in its bounding rectangle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EllipseObject {
    /// Bounding rectangle the ellipse is inscribed in.
    pub rect: Rect,
    /// Fill or stroke color.
    pub color: Color,
    /// Stroke thickness (used when `filled` is `false`).
    pub stroke_width: f32,
    /// Whether the shape is filled instead of stroked.
    pub filled: bool,
}

impl EllipseObject {
    /// Creates an ellipse annotation.
    #[must_use]
    pub fn new(rect: Rect, color: Color, stroke_width: f32, filled: bool) -> Self {
        Self {
            rect,
            color,
            stroke_width,
            filled,
        }
    }
}

impl ToolObject for EllipseObject {
    fn type_id(&self) -> &'static str {
        "ellipse"
    }

    fn bounding_rect(&self) -> Rect {
        self.rect
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        if self.filled {
            sink.fill_ellipse(self.rect, self.color);
        } else {
            sink.stroke_ellipse(self.rect, self.color, self.stroke_width);
        }
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Ellipse(self.clone())
    }
}

/// A text annotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextObject {
    /// Top-left anchor of the text layout box.
    pub position: Point,
    /// The text content.
    pub text: String,
    /// Font size in scene units.
    pub font_size: f32,
    /// Text color.
    pub color: Color,
}

impl TextObject {
    /// Creates a text annotation.
    #[must_use]
    pub fn new(position: Point, text: String, font_size: f32, color: Color) -> Self {
        Self {
            position,
            text,
            font_size,
            color,
        }
    }
}

impl ToolObject for TextObject {
    fn type_id(&self) -> &'static str {
        "text"
    }

    fn bounding_rect(&self) -> Rect {
        // Renderer-agnostic estimate: the core crate has no font metrics, so
        // approximate average glyph advance as 0.6 * font_size.
        let chars = u32::try_from(self.text.chars().count()).unwrap_or(u32::MAX);
        #[allow(clippy::cast_precision_loss)]
        let char_advance = chars as f32;
        let width = char_advance * self.font_size * 0.6;
        let height = self.font_size * 1.2;
        Rect::new(self.position.x, self.position.y, width, height)
    }

    fn paint(&self, sink: &mut dyn PaintSink) {
        sink.draw_text(self.position, &self.text, self.font_size, self.color);
    }

    fn to_data(&self) -> ToolObjectData {
        ToolObjectData::Text(self.clone())
    }
}

// ---------------------------------------------------------------------------
// Scene
// ---------------------------------------------------------------------------

/// Errors produced by scene operations.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum SceneError {
    /// The z-order list length did not match the object count.
    #[error("invalid z-order: expected {expected} entries for {expected} objects, found {actual}")]
    ZOrderLengthMismatch {
        /// Number of objects (and expected z-order entries).
        expected: usize,
        /// Number of z-order entries actually present.
        actual: usize,
    },
    /// The z-order referenced a nonexistent object index.
    #[error("z-order references out-of-range object index {0}")]
    ZOrderIndexOutOfRange(usize),
    /// The z-order listed the same object more than once.
    #[error("z-order contains duplicate entry for object {0}")]
    DuplicateZOrderEntry(usize),
}

/// Serializable form of a [`Scene`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneData {
    /// Objects in arena (id) order.
    pub objects: Vec<ToolObjectData>,
    /// Paint order: object ids, bottom-most first.
    pub z_order: Vec<usize>,
}

/// The annotation scene graph: a dense object arena plus a z-ordered id list.
///
/// Invariants:
/// - `z_order` is always a permutation of `0..objects.len()`.
/// - Object ids are arena indices and shift on removal; treat ids as valid
///   only until the next [`Scene::remove_object`].
#[derive(Debug)]
pub struct Scene {
    objects: Vec<Box<dyn ToolObject>>,
    z_order: Vec<usize>,
}

impl Clone for Scene {
    fn clone(&self) -> Self {
        Self {
            objects: self
                .objects
                .iter()
                .map(|object| object.clone_box())
                .collect(),
            z_order: self.z_order.clone(),
        }
    }
}

impl PartialEq for Scene {
    fn eq(&self, other: &Self) -> bool {
        self.to_data() == other.to_data()
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    /// Creates an empty scene.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            objects: Vec::new(),
            z_order: Vec::new(),
        }
    }

    /// Number of objects in the arena.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Whether the scene holds no objects.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// All object ids (arena indices), in id order rather than paint order.
    #[must_use]
    pub fn ids(&self) -> std::ops::Range<usize> {
        0..self.objects.len()
    }

    /// The z-ordered id list: bottom-most first, top-most last.
    #[must_use]
    pub fn z_order(&self) -> &[usize] {
        &self.z_order
    }

    /// Direct access to the object arena slice (indexed by id).
    #[must_use]
    pub fn objects(&self) -> &[Box<dyn ToolObject>] {
        &self.objects
    }

    /// Borrows an object by id.
    #[must_use]
    pub fn get_object(&self, id: usize) -> Option<&dyn ToolObject> {
        let object = self.objects.get(id)?;
        Some(&**object)
    }

    /// Mutably borrows an object by id.
    ///
    /// Mutating a counter's count directly bypasses renumbering; prefer the
    /// dedicated scene operations for counters.
    pub fn get_object_mut(&mut self, id: usize) -> Option<&mut dyn ToolObject> {
        let object = self.objects.get_mut(id)?;
        Some(&mut **object)
    }

    /// Paint position of an id: `0` = bottom-most, `len - 1` = top-most.
    #[must_use]
    pub fn z_index(&self, id: usize) -> Option<usize> {
        self.z_order.iter().position(|&z| z == id)
    }

    /// Adds an object on top of the z-order and returns its id.
    ///
    /// Counters created with `count == 0` are auto-numbered with the max+1
    /// rule; explicit counts are preserved.
    pub fn add_object(&mut self, mut object: Box<dyn ToolObject>) -> usize {
        if object.count() == Some(0) {
            let next = self.next_counter_value();
            object.set_count(next);
        }
        self.push_object(object)
    }

    /// Re-adds a previously removed object on top of the z-order.
    ///
    /// Undo-restore rule: counters always receive a fresh max+1 number instead
    /// of their stale count, so restored counters never collide with the
    /// renumbered survivors.
    pub fn restore_object(&mut self, mut object: Box<dyn ToolObject>) -> usize {
        if object.count().is_some() {
            let next = self.next_counter_value();
            object.set_count(next);
        }
        self.push_object(object)
    }

    /// Removes an object by id, returning it.
    ///
    /// Ids above the removed one shift down by one and the z-order is remapped
    /// accordingly. If the removed object was a counter, every counter with a
    /// higher count is decremented by one so numbering stays contiguous.
    pub fn remove_object(&mut self, id: usize) -> Option<Box<dyn ToolObject>> {
        if id >= self.objects.len() {
            return None;
        }
        let removed = self.objects.remove(id);
        self.z_order.retain(|&z| z != id);
        for z in &mut self.z_order {
            if *z > id {
                *z -= 1;
            }
        }
        if let Some(removed_count) = removed.count() {
            self.decrement_counts_above(removed_count);
        }
        Some(removed)
    }

    /// The next counter number under the max+1 rule (at least `1`).
    #[must_use]
    pub fn next_counter_value(&self) -> u32 {
        self.objects
            .iter()
            .filter_map(|object| object.count())
            .max()
            .map_or(1, |max| max.saturating_add(1))
    }

    /// Counts of all counters in paint order (bottom-most first).
    #[must_use]
    pub fn counter_counts(&self) -> Vec<u32> {
        self.z_order
            .iter()
            .filter_map(|&id| self.objects.get(id))
            .filter_map(|object| object.count())
            .collect()
    }

    /// Moves an object one step toward the top of the paint order.
    ///
    /// Returns `false` when the id is invalid or already top-most.
    pub fn raise(&mut self, id: usize) -> bool {
        let Some(position) = self.z_index(id) else {
            return false;
        };
        let Some(above) = position.checked_add(1) else {
            return false;
        };
        if above >= self.z_order.len() {
            return false;
        }
        self.z_order.swap(position, above);
        true
    }

    /// Moves an object one step toward the bottom of the paint order.
    ///
    /// Returns `false` when the id is invalid or already bottom-most.
    pub fn lower(&mut self, id: usize) -> bool {
        let Some(position) = self.z_index(id) else {
            return false;
        };
        let Some(below) = position.checked_sub(1) else {
            return false;
        };
        self.z_order.swap(position, below);
        true
    }

    /// Moves an object to the top of the paint order.
    ///
    /// Returns `false` when the id is invalid.
    pub fn raise_to_top(&mut self, id: usize) -> bool {
        let Some(position) = self.z_index(id) else {
            return false;
        };
        let entry = self.z_order.remove(position);
        self.z_order.push(entry);
        true
    }

    /// Moves an object to the bottom of the paint order.
    ///
    /// Returns `false` when the id is invalid.
    pub fn lower_to_bottom(&mut self, id: usize) -> bool {
        let Some(position) = self.z_index(id) else {
            return false;
        };
        let entry = self.z_order.remove(position);
        self.z_order.insert(0, entry);
        true
    }

    /// Paints every object into `sink`, bottom-most first.
    pub fn paint(&self, sink: &mut dyn PaintSink) {
        for &id in &self.z_order {
            if let Some(object) = self.objects.get(id) {
                object.paint(sink);
            }
        }
    }

    /// Converts the scene into its serializable form.
    #[must_use]
    pub fn to_data(&self) -> SceneData {
        SceneData {
            objects: self.objects.iter().map(|object| object.to_data()).collect(),
            z_order: self.z_order.clone(),
        }
    }

    /// Rebuilds a scene from serializable data.
    ///
    /// # Errors
    ///
    /// Returns [`SceneError`] when `data.z_order` is not a permutation of
    /// `0..data.objects.len()`.
    pub fn from_data(data: SceneData) -> Result<Self, SceneError> {
        let count = data.objects.len();
        if data.z_order.len() != count {
            return Err(SceneError::ZOrderLengthMismatch {
                expected: count,
                actual: data.z_order.len(),
            });
        }
        let mut seen = vec![false; count];
        for &z in &data.z_order {
            if z >= count {
                return Err(SceneError::ZOrderIndexOutOfRange(z));
            }
            if seen[z] {
                return Err(SceneError::DuplicateZOrderEntry(z));
            }
            seen[z] = true;
        }
        Ok(Self {
            objects: data
                .objects
                .into_iter()
                .map(ToolObjectData::into_object)
                .collect(),
            z_order: data.z_order,
        })
    }

    fn push_object(&mut self, object: Box<dyn ToolObject>) -> usize {
        let id = self.objects.len();
        self.objects.push(object);
        self.z_order.push(id);
        id
    }

    fn decrement_counts_above(&mut self, threshold: u32) {
        if threshold == 0 {
            return;
        }
        for object in &mut self.objects {
            if let Some(count) = object.count()
                && count > threshold
            {
                object.set_count(count - 1);
            }
        }
    }
}

impl Serialize for Scene {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_data().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Scene {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = SceneData::deserialize(deserializer)?;
        Self::from_data(data).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// Undo stack
// ---------------------------------------------------------------------------

/// Snapshot-based undo history.
///
/// Every modification records a FULL `(before, after)` [`Scene`] pair. Undo
/// returns the `before` snapshot, redo returns the `after` snapshot; both are
/// silent no-ops (returning `None`) at the ends of the history. Pushing a new
/// modification after an undo discards the redo tail. The history is bounded
/// by a limit (config `undo_limit`, default [`DEFAULT_UNDO_LIMIT`]); when the
/// limit is exceeded the oldest snapshot is evicted.
#[derive(Debug, Clone)]
pub struct UndoStack<T = Scene> {
    snapshots: Vec<(T, T)>,
    cursor: usize,
    limit: usize,
}

impl<T: Clone> Default for UndoStack<T> {
    fn default() -> Self {
        Self::with_limit(DEFAULT_UNDO_LIMIT)
    }
}

impl<T: Clone> UndoStack<T> {
    /// Creates a stack with the [`DEFAULT_UNDO_LIMIT`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a stack bounded by `limit` snapshots.
    ///
    /// A limit of `0` disables history entirely (every push is evicted).
    #[must_use]
    pub const fn with_limit(limit: usize) -> Self {
        Self {
            snapshots: Vec::new(),
            cursor: 0,
            limit,
        }
    }

    /// Creates a stack bounded by the config `undo_limit` value
    /// ([`crate::config::EditorConfig::undo_limit`], default
    /// [`DEFAULT_UNDO_LIMIT`]).
    #[must_use]
    pub fn from_undo_limit(limit: u32) -> Self {
        Self::with_limit(usize::try_from(limit).unwrap_or(usize::MAX))
    }

    /// Records a modification as a full before/after snapshot pair.
    ///
    /// Discards any redo tail, then evicts oldest entries while over limit.
    pub fn push(&mut self, before: T, after: T) {
        self.snapshots.truncate(self.cursor);
        self.snapshots.push((before, after));
        while self.snapshots.len() > self.limit {
            self.snapshots.remove(0);
        }
        self.cursor = self.snapshots.len();
    }

    /// Steps one modification back and returns the restored (before) snapshot.
    ///
    /// Silent no-op returning `None` on an empty stack or at history start.
    #[must_use]
    pub fn undo(&mut self) -> Option<T> {
        let index = self.cursor.checked_sub(1)?;
        let snapshot = self.snapshots.get(index)?.0.clone();
        self.cursor = index;
        Some(snapshot)
    }

    /// Steps one modification forward and returns the reapplied (after) snapshot.
    ///
    /// Silent no-op returning `None` when already at the newest state.
    #[must_use]
    pub fn redo(&mut self) -> Option<T> {
        let snapshot = self.snapshots.get(self.cursor)?.1.clone();
        self.cursor = self.cursor.saturating_add(1);
        Some(snapshot)
    }

    /// Drops all history.
    pub fn clear(&mut self) {
        self.snapshots.clear();
        self.cursor = 0;
    }

    /// Whether [`UndoStack::undo`] would restore a state.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    /// Whether [`UndoStack::redo`] would reapply a state.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.cursor < self.snapshots.len()
    }

    /// Number of snapshots currently held (undo depth plus redo tail).
    #[must_use]
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether no history is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Number of undo steps available from the current position.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.cursor
    }

    /// The configured snapshot limit.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Updates the snapshot limit, evicting oldest entries if now over bound.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        while self.snapshots.len() > self.limit {
            self.snapshots.remove(0);
            self.cursor = self.cursor.saturating_sub(1);
        }
    }
}

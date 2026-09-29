//! The display list: the renderer's pure draw-command vocabulary.
//!
//! The renderer is a **pure draw-command consumer**: editor and widget
//! layers build a [`DisplayList`] per frame in physical pixels and hand it
//! to [`super::Renderer`]. No scene or
//! editor semantics live here, and every visual value (colors, radii, blur,
//! spacing) arrives from `flowshot_core::tokens` via the caller - the list
//! itself hardcodes nothing.

use flowshot_core::tokens::Shadow;

use super::color::Color;
use super::geom::{Point, Rect, Size};

/// Opaque handle of an uploaded image texture (frozen frame, magnifier
/// source, icon atlas...). Issued by the consumer; the renderer's texture
/// store keys on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(u64);

impl TextureId {
    /// Builds a texture handle from a consumer-managed integer.
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// The raw integer.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Vector geometry shared by fill and stroke commands.
///
/// Degenerate combinations (filling a [`Shape::Line`], stroking an empty
/// polyline) tessellate to empty geometry rather than erroring.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Axis-aligned rectangle, optionally with rounded corners
    /// (`radius = 0` for sharp corners, clamped to half the smaller side).
    Rect {
        /// The rectangle.
        rect: Rect,
        /// Corner radius in physical px.
        radius: f32,
    },
    /// Axis-aligned ellipse from center and radii.
    Ellipse {
        /// Center point.
        center: Point,
        /// Horizontal and vertical radii.
        radii: Size,
    },
    /// A single line segment (stroke-only in practice).
    Line {
        /// Start point.
        from: Point,
        /// End point.
        to: Point,
    },
    /// Connected segments; `closed` joins the last point back to the first.
    Polyline {
        /// Vertices in order.
        points: Vec<Point>,
        /// Whether the polyline closes into a polygon.
        closed: bool,
    },
    /// Arbitrary bezier path.
    Path {
        /// Path segments; the first should be [`PathSegment::MoveTo`].
        segments: Vec<PathSegment>,
    },
}

/// One segment of a [`Shape::Path`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathSegment {
    /// Starts a new subpath.
    MoveTo(Point),
    /// Straight line.
    LineTo(Point),
    /// Quadratic bezier (control point, end point).
    QuadTo(Point, Point),
    /// Cubic bezier (control 1, control 2, end point).
    CubeTo(Point, Point, Point),
    /// Closes the current subpath.
    Close,
}

/// Which point of the shaped text block [`TextCommand::position`] anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAnchor {
    /// The block's top-left (the layout-box origin; baselines derive from
    /// the layout runs).
    #[default]
    TopLeft,
    /// The block's exact center, computed from the shaping metrics at
    /// raster time (`PaintSink::draw_text_centered` parity - the counter
    /// digit).
    Center,
}

/// A text run: shaped by cosmic-text, rasterized through the glyph atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct TextCommand {
    /// The anchor point of the text box (see [`TextAnchor`]).
    pub position: Point,
    /// The text (may span lines; wrapping needs `max_width`).
    pub text: String,
    /// Font size in physical px (logical size x scale, per the physical-first
    /// rule).
    pub font_size: f32,
    /// Line height in physical px; must be positive (cosmic-text requirement).
    pub line_height: f32,
    /// Ink color (token-derived).
    pub color: Color,
    /// Preferred font family (token `typography.family`); `None` = the
    /// platform sans-serif default with fontconfig fallback.
    pub family: Option<String>,
    /// Wrap width in physical px; `None` = no wrapping.
    pub max_width: Option<f32>,
    /// Where `position` anchors the shaped block.
    pub anchor: TextAnchor,
    /// Bold weight for the whole run (the counter digit's
    /// `LabelStyle::bold`).
    pub bold: bool,
}

/// Drop-shadow parameters, physical px, token-derived via
/// [`ShadowSpec::from_token`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSpec {
    /// Gaussian blur radius in physical px (sigma = blur / 2).
    pub blur: f32,
    /// Shadow offset in physical px.
    pub offset: Point,
    /// Shadow color including alpha (`#RRGGBBAA` token).
    pub color: Color,
}

impl ShadowSpec {
    /// Scales a shadow token (logical px) to physical px at `scale`.
    ///
    /// # Errors
    ///
    /// `None` when the token's color hex is malformed.
    #[must_use]
    pub fn from_token(token: &Shadow, scale: f32) -> Option<Self> {
        let color = Color::from_shadow_token(token)?;
        let factor = scale.max(0.0);
        Some(Self {
            blur: super::geom::f32_from_u32(token.blur) * factor,
            offset: Point::new(token.offset[0] * factor, token.offset[1] * factor),
            color,
        })
    }
}

/// An image quad: a registered texture drawn into `dst`, optionally sampling
/// the pixel sub-region `src` (the magnifier's zoom window), at a
/// uniform `alpha` fade (the motion seam; 1.0 = opaque).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageCommand {
    /// Which uploaded texture to sample.
    pub texture: TextureId,
    /// Destination rect in physical px.
    pub dst: Rect,
    /// Source sub-region in texture pixels; `None` samples the whole texture.
    pub src: Option<Rect>,
    /// Uniform quad opacity in `[0, 1]` (clamped at draw time).
    pub alpha: f32,
}

/// A rounded-rect clip region for [`Command::PushClip`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipCommand {
    /// The clip rectangle.
    pub rect: Rect,
    /// Corner radius in physical px.
    pub radius: f32,
}

/// One draw command. Commands execute in list order; clip push/pop form a
/// stack that scopes every command in between.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Fills a shape (even-odd winding for self-overlapping paths).
    Fill {
        /// The geometry.
        shape: Shape,
        /// Fill color (token-derived).
        color: Color,
    },
    /// Strokes a shape outline; `width` is physical px.
    Stroke {
        /// The geometry.
        shape: Shape,
        /// Stroke width in physical px.
        width: f32,
        /// Stroke color (token-derived).
        color: Color,
    },
    /// The dim layer: `bounds` filled minus every cutout rect, using the
    /// even-odd rule (the contrastOpacity token via
    /// [`Color::dim_from_palette`]).
    Dim {
        /// The dimmed region (typically the whole output).
        bounds: Rect,
        /// Selection holes punched through the dim.
        cutouts: Vec<Rect>,
        /// Dim color with token opacity.
        color: Color,
    },
    /// Inverts everything painted below `rect` (the invert tool's
    /// non-destructive region filter). The complement runs in the renderer's
    /// linear-light compositing space (`1 - dst` per channel, destination
    /// alpha preserved), so channel extremes invert exactly while midtones
    /// follow the sRGB curve - the documented invert colorimetry.
    Invert {
        /// The inverted region.
        rect: Rect,
    },
    /// Draws an image quad. A missing texture renders the magenta
    /// placeholder and logs a tracing error (the renderer's failure path).
    Image(ImageCommand),
    /// Draws a drop shadow behind (below in list order) its content.
    Shadow {
        /// The occluder rectangle.
        rect: Rect,
        /// Its corner radius in physical px.
        radius: f32,
        /// Blur/offset/color (token-derived).
        spec: ShadowSpec,
    },
    /// Draws shaped text.
    Text(TextCommand),
    /// Pushes a rounded-rect clip onto the clip stack.
    PushClip(ClipCommand),
    /// Pops the most recent clip; a no-op when the stack is empty.
    PopClip,
}

/// An ordered list of draw commands - one frame of renderer input.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    commands: Vec<Command>,
}

impl DisplayList {
    /// An empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Number of commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Appends a raw command.
    pub fn push(&mut self, command: Command) {
        self.commands.push(command);
    }

    /// Drops all commands (buffer reuse across frames).
    pub fn clear(&mut self) {
        self.commands.clear();
    }

    /// Iterates the commands in draw order.
    pub fn iter(&self) -> std::slice::Iter<'_, Command> {
        self.commands.iter()
    }

    /// Fills a shape (even-odd rule for self-overlapping paths).
    pub fn fill(&mut self, shape: Shape, color: Color) {
        self.push(Command::Fill { shape, color });
    }

    /// Strokes a shape outline; `width` is physical px.
    pub fn stroke(&mut self, shape: Shape, width: f32, color: Color) {
        self.push(Command::Stroke {
            shape,
            width,
            color,
        });
    }

    /// Dims `bounds` with an even-odd selection cutout.
    pub fn dim(&mut self, bounds: Rect, cutouts: Vec<Rect>, color: Color) {
        self.push(Command::Dim {
            bounds,
            cutouts,
            color,
        });
    }

    /// Inverts everything painted below `rect` (the invert tool).
    pub fn invert(&mut self, rect: Rect) {
        self.push(Command::Invert { rect });
    }

    /// Draws an uploaded texture into `dst` (optionally a pixel sub-region).
    pub fn image(&mut self, texture: TextureId, dst: Rect, src: Option<Rect>) {
        self.image_faded(texture, dst, src, 1.0);
    }

    /// Draws an image quad at a uniform opacity (the motion fade seam;
    /// `alpha` clamps into `[0, 1]`).
    pub fn image_faded(&mut self, texture: TextureId, dst: Rect, src: Option<Rect>, alpha: f32) {
        self.push(Command::Image(ImageCommand {
            texture,
            dst,
            src,
            alpha: alpha.clamp(0.0, 1.0),
        }));
    }

    /// Draws a token-derived drop shadow for a rounded rect.
    pub fn shadow(&mut self, rect: Rect, radius: f32, spec: ShadowSpec) {
        self.push(Command::Shadow { rect, radius, spec });
    }

    /// Draws shaped text.
    pub fn text(&mut self, command: TextCommand) {
        self.push(Command::Text(command));
    }

    /// Pushes a rounded-rect clip.
    pub fn push_clip(&mut self, rect: Rect, radius: f32) {
        self.push(Command::PushClip(ClipCommand { rect, radius }));
    }

    /// Pops the most recent clip.
    pub fn pop_clip(&mut self) {
        self.push(Command::PopClip);
    }
}

impl<'a> IntoIterator for &'a DisplayList {
    type Item = &'a Command;
    type IntoIter = std::slice::Iter<'a, Command>;

    fn into_iter(self) -> Self::IntoIter {
        self.commands.iter()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn shadow_spec_scales_token_to_physical() {
        let token = Shadow {
            blur: 8,
            offset: [0.0, 2.0],
            color: "#00000029".to_owned(),
        };
        let spec = ShadowSpec::from_token(&token, 2.0).unwrap_or_else(|| panic!("token color"));
        assert_eq!(spec.blur, 16.0);
        assert_eq!(spec.offset, Point::new(0.0, 4.0));
        assert_eq!(spec.color.a, f32::from(0x29u8) / 255.0);
        assert!(ShadowSpec::from_token(&token, f32::NAN).is_some_and(|s| s.blur == 0.0));
    }

    #[test]
    fn shadow_spec_rejects_malformed_token() {
        let token = Shadow {
            blur: 4,
            offset: [0.0, 1.0],
            color: "#nope".to_owned(),
        };
        assert!(ShadowSpec::from_token(&token, 1.0).is_none());
    }

    #[test]
    fn display_list_builders_append_in_order() {
        let mut list = DisplayList::new();
        assert!(list.is_empty());
        let color = Color::from_rgba8(1, 2, 3, 4);
        list.fill(
            Shape::Rect {
                rect: Rect::from_parts(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
            },
            color,
        );
        list.stroke(
            Shape::Line {
                from: Point::new(0.0, 0.0),
                to: Point::new(5.0, 5.0),
            },
            1.0,
            color,
        );
        list.push_clip(Rect::from_parts(0.0, 0.0, 4.0, 4.0), 1.0);
        list.pop_clip();
        assert_eq!(list.len(), 4);
        assert!(matches!(list.iter().next(), Some(Command::Fill { .. })));
        assert!(matches!(list.iter().last(), Some(Command::PopClip)));
        list.clear();
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn texture_id_roundtrips() {
        assert_eq!(TextureId::new(7).raw(), 7);
        assert_ne!(TextureId::new(1), TextureId::new(2));
    }
}

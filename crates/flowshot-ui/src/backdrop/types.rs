//! The backdrop's public input vocabulary: what a frozen capture session
//! carries and how the scene is toggled. Types only - planning lives in
//! [`super::plan`], the facade in [`super::Backdrop`].

use flowshot_capture::Frame;
use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo, PhysicalPoint};

/// A platform-neutral cursor image: straight-alpha `RGBA8888` pixels plus
/// the hotspot offset (physical px) the platform's cursor session reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorSprite {
    /// Row-major straight-alpha `RGBA8888` pixels (`width * height * 4`).
    pub rgba: Vec<u8>,
    /// Sprite width in physical pixels.
    pub width: u32,
    /// Sprite height in physical pixels.
    pub height: u32,
    /// The cursor's active point relative to the sprite's top-left corner.
    pub hotspot: PhysicalPoint,
}

/// A cursor sprite frozen at a global logical position (todo 12's
/// `resolve_cursor_pos` output space).
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedCursor {
    /// The sprite image.
    pub sprite: CursorSprite,
    /// Where the hotspot sits, in global logical layout space.
    pub position: LogicalPoint,
}

/// One frozen capture session: the layout, one frame per output (native
/// pre-transform orientation, the shared [`Frame`] contract), and the
/// optional separately-composited cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenCapture {
    /// The captured outputs, in capture order.
    pub outputs: Vec<OutputInfo>,
    /// One frame per output; matched to outputs by connector name.
    pub frames: Vec<Frame>,
    /// The cursor sprite, when composited separately from the frames.
    pub cursor: Option<PlacedCursor>,
}

/// Why one output's frozen frame is unavailable (letterbox placeholder +
/// error state instead of a silent black frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingFrame {
    /// The capture produced no frame for this output.
    NoFrame,
    /// The frame's dimensions disagree with the output geometry.
    Mismatch {
        /// Frame buffer width in physical pixels.
        actual_width: u32,
        /// Frame buffer height in physical pixels.
        actual_height: u32,
        /// Expected native width in physical pixels.
        expected_width: u32,
        /// Expected native height in physical pixels.
        expected_height: u32,
    },
    /// The frame's pixel data failed conversion (short data, remap failure).
    Corrupt,
}

/// Runtime toggles for the backdrop scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackdropOptions {
    /// Whether the dim layer paints (the acceptance pixel-diff runs with the
    /// dim OFF; production defaults to on).
    pub dim: bool,
    /// Whether the cursor sprite composites (false when the frames already
    /// carry a backend-painted cursor).
    pub cursor_visible: bool,
    /// The INITIAL selection in global logical space, seeded into the
    /// selection engine at runtime construction (todo 16); from then on the
    /// live engine rect drives the dim cutout. Direct [`Backdrop::commands`]
    /// callers (tests, offscreen verification) consume it as the cutout.
    ///
    /// [`Backdrop::commands`]: super::Backdrop::commands
    pub selection: Option<LogicalRect>,
}

impl Default for BackdropOptions {
    fn default() -> Self {
        Self {
            dim: true,
            cursor_visible: true,
            selection: None,
        }
    }
}

//! The pixel-overlay layer: the WRITE side of the destructive region ops
//! (their committed form).
//!
//! # Mechanism (documented per the "operates on frame COPY" rule)
//!
//! Pixelate and blur are destructive pixel ops, but the pristine frozen
//! frame ([`FramePixels`], the read side installed via
//! [`OverlayCore::install_frame`](crate::OverlayCore::install_frame)) is
//! NEVER modified - the "never modify origScreenshot" rule and Flameshot's
//! `origScreenshot` retention for undo. Instead each committed op BAKES its
//! redacted output into an immutable [`PixelEffect`] buffer that the editor
//! paints as an image quad (`Command::Image`, above the backdrop, BELOW the
//! annotation scene - Flameshot bakes into the pixmap under all
//! annotations). Consequences:
//!
//! - EXPORT carries the redaction with zero reconstruction surface: the
//!   overlay holds only post-redaction bytes (the secure pixelate never even
//!   reads the region interior), and any renderer-path export composites the
//!   same quads. The CPU-sink export composites the effect layer
//!   through [`EditorState::pixel_effects`](super::EditorState::pixel_effects).
//! - UNDO restores the ORIGINAL pixels losslessly by construction: undo
//!   drops the effect ("undo = remove object") and the untouched
//!   pristine frame shows through - no pixel snapshot can be stale because
//!   nothing was overwritten. The unified (scene, effects) snapshot journal
//!   lives in [`super::undo`].
//! - RE-BAKING is idempotent-safe: effects sample the pristine frame, so a
//!   second effect overlapping a first redacts from the original pixels (the
//!   overlay order decides visibility; both outputs are redacted).
//!
//! Texture ids: effects register under [`effect_texture_id`] (base `1<<40`,
//! far above the backdrop's `1<<16 + output` and the cursor's `1<<15`); the
//! shell syncs per-window uploads from
//! [`EditorState::pixel_effects`](super::EditorState::pixel_effects).

use std::sync::Arc;

use flowshot_core::geometry::{Logical, LogicalRect, ToLogical, ToPhysical};

use crate::render::TextureId;

use super::pixelate::BakeRegion;
use super::tool::FramePixels;

/// Base of the pixel-effect texture ids (consumer-issued scheme of the
/// render list; the backdrop and cursor live far below).
const EFFECT_TEXTURE_BASE: u64 = 1 << 40;

/// Which destructive op baked an effect (the stable tracing token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectKind {
    /// The F27 secure fringe pseudo-pixelation.
    Pixelate,
    /// The two-pass gaussian blur variant.
    Blur,
}

impl EffectKind {
    /// The stable tracing/log token (QA asserts on these, never prose).
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Pixelate => "pixelate",
            Self::Blur => "blur",
        }
    }
}

/// One finished bake: the destination rect (global logical), the physical
/// region it was baked for, and the post-redaction pixels.
pub(super) struct Bake {
    /// The painted region, global logical px.
    pub rect: LogicalRect,
    /// The physical region inside the frame the bake sampled.
    pub region: BakeRegion,
    /// The baked post-redaction pixels (row-major RGBA).
    pub pixels: Vec<u8>,
}

/// One committed destructive region op: the baked post-redaction pixels and
/// where they paint. Immutable after the bake; snapshots share the buffer
/// through the [`Arc`] (undo history stays cheap).
#[derive(Debug, Clone, PartialEq)]
pub struct PixelEffect {
    id: u64,
    kind: EffectKind,
    rect: LogicalRect,
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl PixelEffect {
    /// Wraps a freshly finished bake (the editor assigns the identity via
    /// [`PixelEffect::with_id`] at commit).
    pub(super) fn new(kind: EffectKind, bake: Bake) -> Self {
        Self {
            id: 0,
            kind,
            rect: bake.rect,
            width: bake.region.w,
            height: bake.region.h,
            pixels: Arc::from(bake.pixels),
        }
    }

    /// Stamps the editor-assigned identity (drives [`PixelEffect::texture_id`]).
    pub(super) fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// The editor-assigned identity.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Which op baked this effect.
    #[must_use]
    pub const fn kind(&self) -> EffectKind {
        self.kind
    }

    /// The painted region, global logical px.
    #[must_use]
    pub const fn rect(&self) -> LogicalRect {
        self.rect
    }

    /// The baked buffer's width in physical px.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The baked buffer's height in physical px.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The baked post-redaction pixels (row-major RGBA, `width * height * 4`).
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The texture id this effect registers under (the shell's upload key).
    #[must_use]
    pub fn texture_id(&self) -> TextureId {
        effect_texture_id(self.id)
    }
}

/// The texture id of the effect with the given identity.
#[must_use]
pub fn effect_texture_id(id: u64) -> TextureId {
    TextureId::new(EFFECT_TEXTURE_BASE.saturating_add(id))
}

/// The frame's footprint in global logical space (the region-clamping bound
/// of the destructive tools).
impl FramePixels {
    /// The frame's footprint in global logical space.
    #[must_use]
    pub fn logical_rect(&self) -> LogicalRect {
        let size = flowshot_core::geometry::PhysicalSize::from_raw(
            i32::try_from(self.width).unwrap_or(i32::MAX),
            i32::try_from(self.height).unwrap_or(i32::MAX),
        )
        .to_logical(self.scale);
        LogicalRect::new(self.origin.x, self.origin.y, size.width, size.height)
    }
}

/// Maps a global-logical rect into the frame's physical px (EDGES converted
/// with the frame's own scale - the #4871 physical-first rule), clamped to
/// the frame. `None` when nothing of the rect lands inside the frame or the
/// clamped region collapses below 1px on either axis (the no-op rule).
pub(super) fn frame_region(frame: &FramePixels, rect: LogicalRect) -> Option<BakeRegion> {
    let width = i64::from(frame.width);
    let height = i64::from(frame.height);
    let x0 = edge_to_physical(rect.x.0 - frame.origin.x.0, frame.scale).clamp(0, width);
    let x1 =
        edge_to_physical(rect.x.0 + rect.width.0 - frame.origin.x.0, frame.scale).clamp(0, width);
    let y0 = edge_to_physical(rect.y.0 - frame.origin.y.0, frame.scale).clamp(0, height);
    let y1 =
        edge_to_physical(rect.y.0 + rect.height.0 - frame.origin.y.0, frame.scale).clamp(0, height);
    let (w, h) = (x1 - x0, y1 - y0);
    (w > 0 && h > 0).then(|| BakeRegion {
        x: u32::try_from(x0).unwrap_or(0),
        y: u32::try_from(y0).unwrap_or(0),
        w: u32::try_from(w).unwrap_or(0),
        h: u32::try_from(h).unwrap_or(0),
    })
}

/// One rect edge, frame-local logical -> physical (the core conversion is
/// total: invalid scales fall back to 1.0, rounding is half-away-from-zero).
fn edge_to_physical(offset: f64, scale: f64) -> i64 {
    i64::from(Logical(offset).to_physical(scale).0)
}

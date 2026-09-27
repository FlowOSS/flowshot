//! The frozen-frame backdrop: capture placement, stitching algebra, cursor
//! compositing, and the dim layer (plan todo 15).
//!
//! # Model
//!
//! A capture session freezes the screen: [`capture_frozen`] drives any
//! [`CaptureBackend`] through `outputs` + `capture_outputs(paint_cursor)`,
//! and [`Backdrop::plan`] places the per-output [`Frame`](flowshot_capture::Frame)s into the
//! [`OutputLayout`] algebra of `flowshot-core` - the stitch is LOGICAL
//! (one layout, per-output physical crops), never one resampled atlas, so
//! every monitor window renders its own frozen pixels 1:1 (physical-first
//! rule, #4871 fix) and mixed-DPI spans never blend an averaged scale.
//!
//! Per window, [`Backdrop::commands`] builds the display list in paint
//! order: letterbox placeholder (only for uncovered margins or a missing
//! frame - backend failure is a loud state, never a silent black frame),
//! the frozen frame crop, the cursor sprite (hotspot-applied, on the window
//! whose output owns the position), and the dim layer with the even-odd
//! selection cutout (`contrastOpacity` token via [`Color::dim_from_palette`]).
//!
//! # Cursor contract
//!
//! [`CursorSprite`] is the platform-neutral cursor image (the binary layer
//! converts the platform crate's captured cursor image into it - the purity
//! gate forbids importing the platform crate here). The sprite is only
//! composited when the frozen frames do NOT already carry a backend-painted
//! cursor (`paint_cursor = !hide_cursor`, #3582 fix); the caller decides via
//! [`FrozenCapture::cursor`] and [`BackdropOptions::cursor_visible`].
//!
//! # Texture ids
//!
//! Backdrop textures are consumer-issued handles: output `i` registers under
//! [`backdrop_texture_id(i)`], the cursor sprite under
//! [`cursor_texture_id()`]. The magnifier (todo 17) samples sub-regions of
//! the same ids via [`Backdrop::texture_size`].

mod pixels;
mod plan;
mod scene;
mod types;

use flowshot_capture::{CaptureBackend, CaptureError, CaptureOpts};
use flowshot_core::geometry::OutputLayout;
use flowshot_core::tokens::DesignTokens;

use crate::error::UiError;
use crate::gpu::GpuContext;
use crate::render::{
    Color, DisplayList, Rect, Renderer, RgbaImage, TextureId, f32_from_i32, f32_from_u32,
};

use plan::{Entry, EntryState, PlannedCursor, plan_cursor, plan_entry};
use scene::{WindowScene, local_physical_rect, window_crop};

pub use pixels::{PreparedTexture, prepare_output_texture};
pub use types::{BackdropOptions, CursorSprite, FrozenCapture, MissingFrame, PlacedCursor};

/// Base of the per-output frozen-frame texture ids (consumer-issued scheme).
const OUTPUT_TEXTURE_BASE: u64 = 1 << 16;
/// The cursor sprite's texture id.
const CURSOR_TEXTURE_RAW: u64 = 1 << 15;

/// The texture id output `output_index`'s frozen frame registers under.
#[must_use]
pub fn backdrop_texture_id(output_index: usize) -> TextureId {
    TextureId::new(
        OUTPUT_TEXTURE_BASE.saturating_add(u64::try_from(output_index).unwrap_or(u64::MAX)),
    )
}

/// The texture id the cursor sprite registers under.
#[must_use]
pub const fn cursor_texture_id() -> TextureId {
    TextureId::new(CURSOR_TEXTURE_RAW)
}

/// The planned frozen-frame backdrop spanning the whole output layout.
#[derive(Debug)]
pub struct Backdrop {
    layout: OutputLayout,
    entries: Vec<Entry>,
    cursor: PlannedCursor,
    dim_color: Option<Color>,
    placeholder: Color,
}

impl Backdrop {
    /// Plans the backdrop: pairs frames with outputs by connector, converts
    /// every frame to tight upright `RGBA8888` (CPU, before any GPU work),
    /// resolves the cursor placement, and derives the token colors.
    ///
    /// Outputs whose frame is missing or unusable become letterbox
    /// placeholder entries with a tracing error - one bad output never
    /// blanks the others.
    #[must_use]
    pub fn plan(capture: FrozenCapture, tokens: &DesignTokens) -> Self {
        let layout = OutputLayout::new(capture.outputs);
        let entries = layout
            .outputs
            .iter()
            .enumerate()
            .map(|(index, output)| plan_entry(index, output, &capture.frames))
            .collect();
        let cursor = plan_cursor(&layout, capture.cursor.as_ref());
        let dim_color = Color::dim_from_palette(&tokens.palette);
        if dim_color.is_none() {
            tracing::error!("contrast palette token malformed; dim layer disabled");
        }
        let placeholder = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or_else(|| Color::from_rgba8(255, 0, 255, 255));
        Self {
            layout,
            entries,
            cursor,
            dim_color,
            placeholder,
        }
    }

    /// The stitched layout every placement derives from.
    #[must_use]
    pub const fn layout(&self) -> &OutputLayout {
        &self.layout
    }

    /// Outputs whose frozen frame is unavailable, with the reason (the
    /// error-notification state for the shell/daemon layer).
    pub fn missing(&self) -> impl Iterator<Item = (&str, &MissingFrame)> {
        self.entries.iter().filter_map(|entry| match &entry.state {
            EntryState::Missing { connector, reason } => Some((connector.as_str(), reason)),
            EntryState::Ready { .. } => None,
        })
    }

    /// The upright texture size of output `output_index`'s frozen frame
    /// (the magnifier's sampling bounds, todo 17); `None` when missing.
    #[must_use]
    pub fn texture_size(&self, output_index: usize) -> Option<(u32, u32)> {
        match self.entries.get(output_index)?.state {
            EntryState::Ready { width, height, .. } => Some((width, height)),
            EntryState::Missing { .. } => None,
        }
    }

    /// Uploads output `output_index`'s frozen frame into `renderer`'s
    /// texture store, draining the prepared pixels (one output belongs to
    /// exactly one window; a second upload of the same output logs and
    /// skips).
    ///
    /// # Errors
    ///
    /// Texture validation failures per [`RgbaImage::validate`].
    pub fn upload_for(
        &mut self,
        output_index: usize,
        renderer: &mut Renderer,
        gpu: &GpuContext,
    ) -> Result<(), UiError> {
        let Some(entry) = self.entries.get_mut(output_index) else {
            return Ok(());
        };
        let EntryState::Ready {
            width,
            height,
            pixels,
        } = &mut entry.state
        else {
            return Ok(());
        };
        let Some(data) = pixels.take() else {
            tracing::warn!(output_index, "frozen frame already uploaded; skipping");
            return Ok(());
        };
        renderer.textures_mut().insert(
            &gpu.device,
            &gpu.queue,
            entry.texture,
            &RgbaImage {
                width: *width,
                height: *height,
                data: &data,
            },
        )
    }

    /// Uploads the cursor sprite into `renderer`'s texture store (idempotent;
    /// every window's renderer may hold it - only the owning window draws
    /// it).
    ///
    /// # Errors
    ///
    /// Texture validation failures per [`RgbaImage::validate`].
    pub fn upload_cursor(&self, renderer: &mut Renderer, gpu: &GpuContext) -> Result<(), UiError> {
        let (Some(pixels), Some((width, height))) = (self.cursor.pixels.as_ref(), self.cursor.size)
        else {
            return Ok(());
        };
        renderer.textures_mut().insert(
            &gpu.device,
            &gpu.queue,
            cursor_texture_id(),
            &RgbaImage {
                width,
                height,
                data: pixels,
            },
        )
    }

    /// Builds one window's backdrop display list (physical px): the window
    /// covers `layout.outputs[output_index]` with a surface of `surface`
    /// physical pixels.
    #[must_use]
    pub fn commands(
        &self,
        output_index: usize,
        surface: (u32, u32),
        options: &BackdropOptions,
    ) -> DisplayList {
        let mut list = DisplayList::new();
        let (texture, crop) = match self.entries.get(output_index).map(|entry| &entry.state) {
            Some(EntryState::Ready { width, height, .. }) => (
                backdrop_texture_id(output_index),
                Some(window_crop((*width, *height), surface)),
            ),
            _ => (backdrop_texture_id(output_index), None),
        };
        let cursor = self.cursor_rect(output_index, options);
        let dim = options.dim.then_some(self.dim_color).flatten();
        let cutout = options.selection.and_then(|selection| {
            let output = self.layout.outputs.get(output_index)?;
            local_physical_rect(output, selection)
        });
        let scene = WindowScene {
            surface,
            texture,
            crop,
            cursor,
            dim,
            cutout,
            placeholder: self.placeholder,
        };
        scene::push_window_scene(&mut list, &scene);
        list
    }

    fn cursor_rect(&self, output_index: usize, options: &BackdropOptions) -> Option<Rect> {
        if !options.cursor_visible || self.cursor.pixels.is_none() {
            return None;
        }
        let resolved = self
            .cursor
            .resolved
            .filter(|cursor| cursor.output_index == output_index)?;
        let (width, height) = self.cursor.size?;
        Some(Rect::from_parts(
            f32_from_i32(resolved.top_left.x.0),
            f32_from_i32(resolved.top_left.y.0),
            f32_from_u32(width),
            f32_from_u32(height),
        ))
    }
}

/// The capture orchestration entry (plan todo 15): enumerates the backend's
/// outputs and freezes them with `paint_cursor = !hide_cursor` (#3582 fix -
/// the config decides whether the cursor is baked into the frames).
///
/// The cursor sprite is NOT filled here: the platform cursor image arrives
/// out-of-band from the platform crate (todo 8 contract), so the binary
/// layer attaches it to [`FrozenCapture::cursor`] when the frames were
/// captured with `paint_cursor = false`.
///
/// # Errors
///
/// Enumeration or capture failures from the backend; the caller surfaces a
/// notification and exits typed - never a silent black frame.
pub async fn capture_frozen(
    backend: &dyn CaptureBackend,
    paint_cursor: bool,
) -> Result<FrozenCapture, CaptureError> {
    let outputs = backend.outputs().await?;
    let frames = backend
        .capture_outputs(CaptureOpts::new(paint_cursor))
        .await?;
    Ok(FrozenCapture {
        outputs,
        frames,
        cursor: None,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use bytes::BytesMut;
    use flowshot_capture::{BackendKind, Frame, FrameBuffer, FrameFormat, MockBackend, OutputRef};
    use flowshot_core::geometry::{
        Logical, LogicalPoint, LogicalRect, OutputInfo, PhysicalPoint, PhysicalSize, Transform,
    };

    use super::*;

    fn output(
        connector: &str,
        logical: LogicalRect,
        physical: (i32, i32),
        scale: f64,
        transform: Transform,
    ) -> OutputInfo {
        OutputInfo::new(
            connector,
            connector,
            logical,
            PhysicalSize::from_raw(physical.0, physical.1),
            scale,
            transform,
        )
        .expect("valid fixture output")
    }

    fn solid_frame(connector: &str, width: u32, height: u32, rgba: [u8; 4]) -> Frame {
        let pixels = usize::try_from(width * height).expect("fits");
        let mut data = Vec::with_capacity(pixels * 4);
        for _ in 0..pixels {
            data.extend_from_slice(&rgba);
        }
        Frame {
            buffer: FrameBuffer {
                data: BytesMut::from(data.as_slice()),
                width,
                height,
                stride: width * 4,
                format: FrameFormat::Rgba8888,
            },
            output: OutputRef::Connector(connector.to_owned()),
            scale: 1.0,
            transform: Transform::Normal,
        }
    }

    fn sprite(width: u32, height: u32, hotspot: (i32, i32)) -> CursorSprite {
        CursorSprite {
            rgba: vec![255u8; usize::try_from(width * height * 4).expect("fits")],
            width,
            height,
            hotspot: PhysicalPoint::from_raw(hotspot.0, hotspot.1),
        }
    }

    /// HDMI-A-1 8x6 @ 1x at (0,0) red; DP-3 8x6 @ 2x (logical 4x3) at
    /// (8,0) green - the mixed-scale stitch fixture.
    fn dual_capture() -> FrozenCapture {
        FrozenCapture {
            outputs: vec![
                output(
                    "HDMI-A-1",
                    LogicalRect::from_raw(0.0, 0.0, 8.0, 6.0),
                    (8, 6),
                    1.0,
                    Transform::Normal,
                ),
                output(
                    "DP-3",
                    LogicalRect::from_raw(8.0, 0.0, 4.0, 3.0),
                    (8, 6),
                    2.0,
                    Transform::Normal,
                ),
            ],
            frames: vec![
                solid_frame("HDMI-A-1", 8, 6, [255, 0, 0, 255]),
                solid_frame("DP-3", 8, 6, [0, 255, 0, 255]),
            ],
            cursor: None,
        }
    }

    #[test]
    fn plan_pairs_frames_with_outputs_and_prepares_textures() {
        let backdrop = Backdrop::plan(dual_capture(), &DesignTokens::default());
        assert_eq!(backdrop.layout().outputs.len(), 2);
        assert_eq!(backdrop.texture_size(0), Some((8, 6)));
        assert_eq!(backdrop.texture_size(1), Some((8, 6)));
        assert_eq!(backdrop.missing().count(), 0);
    }

    #[test]
    fn plan_letterboxes_outputs_without_frames() {
        let mut capture = dual_capture();
        capture.frames.pop();
        let backdrop = Backdrop::plan(capture, &DesignTokens::default());
        assert_eq!(backdrop.texture_size(0), Some((8, 6)));
        assert_eq!(backdrop.texture_size(1), None);
        let missing: Vec<_> = backdrop.missing().collect();
        assert_eq!(missing, vec![("DP-3", &MissingFrame::NoFrame)]);
        // The window with the missing frame gets a placeholder-only list.
        let list = backdrop.commands(1, (8, 6), &BackdropOptions::default());
        assert_eq!(list.len(), 2, "placeholder fill + dim, no image");
        assert!(matches!(
            list.iter().next(),
            Some(crate::render::Command::Fill { .. })
        ));
    }

    #[test]
    fn plan_records_dimension_mismatches() {
        let mut capture = dual_capture();
        capture.frames[1] = solid_frame("DP-3", 4, 3, [0, 255, 0, 255]);
        let backdrop = Backdrop::plan(capture, &DesignTokens::default());
        let missing: Vec<_> = backdrop.missing().collect();
        assert!(matches!(
            missing.as_slice(),
            [(
                _,
                MissingFrame::Mismatch {
                    actual_width: 4,
                    expected_width: 8,
                    ..
                }
            )]
        ));
    }

    #[test]
    fn commands_render_the_frozen_crop_one_to_one() {
        let backdrop = Backdrop::plan(dual_capture(), &DesignTokens::default());
        let options = BackdropOptions {
            dim: false,
            ..Default::default()
        };
        let list = backdrop.commands(0, (8, 6), &options);
        let [crate::render::Command::Image(image)] = list.iter().collect::<Vec<_>>().as_slice()
        else {
            panic!("single image command");
        };
        assert_eq!(image.texture, backdrop_texture_id(0));
        // 1:1: src extent == dst extent == surface (never rescaled).
        assert_eq!(image.dst.size.width, image.src.expect("crop").size.width);
        assert_eq!(image.dst, Rect::from_parts(0.0, 0.0, 8.0, 6.0));
    }

    #[test]
    fn commands_dim_cutout_uses_the_selection_and_token_color() {
        let backdrop = Backdrop::plan(dual_capture(), &DesignTokens::default());
        let tokens = DesignTokens::default();
        let options = BackdropOptions {
            selection: Some(LogicalRect::new(
                Logical(2.0),
                Logical(1.0),
                Logical(3.0),
                Logical(2.0),
            )),
            ..Default::default()
        };
        let list = backdrop.commands(0, (8, 6), &options);
        let dim = list
            .iter()
            .find_map(|command| match command {
                crate::render::Command::Dim {
                    bounds,
                    cutouts,
                    color,
                } => Some((bounds, cutouts, color)),
                _ => None,
            })
            .expect("dim command");
        assert_eq!(dim.0, &Rect::from_parts(0.0, 0.0, 8.0, 6.0));
        assert_eq!(dim.1, &[Rect::from_parts(2.0, 1.0, 3.0, 2.0)]);
        let expected = Color::dim_from_palette(&tokens.palette).expect("token color");
        assert_eq!(dim.2, &expected);
    }

    #[test]
    fn dim_off_emits_no_dim_command() {
        let backdrop = Backdrop::plan(dual_capture(), &DesignTokens::default());
        let options = BackdropOptions {
            dim: false,
            ..Default::default()
        };
        let list = backdrop.commands(0, (8, 6), &options);
        assert!(
            list.iter()
                .all(|command| !matches!(command, crate::render::Command::Dim { .. }))
        );
    }

    #[test]
    fn cursor_composites_only_on_the_owning_window_with_hotspot() {
        let mut capture = dual_capture();
        capture.cursor = Some(PlacedCursor {
            sprite: sprite(4, 4, (1, 1)),
            // Global logical (9, 1) is inside DP-3 (origin 8,0 scale 2):
            // local physical (2, 2) - hotspot (1, 1) -> top-left (1, 1).
            position: LogicalPoint::from_raw(9.0, 1.0),
        });
        let backdrop = Backdrop::plan(capture, &DesignTokens::default());
        let options = BackdropOptions {
            dim: false,
            ..Default::default()
        };
        let owning = backdrop.commands(1, (8, 6), &options);
        let images: Vec<_> = owning
            .iter()
            .filter_map(|command| match command {
                crate::render::Command::Image(image) => Some(image),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 2, "frame + cursor");
        assert_eq!(images[1].texture, cursor_texture_id());
        assert_eq!(images[1].dst, Rect::from_parts(1.0, 1.0, 4.0, 4.0));
        // The other window draws no cursor.
        let other = backdrop.commands(0, (8, 6), &options);
        assert_eq!(other.len(), 1);
        // cursor_visible=false suppresses it everywhere (painted-cursor case).
        let hidden = backdrop.commands(
            1,
            (8, 6),
            &BackdropOptions {
                dim: false,
                cursor_visible: false,
                ..Default::default()
            },
        );
        assert_eq!(hidden.len(), 1);
    }

    #[test]
    fn texture_ids_are_stable_and_disjoint() {
        assert_eq!(backdrop_texture_id(0), TextureId::new(OUTPUT_TEXTURE_BASE));
        assert_ne!(backdrop_texture_id(0), backdrop_texture_id(1));
        assert_ne!(cursor_texture_id(), backdrop_texture_id(0));
    }

    #[test]
    fn capture_frozen_pairs_backend_outputs_and_frames() {
        let backend = MockBackend::new(BackendKind::ExtImageCopyCapture);
        let capture = futures::executor::block_on(capture_frozen(&backend, true)).expect("capture");
        assert_eq!(capture.outputs.len(), 2);
        assert_eq!(capture.frames.len(), 2);
        assert!(capture.cursor.is_none());
        // The mock's frames match its outputs by connector - the plan
        // succeeds with nothing missing.
        let backdrop = Backdrop::plan(capture, &DesignTokens::default());
        assert_eq!(backdrop.missing().count(), 0);
    }
}

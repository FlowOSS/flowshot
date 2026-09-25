//! Backdrop planning: pairing captured frames with layout outputs and
//! preparing their pixels (CPU-side, before any GPU work).

use flowshot_capture::{Frame, OutputRef};
use flowshot_core::geometry::{OutputInfo, OutputLayout};

use crate::error::UiError;
use crate::render::TextureId;

use super::backdrop_texture_id;
use super::pixels::{PreparedTexture, prepare_cursor_texture, prepare_output_texture};
use super::scene::{ResolvedCursor, resolve_cursor};
use super::types::{MissingFrame, PlacedCursor};

/// One output's planned frozen frame.
#[derive(Debug)]
pub(super) struct Entry {
    pub texture: TextureId,
    pub state: EntryState,
}

#[derive(Debug)]
pub(super) enum EntryState {
    Ready {
        width: u32,
        height: u32,
        /// Drained by the first upload (one output belongs to one window).
        pixels: Option<Vec<u8>>,
    },
    Missing {
        connector: String,
        reason: MissingFrame,
    },
}

/// The planned cursor state: where the sprite lands, its drawn size, and
/// its premultiplied pixels (all `None` together when no usable sprite).
#[derive(Debug, Default)]
pub(super) struct PlannedCursor {
    pub resolved: Option<ResolvedCursor>,
    pub size: Option<(u32, u32)>,
    pub pixels: Option<Vec<u8>>,
}

/// Plans one output's entry: finds its frame by connector, converts it to
/// tight upright `RGBA8888`, and degrades to a letterbox placeholder (with
/// a tracing error) when the frame is missing or unusable - one bad output
/// never blanks the others.
pub(super) fn plan_entry(index: usize, output: &OutputInfo, frames: &[Frame]) -> Entry {
    let texture = backdrop_texture_id(index);
    let Some(frame) = frames
        .iter()
        .find(|frame| frame.output == OutputRef::Connector(output.connector.clone()))
    else {
        tracing::error!(connector = %output.connector, "capture produced no frame for output; letterbox placeholder");
        return Entry {
            texture,
            state: EntryState::Missing {
                connector: output.connector.clone(),
                reason: MissingFrame::NoFrame,
            },
        };
    };
    match prepare_output_texture(frame, output) {
        Ok(PreparedTexture {
            data,
            width,
            height,
        }) => Entry {
            texture,
            state: EntryState::Ready {
                width,
                height,
                pixels: Some(data),
            },
        },
        Err(error) => {
            tracing::error!(%error, connector = %output.connector, "frozen frame unusable; letterbox placeholder");
            Entry {
                texture,
                state: EntryState::Missing {
                    connector: output.connector.clone(),
                    reason: missing_reason(&error, frame),
                },
            }
        }
    }
}

fn missing_reason(error: &UiError, frame: &Frame) -> MissingFrame {
    match error {
        UiError::BackdropFrameMismatch {
            expected_width,
            expected_height,
            ..
        } => MissingFrame::Mismatch {
            actual_width: frame.buffer.width,
            actual_height: frame.buffer.height,
            expected_width: *expected_width,
            expected_height: *expected_height,
        },
        _ => MissingFrame::Corrupt,
    }
}

/// Resolves and premultiplies the cursor sprite; malformed pixels or a
/// position outside the layout degrade to no cursor (loudly), never a panic.
pub(super) fn plan_cursor(layout: &OutputLayout, placed: Option<&PlacedCursor>) -> PlannedCursor {
    let Some(placed) = placed else {
        return PlannedCursor::default();
    };
    let Some(pixels) = prepare_cursor_texture(&placed.sprite) else {
        tracing::error!("cursor sprite pixel data malformed; cursor compositing disabled");
        return PlannedCursor::default();
    };
    let resolved = resolve_cursor(layout, placed.position, placed.sprite.hotspot);
    if resolved.is_none() {
        tracing::warn!(
            x = placed.position.x.0,
            y = placed.position.y.0,
            "cursor position outside the layout; cursor compositing disabled"
        );
    }
    PlannedCursor {
        resolved,
        size: Some((placed.sprite.width, placed.sprite.height)),
        pixels: Some(pixels),
    }
}

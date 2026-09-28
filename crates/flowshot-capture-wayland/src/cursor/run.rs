//! One-shot cursor runners on dedicated short-lived connections, mirroring the
//! frame-capture execution model: connect, collect the session, create the
//! per-output pointer-cursor sessions, wait bounded, tear down. Closing the
//! connection releases every cursor session, source, and pointer compositor-side,
//! so a failed or cancelled query never leaks.
//!
//! # Degradation, never failure
//!
//! A cursor position is a convenience, never a capture requirement (draft
//! F13). Every failure collapses to `None` with a warning: no
//! compositor, no seat pointer, no capture manager, a `Hyprland`
//! `PERMISSION_TYPE_CURSOR_POS` denial (which leaves the session inert), or the
//! 500 ms wait expiring with the cursor off every output. [`cursor_pos`]
//! therefore returns an `Option`, not a `Result`.

use std::time::{Duration, Instant};

use flowshot_capture::FrameFormat;
use flowshot_core::geometry::{LogicalPoint, OutputInfo, PhysicalPoint};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, EventQueue};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_cursor_session_v1::ExtImageCopyCaptureCursorSessionV1;

use super::protocol::{CursorImage, CursorPositionSpace, resolve_cursor_pos};
use crate::error::{IccError, socket_connect_error};
use crate::icc::dispatch::IccGen;
use crate::icc::shm::ShmBuffer;
use crate::icc::wait::{CAPTURE_TIMEOUT, collect_deadline, dispatch_until};
use crate::session::CaptureState;
use crate::stitch::to_rgba;

/// Bound on waiting for the first cursor `position` event. The compositor
/// pushes the initial cursor state when a session is created, so a stationary
/// cursor answers immediately; the timeout only covers a withheld (denied) or
/// absent position.
const CURSOR_POS_TIMEOUT: Duration = Duration::from_millis(500);

/// A created pointer-cursor session plus the source it was built from, kept
/// alive for the query and destroyed on teardown.
pub(crate) struct CursorSessionHandles {
    session: ExtImageCopyCaptureCursorSessionV1,
    source: ExtImageCaptureSourceV1,
}

/// Queries the cursor position once, degrading to `None` on any failure.
pub(crate) fn cursor_pos() -> Option<LogicalPoint> {
    match cursor_pos_inner() {
        Ok(position) => position,
        Err(error) => {
            tracing::warn!(%error, "cursor position unavailable; degrading without it");
            None
        }
    }
}

fn cursor_pos_inner() -> Result<Option<LogicalPoint>, IccError> {
    let conn = Connection::connect_to_env().map_err(socket_connect_error)?;
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_deadline(
        &conn,
        &mut queue,
        &mut state,
        Instant::now() + CAPTURE_TIMEOUT,
    )?;

    let deadline = Instant::now() + CURSOR_POS_TIMEOUT;
    let handles = create_cursor_sessions(&mut queue, &mut state)?;
    queue.flush()?;
    if let Err(error) = dispatch_until(&mut queue, &mut state, deadline, |state| {
        state.cursor.any_position()
    }) {
        tracing::debug!(%error, "cursor-position wait ended without a position");
    }
    let resolved = resolve_cursor_pos(&state.cursor, &state.cursor_layout, state.cursor_space);
    teardown(&conn, handles, &mut state);
    Ok(resolved)
}

/// Captures the cursor image once through the embedded capture session of the
/// output the cursor currently overlaps, degrading to `None` on any failure.
pub(crate) fn cursor_image() -> Option<CursorImage> {
    match cursor_image_inner() {
        Ok(image) => image,
        Err(error) => {
            tracing::warn!(%error, "cursor image unavailable; degrading without it");
            None
        }
    }
}

fn cursor_image_inner() -> Result<Option<CursorImage>, IccError> {
    let conn = Connection::connect_to_env().map_err(socket_connect_error)?;
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_deadline(
        &conn,
        &mut queue,
        &mut state,
        Instant::now() + CAPTURE_TIMEOUT,
    )?;

    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let handles = create_cursor_sessions(&mut queue, &mut state)?;
    queue.flush()?;
    if let Err(error) = dispatch_until(&mut queue, &mut state, deadline, |state| {
        state.cursor.any_position()
    }) {
        tracing::debug!(%error, "cursor-image wait ended without a position");
        teardown(&conn, handles, &mut state);
        return Ok(None);
    }
    let Some((index, _local)) = state.cursor.first_position() else {
        teardown(&conn, handles, &mut state);
        return Ok(None);
    };
    let image = capture_embedded_image(&conn, &mut queue, &mut state, &handles, index, deadline);
    teardown(&conn, handles, &mut state);
    image
}

/// Runs the embedded capture-session frame chain for the cursor image of the
/// session at `index`.
fn capture_embedded_image(
    conn: &Connection,
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    handles: &[CursorSessionHandles],
    index: usize,
    deadline: Instant,
) -> Result<Option<CursorImage>, IccError> {
    let shm = state
        .shm
        .clone()
        .ok_or(IccError::MissingProtocol("wl_shm"))?;
    let Some(handle) = handles.get(index) else {
        return Ok(None);
    };
    let qh = queue.handle();
    let generation = IccGen(state.active.begin());
    let embedded = handle.session.get_capture_session(&qh, generation);

    dispatch_until(queue, state, deadline, |state| {
        state.active.constraints_settled()
    })?;
    let params = state.active.buffer_params()?;
    let buffer = ShmBuffer::allocate(&shm, &qh, &params)?;
    let frame = embedded.create_frame(&qh, generation);
    frame.attach_buffer(buffer.buffer());
    let (Ok(width), Ok(height)) = (i32::try_from(params.width), i32::try_from(params.height))
    else {
        return Err(IccError::Internal(
            "cursor image overflows the damage wire fields",
        ));
    };
    frame.damage_buffer(0, 0, width, height);
    frame.capture();

    dispatch_until(queue, state, deadline, |state| state.active.frame_settled())?;
    state.active.frame_outcome()?;
    let pixels = buffer.read_pixels()?;
    let hotspot = state
        .cursor
        .sessions
        .get(index)
        .and_then(|session| session.hotspot)
        .unwrap_or_else(PhysicalPoint::zero);

    frame.destroy();
    embedded.destroy();
    drop(buffer);
    if let Err(error) = conn.flush() {
        tracing::warn!(%error, "flushing the cursor-image teardown failed");
    }
    Ok(Some(CursorImage {
        rgba: to_rgba_pixels(&pixels, params.frame_format),
        width: params.width,
        height: params.height,
        hotspot,
    }))
}

/// Creates one pointer-cursor session per output, tagging each with its layout
/// index, and records the layout the position events convert against.
///
/// # Errors
///
/// [`IccError::MissingProtocol`] when a capture manager or the seat pointer is
/// absent, and [`IccError::NoOutputs`] when the session has no usable outputs.
pub(crate) fn create_cursor_sessions(
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
) -> Result<Vec<CursorSessionHandles>, IccError> {
    let source_manager = state
        .icc_source_manager
        .clone()
        .ok_or(IccError::MissingProtocol(
            "ext_output_image_capture_source_manager_v1",
        ))?;
    let copy_manager = state
        .icc_manager
        .clone()
        .ok_or_else(|| IccError::MissingProtocol("ext_image_copy_capture_manager_v1"))?;
    let qh = queue.handle();
    let pointer = state.ensure_pointer(&qh).ok_or(IccError::MissingProtocol(
        "wl_seat with the pointer capability",
    ))?;

    let outputs = ordered_outputs(state)?;
    state.cursor_layout = outputs.iter().map(|(_, info)| info.clone()).collect();
    state.cursor_space = CursorPositionSpace::for_desktop(state.desktop);
    state.cursor.begin(outputs.len());

    let mut handles = Vec::with_capacity(outputs.len());
    for (index, (output, _info)) in outputs.iter().enumerate() {
        let source = source_manager.create_source(output, &qh, ());
        let session = copy_manager.create_pointer_cursor_session(&source, &pointer, &qh, index);
        handles.push(CursorSessionHandles { session, source });
    }
    Ok(handles)
}

/// The session's usable outputs as (proxy, info), in registry-name order.
///
/// # Errors
///
/// [`IccError::NoOutputs`] when no output has valid reported geometry.
fn ordered_outputs(state: &CaptureState) -> Result<Vec<(WlOutput, OutputInfo)>, IccError> {
    let mut outputs = Vec::with_capacity(state.outputs.len());
    for tracked in state.outputs.values() {
        match tracked.data.to_output_info() {
            Ok(info) => outputs.push((tracked.proxy.clone(), info)),
            Err(error) => tracing::warn!(
                output = tracked.data.registry_name,
                %error,
                "skipping output with invalid reported geometry"
            ),
        }
    }
    if outputs.is_empty() {
        return Err(IccError::NoOutputs);
    }
    Ok(outputs)
}

/// Destroys the cursor sessions and sources, drops the pointer, and flushes;
/// the one-shot connection's close releases anything left.
fn teardown(conn: &Connection, handles: Vec<CursorSessionHandles>, state: &mut CaptureState) {
    for handle in handles {
        handle.session.destroy();
        handle.source.destroy();
    }
    state.pointer = None;
    if let Err(error) = conn.flush() {
        tracing::warn!(%error, "flushing the cursor-session teardown failed");
    }
}

/// Converts captured pixels of any v1 frame format into row-major `RGBA8888`.
fn to_rgba_pixels(pixels: &[u8], format: FrameFormat) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels.len());
    for chunk in pixels.as_chunks::<4>().0 {
        rgba.extend_from_slice(&to_rgba(format, *chunk));
    }
    rgba
}

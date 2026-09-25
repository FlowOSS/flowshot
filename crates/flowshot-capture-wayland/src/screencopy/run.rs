//! The one-shot `wlr-screencopy-unstable-v1` capture runner: a dedicated
//! connection per capture run, deadline-bounded dispatch, and the exact
//! request chain per output.
//!
//! # Chain (per output, one-shot)
//!
//! `zwlr_screencopy_manager_v1.capture_output(overlay_cursor, wl_output)` ->
//! await `buffer` + `buffer_done` -> allocate `wl_shm` pool + buffer (reported
//! format/dimensions/stride) -> `copy` -> await `flags`? + `ready` | `failed`
//! -> read pixels -> apply `y_invert` -> destroy frame.
//!
//! # Orientation rule
//!
//! `wlr-screencopy` copies the output's front buffer, so pixels arrive in the
//! output's NATIVE (pre-transform) orientation at the native physical
//! dimensions - verified against wlroots `types/wlr_screencopy_v1.c`
//! (full-output capture sizes the buffer at `output->width x output->height`
//! and reads `output->front_buffer`). That already matches the shared
//! [`Frame`] contract (native pixels + `transform` metadata), so unlike the ICC
//! backend there is NO inverse remap; the only correction is the renderer's
//! `y_invert` readback flag, applied in [`assemble_frame`].
//!
//! # Skew
//!
//! Multi-output runs capture sequentially on one connection: each output's
//! frame is a distinct compositor snapshot, so fast-moving content can skew
//! between outputs by the per-output capture latency (`grim`-equivalent,
//! documented not compensated).

use std::time::Instant;

use flowshot_capture::{CaptureOpts, Frame};
use flowshot_core::geometry::OutputInfo;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, EventQueue};

use super::dispatch::ScreencopyGen;
use super::protocol::assemble_frame;
use crate::error::{ScreencopyError, socket_connect_error};
use crate::icc::shm::ShmBuffer;
use crate::icc::wait::{CAPTURE_TIMEOUT, collect_deadline_with, dispatch_until_with};
use crate::session::CaptureState;
use crate::stitch::CapturedOutputs;

/// Which outputs a capture run covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Selection {
    /// Every enumerated output, in registry-name order.
    All,
    /// The first enumerated output (permission probe).
    First,
    /// The single output with this connector name.
    Named(String),
}

/// Runs one complete screencopy capture: connect, collect the session, capture
/// the selected outputs sequentially, and tear the connection down.
///
/// The connection is one-shot by design: even on error paths where the explicit
/// `destroy` is skipped, closing the connection makes the compositor release
/// every frame, pool, and buffer - so a failed or cancelled run can never leak
/// capture state (an immediate second run succeeds).
///
/// # Errors
///
/// Any [`ScreencopyError`] of the chain: connect/collection failures, missing
/// protocols, constraint or frame failures, timeouts, allocation errors, and
/// [`ScreencopyError::PermissionDenied`] when a delivered frame is classified
/// as the compositor's denial black frame.
pub(crate) fn capture_run(
    opts: CaptureOpts,
    selection: &Selection,
) -> Result<CapturedOutputs, ScreencopyError> {
    let conn = Connection::connect_to_env().map_err(socket_connect_error)?;
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_deadline_with::<ScreencopyError>(
        &conn,
        &mut queue,
        &mut state,
        Instant::now() + CAPTURE_TIMEOUT,
    )?;

    let targets = select_targets(&state, selection)?;
    let mut frames = Vec::with_capacity(targets.len());
    let mut outputs = Vec::with_capacity(targets.len());
    for (proxy, info) in targets {
        let deadline = Instant::now() + CAPTURE_TIMEOUT;
        let frame = capture_output(&conn, &mut queue, &mut state, &proxy, &info, opts, deadline)?;
        if crate::denial::is_denial_frame(&frame.buffer) {
            tracing::warn!(
                output = %info.connector,
                "compositor delivered a permission-denial black frame"
            );
            return Err(ScreencopyError::PermissionDenied);
        }
        frames.push(frame);
        outputs.push(info);
    }
    Ok(CapturedOutputs { outputs, frames })
}

/// Enumerates the session's outputs on a one-shot connection (no capture).
///
/// # Errors
///
/// [`ScreencopyError::Connect`]/[`ScreencopyError::Collection`] for connection
/// and collection failures and [`ScreencopyError::Timeout`] when a phase
/// exceeds the deadline.
pub(crate) fn outputs_run() -> Result<Vec<OutputInfo>, ScreencopyError> {
    let conn = Connection::connect_to_env().map_err(socket_connect_error)?;
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_deadline_with::<ScreencopyError>(
        &conn,
        &mut queue,
        &mut state,
        Instant::now() + CAPTURE_TIMEOUT,
    )?;
    Ok(state.snapshot().outputs)
}

/// Resolves the selection against the live tracked outputs, assembling each
/// target's [`OutputInfo`] (outputs with invalid reported geometry are skipped
/// with a warning, mirroring the session snapshot).
///
/// # Errors
///
/// [`ScreencopyError::NoOutputs`] when the session has no usable outputs and
/// [`ScreencopyError::OutputNotFound`] for a connector name the session does
/// not advertise.
fn select_targets(
    state: &CaptureState,
    selection: &Selection,
) -> Result<Vec<(WlOutput, OutputInfo)>, ScreencopyError> {
    let mut targets = Vec::with_capacity(state.outputs.len());
    for tracked in state.outputs.values() {
        match tracked.data.to_output_info() {
            Ok(info) => targets.push((tracked.proxy.clone(), info)),
            Err(error) => tracing::warn!(
                output = tracked.data.registry_name,
                %error,
                "skipping output with invalid reported geometry"
            ),
        }
    }
    if targets.is_empty() {
        return Err(ScreencopyError::NoOutputs);
    }
    match selection {
        Selection::All => Ok(targets),
        Selection::First => Ok(targets.into_iter().take(1).collect()),
        Selection::Named(name) => named_target(targets, name).map(|target| vec![target]),
    }
}

/// Picks the target with the given connector name.
///
/// # Errors
///
/// [`ScreencopyError::OutputNotFound`] listing every available connector name.
fn named_target<T>(
    targets: Vec<(T, OutputInfo)>,
    connector: &str,
) -> Result<(T, OutputInfo), ScreencopyError> {
    let available = targets
        .iter()
        .map(|(_, info)| info.connector.clone())
        .collect();
    targets
        .into_iter()
        .find(|(_, info)| info.connector == connector)
        .ok_or(ScreencopyError::OutputNotFound {
            requested: connector.to_owned(),
            available,
        })
}

/// Captures one output through the full one-shot screencopy chain.
///
/// # Errors
///
/// Any [`ScreencopyError`] of the chain for this output.
fn capture_output(
    conn: &Connection,
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    output: &WlOutput,
    info: &OutputInfo,
    opts: CaptureOpts,
    deadline: Instant,
) -> Result<Frame, ScreencopyError> {
    let Some(manager) = state.screencopy_manager.clone() else {
        return Err(ScreencopyError::MissingProtocol(
            "zwlr_screencopy_manager_v1",
        ));
    };
    let Some(shm) = state.shm.clone() else {
        return Err(ScreencopyError::MissingProtocol("wl_shm"));
    };

    let generation = ScreencopyGen(state.screencopy.begin());
    let qh = queue.handle();
    // overlay_cursor is the protocol's cursor-inclusion switch (1 = composite
    // the cursor onto the frame); driven by CaptureOpts.paint_cursor, which the
    // hide_cursor config inverts. There is no separate with/without request.
    let overlay_cursor = i32::from(opts.paint_cursor);
    let frame = manager.capture_output(overlay_cursor, output, &qh, generation);

    dispatch_until_with::<ScreencopyError>(queue, state, deadline, |state| {
        state.screencopy.constraints_settled()
    })?;
    let params = state.screencopy.buffer_params()?;
    tracing::debug!(
        output = %info.connector,
        width = params.width,
        height = params.height,
        stride = params.stride,
        format = ?params.frame_format,
        "screencopy frame buffer constraints received"
    );

    let shm_buffer = ShmBuffer::allocate_with::<ScreencopyError>(&shm, &qh, &params)?;
    frame.copy(shm_buffer.buffer());

    dispatch_until_with::<ScreencopyError>(queue, state, deadline, |state| {
        state.screencopy.frame_settled()
    })?;
    state.screencopy.frame_outcome()?;
    let y_invert = state.screencopy.y_invert;
    let pixels = shm_buffer.read_pixels_with::<ScreencopyError>()?;

    frame.destroy();
    drop(shm_buffer);
    if let Err(error) = conn.flush() {
        tracing::warn!(%error, "flushing the post-capture teardown requests failed");
    }
    if y_invert {
        tracing::debug!(
            output = %info.connector,
            "screencopy frame arrived y-inverted; flipping to native orientation"
        );
    }

    assemble_frame(&pixels, &params, info, y_invert)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    use super::*;

    fn output_info(connector: &str, transform: Transform, width: i32, height: i32) -> OutputInfo {
        OutputInfo::new(
            connector,
            "Test output",
            LogicalRect::new(Logical(0.0), Logical(0.0), Logical(8.0), Logical(6.0)),
            PhysicalSize::new(PhysicalPx(width), PhysicalPx(height)),
            1.0,
            transform,
        )
        .unwrap()
    }

    #[test]
    fn named_target_rejects_unknown_connectors_listing_available() {
        let targets = vec![
            ((), output_info("TEST-1", Transform::Normal, 2, 2)),
            ((), output_info("DP-3", Transform::Normal, 2, 2)),
        ];
        let ((), found) = named_target(targets.clone(), "DP-3").unwrap();
        assert_eq!(found.connector, "DP-3");
        match named_target(targets, "DP-99").unwrap_err() {
            ScreencopyError::OutputNotFound {
                requested,
                available,
            } => {
                assert_eq!(requested, "DP-99");
                assert_eq!(available, vec!["TEST-1".to_owned(), "DP-3".to_owned()]);
            }
            other => panic!("expected OutputNotFound, got {other:?}"),
        }
    }
}

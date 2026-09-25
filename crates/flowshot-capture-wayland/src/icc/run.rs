//! The one-shot capture runner: a dedicated connection per capture run,
//! deadline-bounded dispatch, and the exact `ext-image-copy-capture-v1`
//! request chain per output.
//!
//! # Chain (per output, one-shot)
//!
//! `ext_output_image_capture_source_manager_v1.create_source(wl_output)` ->
//! `ext_image_copy_capture_manager_v1.create_session(source, paint_cursors)`
//! -> await `shm_format`/`buffer_size`/`done` -> allocate `wl_shm` pool +
//! buffer -> `create_frame` -> `attach_buffer` -> `damage_buffer` (full) ->
//! `capture` -> await `ready` | `failed(reason)` -> read pixels -> destroy
//! frame + session + source.
//!
//! # Orientation rule
//!
//! `ext-image-copy-capture-v1` delivers buffers in POST-transform (upright,
//! layout) orientation: both reference compositor implementations size the
//! session buffer at the output's transformed resolution (wlroots
//! `wlr_ext_image_copy_capture_v1.c` sends the post-transform
//! `output->width/height`; `Hyprland` `ScreenshareSession` sends
//! `m_transformedSize`), and the frame's `transform` event reports what the
//! compositor APPLIED (`Hyprland` always sends `normal`). The shared
//! [`Frame`] contract, however, is native pre-transform buffers with the
//! transform as metadata - uniform with `wlr-screencopy` - so non-normal
//! outputs are inverse-remapped back to native orientation here (lossless
//! pixel permutation; the common `Normal` case skips it entirely).
//!
//! # Skew
//!
//! Multi-output runs capture sequentially on one connection: each output's
//! frame is a distinct compositor snapshot, so fast-moving content can skew
//! between outputs by the per-output capture latency. This is the
//! `grim`-equivalent behavior and is documented, not compensated.

use std::time::Instant;

use flowshot_capture::{CaptureOpts, Frame};
use flowshot_core::geometry::OutputInfo;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, EventQueue};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::Options;

use super::dispatch::IccGen;
use super::protocol::assemble_frame;
use super::shm::ShmBuffer;
use super::wait::{CAPTURE_TIMEOUT, collect_deadline, dispatch_until};
use crate::error::{IccError, socket_connect_error};
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

/// Runs one complete capture: connect, collect the session, capture the
/// selected outputs sequentially, and tear the connection down.
///
/// The connection is one-shot by design: even on error paths where explicit
/// `destroy` requests are skipped, closing the connection makes the
/// compositor release every session, frame, pool, and buffer - so a failed
/// or cancelled run can never leak capture sessions (acceptance: an
/// immediate second run succeeds).
///
/// # Errors
///
/// Any [`IccError`] of the chain: connect/collection failures, missing
/// protocols, constraint or frame failures, timeouts, allocation errors,
/// and [`IccError::PermissionDenied`] when a delivered frame is classified
/// as the compositor's denial black frame.
pub(crate) fn capture_run(
    opts: CaptureOpts,
    selection: &Selection,
) -> Result<CapturedOutputs, IccError> {
    let conn = Connection::connect_to_env().map_err(socket_connect_error)?;
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_deadline(
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
            return Err(IccError::PermissionDenied);
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
/// [`IccError::Connect`]/[`IccError::Collection`] for connection and
/// collection failures and [`IccError::Timeout`] when a phase exceeds the
/// deadline.
pub(crate) fn outputs_run() -> Result<Vec<OutputInfo>, IccError> {
    let conn = Connection::connect_to_env().map_err(socket_connect_error)?;
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_deadline(
        &conn,
        &mut queue,
        &mut state,
        Instant::now() + CAPTURE_TIMEOUT,
    )?;
    Ok(state.snapshot().outputs)
}

/// Resolves the selection against the live tracked outputs, assembling each
/// target's [`OutputInfo`] (outputs with invalid reported geometry are
/// skipped with a warning, mirroring the session snapshot).
///
/// # Errors
///
/// [`IccError::NoOutputs`] when the session has no usable outputs and
/// [`IccError::OutputNotFound`] for a connector name the session does not
/// advertise.
fn select_targets(
    state: &CaptureState,
    selection: &Selection,
) -> Result<Vec<(WlOutput, OutputInfo)>, IccError> {
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
        return Err(IccError::NoOutputs);
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
/// [`IccError::OutputNotFound`] listing every available connector name.
fn named_target<T>(
    targets: Vec<(T, OutputInfo)>,
    connector: &str,
) -> Result<(T, OutputInfo), IccError> {
    let available = targets
        .iter()
        .map(|(_, info)| info.connector.clone())
        .collect();
    targets
        .into_iter()
        .find(|(_, info)| info.connector == connector)
        .ok_or(IccError::OutputNotFound {
            requested: connector.to_owned(),
            available,
        })
}

/// Captures one output through the full one-shot chain.
///
/// # Errors
///
/// Any [`IccError`] of the chain for this output.
fn capture_output(
    conn: &Connection,
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    output: &WlOutput,
    info: &OutputInfo,
    opts: CaptureOpts,
    deadline: Instant,
) -> Result<Frame, IccError> {
    let Some(source_manager) = state.icc_source_manager.clone() else {
        return Err(IccError::MissingProtocol(
            "ext_output_image_capture_source_manager_v1",
        ));
    };
    let Some(copy_manager) = state.icc_manager.clone() else {
        return Err(IccError::MissingProtocol(
            "ext_image_copy_capture_manager_v1",
        ));
    };
    let Some(shm) = state.shm.clone() else {
        return Err(IccError::MissingProtocol("wl_shm"));
    };

    let generation = IccGen(state.active.begin());
    let qh = queue.handle();
    let source = source_manager.create_source(output, &qh, ());
    let options = if opts.paint_cursor {
        Options::PaintCursors
    } else {
        Options::empty()
    };
    let session = copy_manager.create_session(&source, options, &qh, generation);

    dispatch_until(queue, state, deadline, |state| {
        state.active.constraints_settled()
    })?;
    let params = state.active.buffer_params()?;
    tracing::debug!(
        output = %info.connector,
        width = params.width,
        height = params.height,
        format = ?params.frame_format,
        "capture session constraints received"
    );

    let shm_buffer = ShmBuffer::allocate(&shm, &qh, &params)?;
    let frame = session.create_frame(&qh, generation);
    frame.attach_buffer(shm_buffer.buffer());
    let (Ok(width), Ok(height)) = (i32::try_from(params.width), i32::try_from(params.height))
    else {
        return Err(IccError::Internal(
            "buffer dimensions overflow the damage wire fields",
        ));
    };
    frame.damage_buffer(0, 0, width, height);
    frame.capture();

    dispatch_until(queue, state, deadline, |state| state.active.frame_settled())?;
    state.active.frame_outcome()?;
    let wire_transform = state.active.frame_transform;
    let pixels = shm_buffer.read_pixels()?;

    frame.destroy();
    session.destroy();
    source.destroy();
    drop(shm_buffer);
    if let Err(error) = conn.flush() {
        tracing::warn!(%error, "flushing the post-capture teardown requests failed");
    }
    if wire_transform.is_some_and(|value| value != 0) {
        tracing::debug!(
            output = %info.connector,
            wire_transform,
            "compositor reported a non-normal applied transform"
        );
    }

    assemble_frame(&pixels, &params, info)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    use super::*;

    fn output_info(transform: Transform, width: i32, height: i32) -> OutputInfo {
        OutputInfo::new(
            "TEST-1",
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
        let second = OutputInfo::new(
            "DP-3",
            "Second",
            LogicalRect::new(Logical(8.0), Logical(0.0), Logical(8.0), Logical(6.0)),
            PhysicalSize::new(PhysicalPx(2), PhysicalPx(2)),
            1.0,
            Transform::Normal,
        )
        .unwrap();
        let targets = vec![((), output_info(Transform::Normal, 2, 2)), ((), second)];
        let ((), found) = named_target(targets.clone(), "DP-3").unwrap();
        assert_eq!(found.connector, "DP-3");
        match named_target(targets, "DP-99").unwrap_err() {
            IccError::OutputNotFound {
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

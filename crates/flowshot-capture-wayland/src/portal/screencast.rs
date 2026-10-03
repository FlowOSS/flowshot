//! The `org.freedesktop.portal.ScreenCast` single-frame capture chain.
//!
//! # Chain (one-shot, no restore token)
//!
//! `CreateSession` -> `SelectSources(Monitor, cursor_mode from
//! paint_cursor, multiple)` -> `Start` -> `OpenPipeWireRemote` ->
//! `pipewire-rs` core via the portal fd -> one capture stream per portal
//! stream node -> FIRST frame per stream ([`pipewire`]) -> full teardown
//! (close the session, destroy the streams, disconnect the core). One-shot
//! sessions need no `persist_mode`/restore token. Frame normalization
//! lives in [`assemble`]; the trait implementation in [`backend`].
//!
//! # One session per output (implementation reality)
//!
//! The spec allows one session to yield every monitor's stream (GNOME's
//! picker multi-selects), but `XDPH` shares exactly ONE source per session
//! (its picker selects a single output, reported in the stream's
//! `mapping_id`). The runner therefore tops up: after each session, any
//! output still missing a frame gets another session, bounded by one
//! session per output. A session that captures nothing new ends the loop
//! (a picker re-selecting captured outputs must not spin).
//!
//! # Permissions
//!
//! Portals decide per session inside `SelectSources`/`Start` (the picker);
//! response status 1/2 maps to [`PortalErrorKind::Denied`].
//!
//! [`pipewire`]: super::pipewire
//! [`PortalErrorKind::Denied`]: super::error::PortalErrorKind::Denied

mod assemble;
mod backend;

use std::os::fd::OwnedFd;

use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, Request, Session};
use ashpd::enumflags2::BitFlags;
use assemble::assemble_frame;
use flowshot_capture::CaptureOpts;

use super::error::{PortalErrorKind, PortalScreenCastError};
use super::pipewire::{RawPwFrame, capture_first_frames};
use super::run::{
    HANDSHAKE_TIMEOUT, PORTAL_TIMEOUT, Selection, build_runtime, classify, collect_outputs,
    select_outputs, with_deadline,
};
use super::streams::{StreamMeta, match_output};
use crate::stitch::CapturedOutputs;

/// The result of one portal `ScreenCast` handshake.
struct Handshake {
    fd: OwnedFd,
    metas: Vec<StreamMeta>,
    session: Session<Screencast>,
}

/// Runs one complete portal screencast capture on the worker thread.
///
/// # Errors
///
/// Any [`PortalScreenCastError`] of the chain: connect/collection failures,
/// portal denials, handshake or first-frame timeouts, `PipeWire` failures,
/// stream-mapping failures, and [`PortalErrorKind::OutputsUncaptured`] when
/// the session budget ran out with outputs missing frames.
pub(crate) fn screencast_run(
    opts: CaptureOpts,
    selection: &Selection,
) -> Result<CapturedOutputs, PortalScreenCastError> {
    let outputs = select_outputs(collect_outputs::<PortalScreenCastError>()?, selection)?;
    let runtime = build_runtime::<PortalScreenCastError>()?;

    let mut captured: Vec<Option<RawPwFrame>> = vec![None; outputs.len()];
    let mut claimed = vec![false; outputs.len()];
    // One session per output is the worst case (XDPH shares a single
    // source per session); the bound keeps a misbehaving picker finite.
    for _round in 0..outputs.len() {
        let missing = claimed.iter().filter(|claimed| !**claimed).count();
        if missing == 0 {
            break;
        }
        let handshake = runtime.block_on(with_deadline(
            HANDSHAKE_TIMEOUT,
            handshake(opts.paint_cursor, missing > 1),
        ))?;
        let nodes: Vec<u32> = handshake.metas.iter().map(|meta| meta.node_id).collect();
        let frames = capture_first_frames(handshake.fd, &nodes, PORTAL_TIMEOUT);
        let metas = handshake.metas;
        runtime.block_on(close_session(handshake.session));
        let frames = frames?;

        let before = claimed.iter().filter(|claimed| **claimed).count();
        let mut unidentified = Vec::new();
        for frame in frames {
            let Some(meta) = metas.iter().find(|meta| meta.node_id == frame.node_id) else {
                tracing::warn!(
                    node_id = frame.node_id,
                    "PipeWire frame arrived for a node the portal never announced"
                );
                continue;
            };
            match match_output(meta, (frame.width, frame.height), &outputs, &claimed) {
                Some(index) => {
                    claimed[index] = true;
                    captured[index] = Some(frame);
                }
                // A stream WITH identifying metadata that matches nothing is
                // a hard miss (pairing it would silently capture the wrong
                // monitor); an anonymous stream may still pair with the
                // lone remaining output below.
                None if meta.mapping_id.is_some() || meta.position.is_some() => {
                    return Err(PortalErrorKind::StreamUnmapped {
                        node_id: meta.node_id,
                        frame: (frame.width, frame.height),
                    }
                    .into());
                }
                None => unidentified.push(frame),
            }
        }
        pair_lone_stream(&mut unidentified, &mut claimed, &mut captured);
        for frame in &unidentified {
            tracing::warn!(
                node_id = frame.node_id,
                width = frame.width,
                height = frame.height,
                "screencast stream matches no output; discarding"
            );
        }
        let progress = claimed.iter().filter(|claimed| **claimed).count() > before;
        if !progress {
            tracing::warn!("portal screencast session captured no new output; stopping");
            break;
        }
    }

    let missing: Vec<String> = outputs
        .iter()
        .zip(&claimed)
        .filter(|(_, claimed)| !**claimed)
        .map(|(output, _)| output.connector.clone())
        .collect();
    if !missing.is_empty() {
        return Err(PortalErrorKind::OutputsUncaptured { missing }.into());
    }

    let mut frames = Vec::with_capacity(outputs.len());
    for (output, frame) in outputs.iter().zip(captured) {
        let Some(frame) = frame else {
            return Err(PortalErrorKind::Internal("a claimed output has no captured frame").into());
        };
        frames.push(assemble_frame(output, frame)?);
    }
    Ok(CapturedOutputs { outputs, frames })
}

/// The anonymous-stream last resort: exactly one unidentified frame pairs
/// with exactly one still-free output (a lone stream from a metadata-less
/// implementation). Anything else stays unidentified and is discarded.
fn pair_lone_stream(
    unidentified: &mut Vec<RawPwFrame>,
    claimed: &mut [bool],
    captured: &mut [Option<RawPwFrame>],
) {
    if unidentified.len() != 1 {
        return;
    }
    let free: Vec<usize> = claimed
        .iter()
        .enumerate()
        .filter(|(_, claimed)| !**claimed)
        .map(|(index, _)| index)
        .collect();
    if free.len() != 1 {
        return;
    }
    let (Some(index), Some(frame)) = (free.first().copied(), unidentified.pop()) else {
        return;
    };
    claimed[index] = true;
    captured[index] = Some(frame);
}

/// The portal handshake: session, source selection, start, and the
/// `PipeWire` remote fd.
async fn handshake(paint_cursor: bool, multiple: bool) -> Result<Handshake, PortalScreenCastError> {
    let proxy: Screencast = classify(Screencast::new().await)?;
    let session = classify(proxy.create_session(CreateSessionOptions::default()).await)?;
    // hideCursor config drives paint_cursor: Embedded composites the cursor
    // into the stream, Hidden excludes it (Metadata is a cursor-STREAM
    // channel v1 does not consume; XDPH falls back for it anyway).
    let cursor_mode = if paint_cursor {
        CursorMode::Embedded
    } else {
        CursorMode::Hidden
    };
    let select: Request<()> = classify(
        proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(cursor_mode)
                    .set_sources(BitFlags::from(SourceType::Monitor))
                    .set_multiple(multiple)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await,
    )?;
    classify(select.response())?;
    let start = classify(
        proxy
            .start(&session, None, StartCastOptions::default())
            .await,
    )?;
    let streams = classify(start.response())?;
    if streams.streams().is_empty() {
        return Err(PortalErrorKind::NoStreams.into());
    }
    let metas = streams.streams().iter().map(stream_meta).collect();
    let fd = classify(
        proxy
            .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
            .await,
    )?;
    Ok(Handshake { fd, metas, session })
}

fn stream_meta(stream: &ashpd::desktop::screencast::Stream) -> StreamMeta {
    let meta = StreamMeta {
        node_id: stream.pipe_wire_node_id(),
        mapping_id: stream.mapping_id().map(str::to_owned),
        position: stream.position(),
        size: stream.size(),
    };
    tracing::debug!(?meta, "portal screencast stream announced");
    meta
}

/// Best-effort session teardown: the portal releases the share when the
/// session closes, and the caller's `PipeWire` nodes are already gone
/// (core disconnect precedes this call).
async fn close_session(session: Session<Screencast>) {
    if let Err(error) = session.close().await {
        tracing::warn!(%error, "closing the portal screencast session failed");
    }
}

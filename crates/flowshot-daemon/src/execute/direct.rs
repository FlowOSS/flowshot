//! The non-interactive capture path: full
//! desktop, single output, delayed, and stdout modes - no overlay window,
//! so every step here is invisible-protocol QA-able on a live session.
//!
//! Exports are PHYSICAL-FIRST (#4871): each output contributes its native
//! post-transform pixels through [`prepare_output_texture`], placed by
//! [`composite_selection`] at its own scale - never resampled to logical
//! resolution, never an averaged factor.

use std::time::Instant;

use flowshot_capture::CaptureOpts;
use flowshot_core::geometry::{LogicalRect, OutputInfo, OutputLayout};
use flowshot_ui::completion::RenderedOutput;
use flowshot_ui::{Completion, CompletionKind, composite_selection, prepare_output_texture};

use super::backend::{open_session_excluding, resolve_cursor};
use super::post;
use super::{ExecCtx, ExecOutcome, ExecuteError, elapsed};
use crate::request::CaptureRequest;

/// What a direct (window-less) capture targets.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// The full desktop (every output, physical-first composite).
    Full,
    /// One output.
    Screen(ScreenTarget),
    /// An explicit rect in global logical pixels (the launcher dialog's
    /// manual-coordinate dispatch; clamped to the layout by the composite).
    Region(LogicalRect),
}

/// Which output a [`Target::Screen`] names.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenTarget {
    /// The output under the cursor (the cursor-resolution contract).
    Cursor,
    /// Probe-order index (the registry indexing authority).
    Index(u32),
    /// Connector name (e.g. `DP-1`).
    Connector(String),
}

/// Runs one direct capture to completion.
///
/// # Errors
///
/// [`ExecuteError`] per stage (usage, capture, export).
pub async fn run(
    target: Target,
    request: CaptureRequest,
    ctx: &ExecCtx,
    started: Instant,
) -> Result<ExecOutcome, ExecuteError> {
    post::validate_stdout_modes(&request)?;
    let (config, config_path) = ctx.load_config();
    if request.delay_ms > 0 {
        tracing::info!(delay_ms = request.delay_ms, "capture delay armed");
        tokio::time::sleep(std::time::Duration::from_millis(u64::from(
            request.delay_ms,
        )))
        .await;
        tracing::info!(
            target: "flowshot_perf",
            elapsed_us = elapsed(started),
            "perf.delay_done"
        );
    }
    let hide_cursor = request.hide_cursor || config.capture.hide_cursor;
    let completion = capture_and_composite(&target, hide_cursor, started).await?;
    post::run_post(completion, &request, &config, config_path.as_deref(), ctx).await
}

/// The capture leg alone (no post-capture): ladder -> frames -> upright
/// preparation -> physical-first composite. Shared by [`run`] and the
/// launcher child's one-shot dispatch.
///
/// # Errors
///
/// [`ExecuteError`] for usage/capture/composite failures.
pub async fn capture_and_composite(
    target: &Target,
    hide_cursor: bool,
    started: Instant,
) -> Result<Completion, ExecuteError> {
    // Runtime ladder fallthrough: a rung that probes green can still fail
    // AT CAPTURE (the rotated-headless BufferSizeMismatch class) -
    // the ladder's promise is the next rung, exhausted rungs surface the
    // typed NoBackendAvailable.
    let mut excluded: Vec<flowshot_capture::BackendKind> = Vec::new();
    loop {
        let session = open_session_excluding(&excluded).await?;
        let layout = OutputLayout::new(session.outputs.clone());
        // Usage-class resolution errors abort (never a fallback trigger).
        let selection = resolve_target(target, &layout).await?;
        match capture_once(&session, &layout, selection, hide_cursor, started).await {
            Ok(completion) => return Ok(completion),
            Err(error) => {
                tracing::warn!(
                    backend = ?session.kind,
                    %error,
                    "capture failed on this rung; falling through the ladder"
                );
                excluded.push(session.kind);
            }
        }
    }
}

async fn capture_once(
    session: &super::backend::CaptureSession,
    layout: &OutputLayout,
    selection: LogicalRect,
    hide_cursor: bool,
    started: Instant,
) -> Result<Completion, flowshot_capture::CaptureError> {
    let frames = session
        .backend
        .capture_outputs(CaptureOpts::new(!hide_cursor))
        .await?;
    tracing::info!(
        target: "flowshot_perf",
        elapsed_us = elapsed(started),
        frames = frames.len(),
        "perf.capture_ready"
    );
    let mut renders = Vec::with_capacity(frames.len());
    for (index, output) in layout.outputs.iter().enumerate() {
        let frame = frames
            .iter()
            .find(|frame| {
                frame.output == flowshot_capture::OutputRef::Connector(output.connector.clone())
            })
            .ok_or_else(|| {
                detail_error(
                    session.kind,
                    format!("capture produced no frame for {}", output.connector),
                )
            })?;
        let prepared = prepare_output_texture(frame, output).map_err(|error| {
            detail_error(
                session.kind,
                format!("frame preparation failed for {}: {error}", output.connector),
            )
        })?;
        renders.push(RenderedOutput {
            output_index: index,
            width: prepared.width,
            height: prepared.height,
            rgba: prepared.data,
        });
    }
    let image = composite_selection(layout, selection, &renders)
        .ok_or(flowshot_capture::CaptureError::RegionOutsideLayout { region: selection })?;
    tracing::info!(
        target: "flowshot_perf",
        elapsed_us = elapsed(started),
        width = image.width,
        height = image.height,
        "perf.frame_ready"
    );
    Ok(Completion {
        kind: CompletionKind::Accept,
        selection,
        image,
    })
}

/// A backend failure carrying a plain-text detail (the frame-missing and
/// preparation classes have no underlying protocol error to attach).
fn detail_error(
    kind: flowshot_capture::BackendKind,
    detail: String,
) -> flowshot_capture::CaptureError {
    flowshot_capture::CaptureError::Backend {
        backend: kind,
        source: Box::new(Detail { detail }),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
struct Detail {
    detail: String,
}

/// Resolves the capture rect (global logical space) for the target.
async fn resolve_target(
    target: &Target,
    layout: &OutputLayout,
) -> Result<LogicalRect, ExecuteError> {
    match target {
        Target::Full => layout
            .union_bounds()
            .ok_or_else(|| ExecuteError::Usage("the session reports no outputs".to_owned())),
        Target::Screen(screen) => {
            let output = resolve_screen(screen, layout).await?;
            Ok(output.logical_rect)
        }
        Target::Region(region) => Ok(*region),
    }
}

async fn resolve_screen<'a>(
    screen: &ScreenTarget,
    layout: &'a OutputLayout,
) -> Result<&'a OutputInfo, ExecuteError> {
    match screen {
        ScreenTarget::Index(index) => {
            let index = usize::try_from(*index)
                .map_err(|_| ExecuteError::Usage(format!("screen index {index} out of range")))?;
            layout.outputs.get(index).ok_or_else(|| {
                ExecuteError::Usage(format!(
                    "screen index {index} outside the {}-output probe order",
                    layout.outputs.len()
                ))
            })
        }
        ScreenTarget::Connector(name) => layout
            .outputs
            .iter()
            .find(|output| output.connector == *name)
            .ok_or_else(|| ExecuteError::Usage(format!("no output with connector name {name:?}"))),
        ScreenTarget::Cursor => {
            let cursor = resolve_cursor().await;
            if let Some(output) = flowshot_ui::output_at_cursor(layout, cursor) {
                return Ok(output);
            }
            // Fallback policy (open question, decided here): the
            // cursor ladder resolved nothing usable - capture the FIRST
            // output with a loud warning instead of failing (deterministic,
            // and the ladder only misses on compositors without cursor
            // position protocols).
            tracing::warn!("cursor position unresolved; falling back to the first output");
            layout
                .outputs
                .first()
                .ok_or_else(|| ExecuteError::Usage("the session reports no outputs".to_owned()))
        }
    }
}

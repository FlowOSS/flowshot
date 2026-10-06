//! The interactive overlay session: frozen capture ->
//! `OverlayRuntime` with the full production wiring -> the post-capture
//! pipeline.
//!
//! Process topology (the winit one-event-loop-per-process constraint, see
//! [`super::session`]): the PARENT (daemon or one-shot CLI) orchestrates
//! (config, delay, heartbeat, post-capture - so the clipboard offer stays
//! daemon-owned); the CHILD owns the winit loop and the export render and
//! hands the image back as a temp PNG.
//!
//! Splits: `wiring` holds `configure_core` (the SINGLE wiring function
//! shared with the headless execution mode) and the launch mapping;
//! `session` holds the child's blocking winit leg and the
//! frame/geometry helpers; `reroute` holds the X11 headless reroute
//! decision.

mod reroute;
mod session;
mod wiring;

use std::path::PathBuf;
use std::time::Instant;

use flowshot_core::geometry::OutputLayout;
use flowshot_ui::launcher::RegionGeometry;

use super::backend::{open_session_excluding, resolve_cursor};
use super::session::{self as child, SessionKind, SessionResult, SessionSpec};
use super::{ExecCtx, ExecOutcome, ExecuteError, elapsed};
use crate::request::CaptureRequest;

pub use self::session::{
    OverlaySession, SessionOutcome, region_rect_of, run_overlay_session, stitched_editor_frame,
};
pub use self::wiring::{CoreSinks, SessionEvents, build_launch_request, configure_core};

use self::reroute::x11_headless_region;

/// The parent-side orchestration of one interactive capture (or
/// `flowshot color` with `color_mode`).
///
/// X11 headless exception: `--no-edit` invocations whose target resolves
/// without a window (a typed `--region` token, a persisted last region,
/// `--region at-cursor`) reroute onto the direct leg
/// ([`x11_headless_region`]) instead of spawning the overlay child.
///
/// # Errors
///
/// [`ExecuteError`] per stage; user cancellation is an outcome.
pub async fn run_interactive(
    request: CaptureRequest,
    ctx: &ExecCtx,
    started: Instant,
    color_mode: bool,
) -> Result<ExecOutcome, ExecuteError> {
    super::post::validate_stdout_modes(&request)?;
    if let Some(target) = x11_headless_region(&request, ctx).await? {
        tracing::info!(
            ?target,
            "X11 headless region reroute; no overlay child is spawned"
        );
        return super::direct::run(target, request, ctx, started).await;
    }
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
    let heartbeat = super::Heartbeat::start(ctx);
    let image_path = child::temp_path("export.png");
    let result = child::spawn(SessionSpec {
        kind: SessionKind::Overlay,
        result_path: PathBuf::new(),
        image_path: Some(image_path.clone()),
        config_path: config_path.clone(),
        request: request.clone(),
        color_mode,
        forward_to_daemon: false,
        pin: None,
    })
    .await;
    drop(heartbeat);
    match result? {
        SessionResult::Completed { kind, selection } => {
            let Some(token) = child::kind_from_token(&kind) else {
                return Err(ExecuteError::Task(format!(
                    "the session child reported an unknown completion kind {kind:?}"
                )));
            };
            let image = child::read_export(&image_path)?;
            tracing::info!(
                target: "flowshot_perf",
                elapsed_us = elapsed(started),
                width = image.width,
                height = image.height,
                "perf.session_done"
            );
            let completion = flowshot_ui::Completion {
                kind: token,
                selection,
                image,
            };
            super::post::run_post(completion, &request, &config, config_path.as_deref(), ctx).await
        }
        SessionResult::Color { hex } => {
            // The PARENT owns the clipboard copy (the offer must
            // outlive the window session - in daemon mode this process is
            // the resident daemon).
            let clipboard = flowshot_actions::Clipboard::for_session()?;
            clipboard.copy_text(&hex)?;
            if let Some(state) = ctx.state.as_ref() {
                state.set_clipboard_offer_held(true);
            }
            Ok(ExecOutcome::ColorPicked(hex))
        }
        SessionResult::Cancelled => {
            let _ = std::fs::remove_file(&image_path);
            Ok(ExecOutcome::Cancelled)
        }
        SessionResult::Failed { error, exit_code } => {
            let _ = std::fs::remove_file(&image_path);
            Err(ExecuteError::Child { error, exit_code })
        }
        other => Err(ExecuteError::Task(format!(
            "the overlay child reported an unexpected result: {other:?}"
        ))),
    }
}

/// The child-side overlay session (runs on the child's MAIN thread - the
/// winit contract): captures live, wires the production core, runs the
/// loop, and writes the export PNG.
#[must_use]
pub fn overlay_child(spec: &SessionSpec) -> SessionResult {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return failed(error.to_string()),
    };
    let session = match runtime.block_on(prepare_overlay(spec)) {
        Ok(session) => session,
        Err(error) => {
            let code = child::exit::code_for(&error);
            return SessionResult::Failed {
                error: error.to_string(),
                exit_code: code,
            };
        }
    };
    match run_overlay_session(session) {
        SessionOutcome::Completed(completion) => {
            let Some(image_path) = spec.image_path.as_ref() else {
                return failed("the overlay spec carries no image path".to_owned());
            };
            if let Err(error) = write_export_png(image_path, &completion.image) {
                return failed(format!("export PNG write failed: {error}"));
            }
            SessionResult::Completed {
                kind: child::kind_token(completion.kind),
                selection: completion.selection,
            }
        }
        SessionOutcome::ColorPicked(hex) => SessionResult::Color { hex },
        SessionOutcome::Cancelled => SessionResult::Cancelled,
        SessionOutcome::Failed(error) => SessionResult::Failed {
            error: error.to_string(),
            exit_code: 1,
        },
    }
}

fn failed(error: String) -> SessionResult {
    SessionResult::Failed {
        error,
        exit_code: 1,
    }
}

fn write_export_png(
    path: &std::path::Path,
    image: &flowshot_ui::ExportedImage,
) -> Result<(), String> {
    let rgba = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())
        .ok_or_else(|| "export dimensions disagree with the pixel data".to_owned())?;
    rgba.save(path).map_err(|error| error.to_string())
}

/// The child-side capture preparation: the live ladder, the frozen frames,
/// the stitched editor frame, and the cursor resolution.
async fn prepare_overlay(spec: &SessionSpec) -> Result<OverlaySession, ExecuteError> {
    let started = Instant::now();
    let config = child::load_config(spec.config_path.as_deref());
    let request = spec.request.clone();
    let hide_cursor = request.hide_cursor || config.capture.hide_cursor;
    // Runtime ladder fallthrough (the direct path's contract): a rung that
    // fails AT CAPTURE hands off to the next.
    let mut excluded: Vec<flowshot_capture::BackendKind> = Vec::new();
    let (frozen, kind) = loop {
        let session = open_session_excluding(&excluded).await?;
        let kind = session.kind;
        match flowshot_ui::capture_frozen(session.backend.as_ref(), !hide_cursor).await {
            Ok(frozen) => break (frozen, kind),
            Err(error) => {
                tracing::warn!(backend = ?kind, %error, "frozen capture failed; falling through the ladder");
                excluded.push(kind);
            }
        }
    };
    tracing::info!(
        target: "flowshot_perf",
        elapsed_us = elapsed(started),
        outputs = frozen.outputs.len(),
        "perf.capture_ready"
    );
    let layout = OutputLayout::new(frozen.outputs.clone());
    let editor_frame = stitched_editor_frame(&frozen, kind, &layout);
    // The cursor ladder costs an extra ICC round-trip; only pay it when a
    // cursor-dependent preselect exists (offset-less --region, at-cursor)
    // - the perf-budget fix.
    let cursor = if needs_cursor(&request) {
        resolve_cursor().await
    } else {
        tracing::debug!("launch preselect needs no cursor; skipping the position ladder");
        None
    };
    Ok(OverlaySession {
        frozen,
        editor_frame,
        config,
        config_path: spec.config_path.clone(),
        request,
        cursor,
        color_mode: spec.color_mode,
    })
}

/// Whether the launch preselect depends on a resolved cursor position.
fn needs_cursor(request: &CaptureRequest) -> bool {
    match request.region.as_deref() {
        Some("at-cursor") => true,
        Some(token) => RegionGeometry::parse(token)
            .is_ok_and(|geometry| geometry.x.is_none() || geometry.y.is_none()),
        None => false,
    }
}

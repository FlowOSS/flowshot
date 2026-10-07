//! The launcher-dialog session (`DaemonCommand::Launcher`
//! and CLI `capture --dialog`): the egui dialog runs in a session CHILD
//! (the winit one-loop constraint). Its Capture dispatch follows the
//! recorded mapping:
//!
//! - daemon-resident (`forward_to_daemon`): the child returns the
//!   dispatch as DATA (`SessionResult::Dispatched { argv }`) and the
//!   PARENT executes it through the lossless `Invoke` channel after the
//!   session ends: `Region{geometry, delay}` -> `capture --region TOKEN
//!   [-d MS]` (interactive overlay preselected at that rect);
//!   `Screen{screen, delay}` -> `capture screen <n> [-d <ms>]`. DECISION
//!   on the open question (recorded): the delay
//!   rides the existing vocabulary, no wire extension, nothing dropped.
//!   F3 fix (2026-09-28): the child-side BUS forward this replaced could
//!   never acquire the single-window-session gate the child itself held -
//!   the Capture button was dead in daemon mode (live-QA found).
//! - one-shot (`--dialog --no-daemon`): the child captures the typed
//!   geometry DIRECTLY in-process (a second overlay loop inside the child
//!   is impossible; the launcher harness proved exactly this direct
//!   semantics live) and hands the export to the parent's post-capture.
//!
//! Cancel -> [`ExecOutcome::Cancelled`] (the CLI's exit-3 class).

mod dispatch;

use std::time::Instant;

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::launcher::{
    LaunchCallback, LauncherRequest, LauncherWindow, LauncherWindowOptions, MonitorProbe,
};

use super::session::{self, SessionKind, SessionResult, SessionSpec};
use super::{ExecCtx, ExecOutcome, ExecuteError, elapsed};
use crate::request::CaptureRequest;

/// The parent side: spawns the launcher child and maps its result.
///
/// # Errors
///
/// [`ExecuteError`] per stage; Cancel is an outcome.
pub async fn run(ctx: &ExecCtx, started: Instant) -> Result<ExecOutcome, ExecuteError> {
    let (config, config_path) = ctx.load_config();
    let heartbeat = super::Heartbeat::start(ctx);
    let image_path = session::temp_path("launcher-export.png");
    let result = session::spawn(SessionSpec {
        kind: SessionKind::Launcher,
        result_path: std::path::PathBuf::new(),
        image_path: Some(image_path.clone()),
        config_path: config_path.clone(),
        request: CaptureRequest::default(),
        color_mode: false,
        forward_to_daemon: ctx.state.is_some(),
        pin: None,
    })
    .await;
    drop(heartbeat);
    tracing::info!(
        target: "flowshot_perf",
        elapsed_us = elapsed(started),
        "perf.launcher_closed"
    );
    match result? {
        // The parent executes the handed-back dispatch itself; the
        // session guard released when `spawn` returned, so the capture's
        // own window session can acquire it now. Dispatched through the
        // concrete Invoke legs (not `execute` - that would recurse into
        // the Launcher arm; `dispatch_argv` only ever builds captures).
        SessionResult::Dispatched { argv } => match super::invoke::parse(&argv)? {
            super::invoke::InvokeCall::Direct(target, request) => {
                super::direct::run(target, request, ctx, started).await
            }
            super::invoke::InvokeCall::Interactive(request) => {
                super::overlay::run_interactive(request, ctx, started, false).await
            }
            other => Err(ExecuteError::Task(format!(
                "the launcher dispatch parsed to a non-capture call: {other:?}"
            ))),
        },
        // One-shot: the child captured directly; the parent runs
        // post-capture (clipboard offer ownership stays with the invoker).
        SessionResult::Completed { kind, selection } => {
            let Some(token) = session::kind_from_token(&kind) else {
                return Err(ExecuteError::Task(format!(
                    "the launcher child reported an unknown completion kind {kind:?}"
                )));
            };
            let image = session::read_export(&image_path)?;
            let completion = flowshot_ui::Completion {
                kind: token,
                selection,
                image,
            };
            super::post::run_post(
                completion,
                &CaptureRequest::default(),
                &config,
                config_path.as_deref(),
                ctx,
            )
            .await
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
            "the launcher child reported an unexpected result: {other:?}"
        ))),
    }
}

/// The child-side launcher session (main thread).
#[must_use]
pub fn launcher_child(spec: &SessionSpec) -> SessionResult {
    let config = session::load_config(spec.config_path.as_deref());
    let forward = spec.forward_to_daemon;
    let result_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<SessionResult>));
    let on_capture = {
        let result_slot = std::sync::Arc::clone(&result_slot);
        let spec = spec.clone();
        LaunchCallback::new(move |request: &LauncherRequest| {
            let outcome = if forward {
                dispatch::dispatch_argv(request)
            } else {
                dispatch::capture_in_child(request, &spec)
            };
            *result_slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
        })
    };
    let options = LauncherWindowOptions {
        tokens: DesignTokens::default(),
        ui_config: config.ui.clone(),
        system_theme: super::settings::query_system_theme(),
        monitor_probe: Some(MonitorProbe::new(probe_outputs)),
        on_capture: Some(on_capture),
        clipboard: Some(super::settings::clipboard_bridge()),
        window_customizer: Some(super::window::session_window_customizer(
            "flowshot-launcher",
            "Capture Launcher",
        )),
    };
    let window = match LauncherWindow::new(options) {
        Ok(window) => window,
        Err(error) => {
            return SessionResult::Failed {
                error: error.to_string(),
                exit_code: 1,
            };
        }
    };
    if let Err(error) = window.run() {
        return SessionResult::Failed {
            error: error.to_string(),
            exit_code: 1,
        };
    }
    match result_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        Some(result) => result,
        None => SessionResult::Cancelled,
    }
}

fn probe_outputs() -> Vec<flowshot_core::geometry::OutputInfo> {
    use flowshot_actions::clipboard::{SessionKind, detect_session};
    match detect_session() {
        Ok(SessionKind::X11) => probe_outputs_x11(),
        // A Wayland session - or no session variable at all, where this leg
        // keeps its existing warn-and-empty degradation (the UI gate errors
        // first anyway; the execute/backend.rs routing precedent).
        Ok(SessionKind::Wayland) | Err(_) => probe_outputs_wayland(),
    }
}

fn probe_outputs_wayland() -> Vec<flowshot_core::geometry::OutputInfo> {
    match flowshot_capture_wayland::CaptureThread::spawn() {
        Ok(thread) => thread.outputs().unwrap_or_else(|error| {
            tracing::warn!(%error, "launcher monitor probe failed");
            Vec::new()
        }),
        Err(error) => {
            tracing::warn!(%error, "launcher probe connection failed");
            Vec::new()
        }
    }
}

fn probe_outputs_x11() -> Vec<flowshot_core::geometry::OutputInfo> {
    use flowshot_capture::CaptureBackend;
    // The session child has no ambient executor; the X11 backend's worker
    // futures are runtime-agnostic, so a plain block_on drives the RANDR
    // enumeration (the query_system_theme blocking-probe precedent).
    let probe = async {
        let backend = flowshot_capture_x11::X11Backend::connect_bounded().await?;
        backend.outputs().await
    };
    futures::executor::block_on(probe).unwrap_or_else(|error| {
        tracing::warn!(
            error = %crate::execute::backend::error_detail(&error),
            "launcher monitor probe failed"
        );
        Vec::new()
    })
}

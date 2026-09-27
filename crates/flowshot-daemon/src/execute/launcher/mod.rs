//! The launcher-dialog session (plan todo 37/38, `DaemonCommand::Launcher`
//! and CLI `capture --dialog`): the egui dialog runs in a session CHILD
//! (the winit one-loop constraint). Its Capture dispatch follows the
//! todo-37 mapping:
//!
//! - daemon-resident (`forward_to_daemon`): `Region{geometry, delay}` ->
//!   the bus `Capture(a{sv})` member with the geometry's token (the daemon
//!   then runs the interactive overlay preselected at that rect);
//!   `Screen{screen, delay}` -> the bus `Invoke` channel
//!   (`capture screen <n> -d <ms>` - the typed `CaptureScreen(u)` member
//!   carries no delay; the Invoke channel is lossless). DECISION on the
//!   todo-37 open question, recorded in decisions.md: the delay rides the
//!   existing vocabulary, no wire extension, nothing dropped.
//! - one-shot (`--dialog --no-daemon`): the child captures the typed
//!   geometry DIRECTLY in-process (a second overlay loop inside the child
//!   is impossible; the todo-37 harness proved exactly this direct
//!   semantics live) and hands the export to the parent's post-capture.
//!
//! Cancel -> [`ExecOutcome::Cancelled`] (the CLI's exit-3 class).

mod dispatch;

use std::time::Instant;

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::launcher::{
    LaunchCallback, LauncherRequest, LauncherWindow, LauncherWindowOptions, MonitorProbe,
};
use flowshot_ui::pins::WindowCustomizer;
use winit::platform::wayland::WindowAttributesExtWayland;

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
        // The daemon executes the dispatched capture itself.
        SessionResult::Dispatched => Ok(ExecOutcome::Done(
            flowshot_actions::clipboard::PostCaptureReport::default(),
        )),
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
                dispatch::forward_to_daemon(request)
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
        window_customizer: Some(WindowCustomizer::new(|attributes| {
            attributes.with_name("flowshot-launcher", "Capture Launcher")
        })),
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

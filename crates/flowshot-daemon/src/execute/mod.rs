//! The executing command pipeline: `DaemonCommand` ->
//! capture -> overlay/editor -> post-capture actions.
//!
//! One executor, three drivers:
//!
//! - the DAEMON ([`ExecutingSink`]): every bus/tray/shortcut command runs
//!   on a dedicated thread with its own current-thread runtime (zbus's
//!   dispatch tasks must never host blocking window loops - the
//!   recorded executor decision);
//! - the CLI ONE-SHOT path (`--no-daemon`, `--raw`, `--print-geometry`):
//!   `flowshot-cli` awaits [`execute`] directly inside its tokio runtime
//!   (the blocking overlay leg rides `spawn_blocking`);
//! - the HEADLESS execution mode (feature `test-drive`,
//!   `headless`: the same core wiring
//!   ([`overlay::configure_core`]) and the same export implementation
//!   ([`flowshot_ui::render_export`]) driven by synthetic input with
//!   offscreen GPU renders - no window, no compositor disturbance. This is
//!   the minimal honest stand-in for the visible overlay session (a
//!   virtual seat would need a nested compositor, which is forbidden);
//!   the winit/Wayland window leg itself is covered by the per-module live
//!   evidence and the deferred GUI-QA batch.
//!
//! Perf budget instrumentation (Metis #12): every stage emits
//! `target: "flowshot_perf"` events with monotonic `elapsed_us` from the
//! command-received instant (`perf.command`, `perf.capture_ready`,
//! `perf.frame_ready`, `perf.done`), so hotkey->frame-ready budgets are
//! assertable from logs alone.

pub use sink::{ExecutingSink, Heartbeat};

pub mod backend;
pub mod consent;
pub mod direct;
#[cfg(feature = "test-drive")]
pub mod headless;
pub mod invoke;
pub mod launcher;
pub mod overlay;
pub mod pin;
pub mod post;
pub mod session;
pub mod settings;
mod sink;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use flowshot_capture::CaptureError;
use flowshot_core::Config;

use crate::command::DaemonCommand;
use crate::error::DaemonError;
use crate::notify::Notifier;
use crate::state::DaemonState;

/// What the executor needs beyond the command itself.
#[derive(Clone)]
pub struct ExecCtx {
    /// Explicit config path (`None` = the platform default location).
    pub config_path: Option<PathBuf>,
    /// Daemon-resident state (persistence reasons, pin registry); `None`
    /// in the one-shot CLI process.
    pub state: Option<Arc<DaemonState>>,
    /// The daemon's notification seam; `None` falls back to a fresh
    /// config-gated desktop notifier.
    pub notifier: Option<Arc<dyn Notifier>>,
    /// Upload endpoint override (QA knob, the `bus_address` precedent):
    /// `Some(base_url)` points the Imgur provider at a stub (wiremock e2e);
    /// `None` = the production endpoint.
    pub upload_base_url: Option<String>,
}

impl std::fmt::Debug for ExecCtx {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExecCtx")
            .field("config_path", &self.config_path)
            .field("state", &self.state.is_some())
            .field("notifier", &self.notifier.is_some())
            .field("upload_base_url", &self.upload_base_url)
            .finish()
    }
}

impl ExecCtx {
    /// A one-shot CLI context (no daemon state).
    #[must_use]
    pub fn one_shot(config_path: Option<PathBuf>) -> Self {
        Self {
            config_path,
            state: None,
            notifier: None,
            upload_base_url: None,
        }
    }

    /// Loads the config fresh per execution (a settings apply takes effect
    /// on the next capture - the "no `ConfigChanged` signal yet"
    /// decision), falling back to defaults with a warning (the
    /// corrupt-file contract).
    #[must_use]
    pub fn load_config(&self) -> (Config, Option<PathBuf>) {
        let path = self
            .config_path
            .clone()
            .or_else(|| crate::paths::default_config_path().ok());
        let config = match &path {
            Some(path) => Config::load(path).unwrap_or_else(|error| {
                tracing::warn!(%error, "config load failed; using defaults");
                Config::default()
            }),
            None => Config::default(),
        };
        (config, path)
    }
}

/// How one executed command ended.
#[derive(Debug)]
pub enum ExecOutcome {
    /// A capture completed; the report carries per-action outcomes.
    Done(flowshot_actions::clipboard::PostCaptureReport),
    /// The user cancelled (Esc teardown, dialog Cancel).
    Cancelled,
    /// `flowshot color`: the picked hex was copied to the clipboard.
    ColorPicked(String),
}

/// Executor failures, mapped onto the CLI exit-code table by
/// `flowshot-cli`.
#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    /// A usage-class rejection (bad argv, conflicting stdout modes).
    #[error("usage error: {0}")]
    Usage(String),
    /// The capture ladder failed (backend or probe).
    #[error("capture failed: {0}")]
    Capture(#[from] CaptureError),
    /// The Wayland session probe thread failed.
    #[error("session probe failed: {0}")]
    Probe(#[from] flowshot_capture_wayland::ProbeError),
    /// The compositor connection failed.
    #[error("compositor connection failed: {0}")]
    Connect(#[from] flowshot_capture_wayland::ConnectError),
    /// A window runtime failed (overlay, pin, launcher, settings).
    #[error("ui runtime failed: {0}")]
    Ui(#[from] flowshot_ui::UiError),
    /// Encoding/saving/stdout failed.
    #[error("export failed: {0}")]
    Export(#[from] flowshot_actions::ExportError),
    /// The clipboard offer failed.
    #[error("clipboard failed: {0}")]
    Clipboard(#[from] flowshot_actions::ClipboardError),
    /// Filesystem failure.
    #[error("io failed: {0}")]
    Io(#[from] std::io::Error),
    /// A blocking executor task failed to join.
    #[error("executor task failed: {0}")]
    Task(String),
    /// A window-session child failed; `exit_code` is the child's mapping
    /// of its own failure onto the shared exit-code table (the one-shot CLI
    /// propagates it verbatim).
    #[error("session child failed (exit {exit_code}): {error}")]
    Child {
        /// The child's error text.
        error: String,
        /// The shared exit-code table value.
        exit_code: u8,
    },
    /// A daemon-side typed error (config paths).
    #[error("daemon error: {0}")]
    Daemon(#[from] DaemonError),
}

/// Runs one command to completion (blocking the current thread for window
/// sessions; the async legs are the capture/upload protocol round-trips).
///
/// # Errors
///
/// [`ExecuteError`] per stage; user cancellation is an [`ExecOutcome`],
/// never an error.
pub async fn execute(command: DaemonCommand, ctx: &ExecCtx) -> Result<ExecOutcome, ExecuteError> {
    let started = Instant::now();
    tracing::info!(
        target: "flowshot_perf",
        command = %command,
        elapsed_us = 0u64,
        "perf.command"
    );
    let outcome = route(command, ctx, started).await;
    match &outcome {
        Ok(_) => tracing::info!(
            target: "flowshot_perf",
            elapsed_us = elapsed(started),
            "perf.done"
        ),
        Err(error) => {
            tracing::error!(%error, "execution failed");
            capture_failure(error);
        }
    }
    outcome
}

/// The executor's telemetry seam: captures one typed failure with the
/// derived `backend` tag (the `surface` tag rides every event from init).
/// No-op while telemetry is disabled. Both funnels call this: the daemon
/// side ([`execute`]) and the CLI one-shot side (`flowshot-cli`'s
/// exit-code mapping).
pub fn capture_failure(error: &ExecuteError) {
    match backend_tag(error) {
        Some(backend) => crate::telemetry::capture_error_tagged(error, &[("backend", &backend)]),
        None => crate::telemetry::capture_error(error),
    }
}

/// The capture-`backend` tag for one executor failure: the failing ladder
/// rung's protocol name, `none` for the exhausted-ladder class, `layout`
/// for the region miss, `probe` for the session probe/connect failures,
/// `session-child` for the child-process failures.
fn backend_tag(error: &ExecuteError) -> Option<String> {
    match error {
        ExecuteError::Capture(capture) => Some(match capture {
            CaptureError::NoBackendAvailable { .. } => "none".to_owned(),
            CaptureError::Timeout { backend }
            | CaptureError::Decode { backend, .. }
            | CaptureError::Backend { backend, .. } => backend.to_string(),
            CaptureError::RegionOutsideLayout { .. } => "layout".to_owned(),
        }),
        ExecuteError::Probe(_) | ExecuteError::Connect(_) => Some("probe".to_owned()),
        ExecuteError::Child { .. } => Some("session-child".to_owned()),
        ExecuteError::Usage(_)
        | ExecuteError::Ui(_)
        | ExecuteError::Export(_)
        | ExecuteError::Clipboard(_)
        | ExecuteError::Io(_)
        | ExecuteError::Task(_)
        | ExecuteError::Daemon(_) => None,
    }
}

async fn route(
    command: DaemonCommand,
    ctx: &ExecCtx,
    started: Instant,
) -> Result<ExecOutcome, ExecuteError> {
    match command {
        DaemonCommand::CaptureFull => {
            direct::run(
                direct::Target::Full,
                crate::request::CaptureRequest::default(),
                ctx,
                started,
            )
            .await
        }
        DaemonCommand::CaptureScreen(screen) => {
            let target = if screen == crate::shortcut::ACTIVE_SCREEN {
                direct::Target::Screen(direct::ScreenTarget::Cursor)
            } else {
                direct::Target::Screen(direct::ScreenTarget::Index(screen))
            };
            direct::run(
                target,
                crate::request::CaptureRequest::default(),
                ctx,
                started,
            )
            .await
        }
        DaemonCommand::Capture(request) => {
            overlay::run_interactive(request, ctx, started, false).await
        }
        DaemonCommand::Launcher => launcher::run(ctx, started).await,
        DaemonCommand::Settings => settings::run(ctx).await,
        DaemonCommand::Invoke(argv) => match invoke::parse(&argv)? {
            invoke::InvokeCall::Direct(target, request) => {
                direct::run(target, request, ctx, started).await
            }
            invoke::InvokeCall::Interactive(request) => {
                overlay::run_interactive(request, ctx, started, false).await
            }
            invoke::InvokeCall::Pin(file) => sink::pin_last_or_file(file, ctx).await,
            invoke::InvokeCall::Color => {
                overlay::run_interactive(
                    crate::request::CaptureRequest::default(),
                    ctx,
                    started,
                    true,
                )
                .await
            }
        },
    }
}

/// Microseconds since `started` (saturating at `u64::MAX`).
#[must_use]
pub fn elapsed(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

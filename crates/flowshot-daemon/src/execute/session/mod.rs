//! The window-session child-process contract.
//!
//! winit 0.30 permits exactly ONE event loop per process
//! (`EventLoopBuilder::build` -> `RecreationAttempt` on the second), so a
//! daemon can never host overlay, pin, launcher, and settings sessions on
//! worker threads. Every window session therefore runs in a dedicated
//! child of the daemon/CLI process:
//!
//! ```text
//! <current_exe> session --spec /tmp/flowshot-session-<nonce>.json
//! ```
//!
//! The child owns the winit loop (main thread, one per process - the
//! Flameshot daemon/GUI split precedent), renders the export through the
//! production path, and writes the result (JSON + export PNG) to the
//! spec-mandated temp paths. The PARENT runs post-capture, so the
//! clipboard offer stays daemon-owned, the pin registry and
//! persistence reasons stay in one process, and a crashed
//! session never takes the daemon down.
//!
//! The verb is hidden from the user surface (internal process-model
//! contract, not a dev subcommand - recorded).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use flowshot_core::geometry::LogicalRect;
use serde::{Deserialize, Serialize};

pub mod exit;

use super::ExecuteError;
use crate::request::CaptureRequest;

/// One window session the parent asks the child to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionKind {
    /// The interactive capture overlay (or the `flowshot color` session
    /// with `color_mode`).
    Overlay,
    /// The manual-coordinate launcher dialog.
    Launcher,
    /// The settings window.
    Settings,
    /// A pin window session.
    Pin,
    /// The first-launch telemetry consent dialog (the daemon-startup
    /// prompt; spawned DETACHED - see [`super::consent`]).
    Consent,
}

/// Pin-session parameters (the image travels as a temp PNG).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinParams {
    /// Host-assigned pin id (registry bridge value).
    pub id: u64,
    /// `[pin].min_size`.
    pub min_size: u32,
}

/// The child's job description (JSON at `--spec PATH`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    /// Which session to run.
    pub kind: SessionKind,
    /// Where the child writes the [`SessionResult`] JSON.
    pub result_path: PathBuf,
    /// Where the child writes the export PNG (overlay/launcher one-shot).
    pub image_path: Option<PathBuf>,
    /// Config TOML the child loads (`None` = the platform default).
    pub config_path: Option<PathBuf>,
    /// The invocation modifiers (overlay sessions).
    #[serde(default)]
    pub request: CaptureRequest,
    /// `flowshot color`: the eyedropper session.
    #[serde(default)]
    pub color_mode: bool,
    /// Launcher child: forward the dispatch to the daemon over the bus
    /// (production mapping) instead of capturing in-process.
    #[serde(default)]
    pub forward_to_daemon: bool,
    /// Pin-session parameters.
    pub pin: Option<PinParams>,
}

/// What the child reports back (JSON at `result_path`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum SessionResult {
    /// A capture completed; the export PNG is at `image_path`.
    Completed {
        /// The completing gesture token (`accept`, `copy`, `save`, `pin`,
        /// `upload`, `open-with`).
        kind: String,
        /// The accepted selection (global logical).
        selection: LogicalRect,
    },
    /// The standalone color pick (hex already recorded; the PARENT owns
    /// the clipboard copy so the offer outlives the child).
    Color {
        /// The picked `#RRGGBB`.
        hex: String,
    },
    /// Esc/Cancel without a completion.
    Cancelled,
    /// The launcher handed its dispatch back for the PARENT to run (the
    /// parent executes the `Invoke` argv after the session ends - the
    /// child's own session holds the single-window-session gate, so a
    /// child-side dispatch can never acquire it).
    Dispatched {
        /// The lossless `Invoke` argv tail (everything after argv\[0\]).
        argv: Vec<String>,
    },
    /// The session window closed normally (settings/pins).
    Closed,
    /// The session failed; `exit_code` carries the shared exit-code table.
    Failed {
        /// Human-readable error.
        error: String,
        /// The mapped exit code (the parent propagates it one-shot).
        exit_code: u8,
    },
}

/// Single-active-window-session gate (the dropped
/// `allowMultipleGuiInstances` semantic: ONE exclusive GUI session at a
/// time; direct window-less captures are not gated). Pin sessions are
/// EXEMPT: pins are not exclusive GUI sessions - the multi-pin registry
/// and the `pins_alive` lifecycle reason are built for coexisting pins,
/// and a new capture must work while pins float (Flameshot parity, where
/// the single-instance option gates the capture GUI, never pin widgets).
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);

struct SessionGuard;

impl SessionGuard {
    fn acquire() -> Result<Self, ExecuteError> {
        if SESSION_ACTIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            == Ok(false)
        {
            Ok(Self)
        } else {
            Err(ExecuteError::Usage(
                "a FlowShot window session is already active".to_owned(),
            ))
        }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        SESSION_ACTIVE.store(false, Ordering::Release);
    }
}

/// Whether one session kind takes the single-active-window-session gate:
/// every exclusive GUI session does; pins are EXEMPT (the multi-pin
/// registry and the `pins_alive` lifecycle reason exist for coexisting
/// pins, and a new capture must work while pins float - Flameshot parity,
/// where the single-instance option gates the capture GUI, never pin
/// widgets). Platform-free: the exemption applies to Wayland and X11
/// alike. A NEW variant is gated by default (the safe direction: an
/// exclusive session that wrongly coexists is worse than a coexisting one
/// that wrongly gates); exempt it here deliberately, with a test.
const fn takes_session_gate(kind: &SessionKind) -> bool {
    !matches!(kind, SessionKind::Pin)
}

/// Parent side: spawns one session child and waits for its result.
///
/// # Errors
///
/// [`ExecuteError::Io`] for spawn/wait failures, [`ExecuteError::Task`]
/// when the child produced no parsable result, [`ExecuteError::Usage`]
/// when another window session is already active.
pub async fn spawn(spec: SessionSpec) -> Result<SessionResult, ExecuteError> {
    let _guard = if takes_session_gate(&spec.kind) {
        Some(SessionGuard::acquire()?)
    } else {
        None
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ExecuteError::Task(format!("clock before the epoch: {error}")))?
        .as_nanos();
    let base =
        std::env::temp_dir().join(format!("flowshot-session-{}-{nonce}", std::process::id()));
    let spec_path = base.with_extension("json");
    let result_path = base.with_extension("result.json");
    let mut spec = spec;
    spec.result_path = result_path.clone();
    serde_json::to_writer(std::fs::File::create(&spec_path)?, &spec)
        .map_err(|error| ExecuteError::Task(format!("session spec write failed: {error}")))?;
    let exe = std::env::current_exe()?;
    tracing::info!(exe = %exe.display(), spec = %spec_path.display(), "session child spawning");
    let spawn_spec_path = spec_path.clone();
    let child = tokio::task::spawn_blocking(move || {
        std::process::Command::new(&exe)
            .arg("session")
            .arg("--spec")
            .arg(&spawn_spec_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
    })
    .await
    .map_err(|error| ExecuteError::Task(format!("session spawn join failed: {error}")))??;
    let result = read_result(&result_path).unwrap_or_else(|| SessionResult::Failed {
        error: format!("the session child exited {child} without a result"),
        exit_code: exit::exit_code_for_child_failure(),
    });
    let _ = std::fs::remove_file(&spec_path);
    let _ = std::fs::remove_file(&result_path);
    Ok(result)
}

/// A collision-free temp path for session handoff artifacts.
#[must_use]
pub fn temp_path(tag: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!("flowshot-{}-{nonce}-{tag}", std::process::id()))
}

fn read_result(path: &Path) -> Option<SessionResult> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<SessionResult>(&text) {
        Ok(result) => Some(result),
        Err(error) => {
            tracing::error!(%error, "session result JSON is corrupt");
            None
        }
    }
}

/// Reads the child's export PNG into the completion image type.
///
/// # Errors
///
/// [`ExecuteError::Export`] when the file is missing or undecodable.
pub fn read_export(image_path: &Path) -> Result<flowshot_ui::ExportedImage, ExecuteError> {
    let loaded = image::open(image_path)
        .map_err(|error| {
            flowshot_actions::ExportError::ImageEncode(format!(
                "session export PNG unreadable ({}): {error}",
                image_path.display()
            ))
        })?
        .to_rgba8();
    let (width, height) = loaded.dimensions();
    let rgba = loaded.into_raw();
    let _ = std::fs::remove_file(image_path);
    Ok(flowshot_ui::ExportedImage {
        width,
        height,
        rgba,
    })
}

/// The child-side config load (explicit path or the platform default,
/// corrupt/missing -> defaults with a warning - the resilience contract).
#[must_use]
pub fn load_config(path: Option<&Path>) -> flowshot_core::Config {
    match path {
        Some(path) => flowshot_core::Config::load(path).unwrap_or_else(|error| {
            tracing::warn!(%error, "config load failed; using defaults");
            flowshot_core::Config::default()
        }),
        None => crate::paths::default_config_path()
            .ok()
            .and_then(|path| flowshot_core::Config::load(&path).ok())
            .unwrap_or_default(),
    }
}

/// Child side: runs the session described by `--spec PATH` and writes the
/// result JSON. Returns the process exit code.
#[must_use]
pub fn run_child(spec_path: &Path) -> u8 {
    let spec: SessionSpec = match std::fs::read_to_string(spec_path)
        .map_err(ExecuteError::Io)
        .and_then(|text| {
            serde_json::from_str(&text)
                .map_err(|error| ExecuteError::Task(format!("session spec parse failed: {error}")))
        }) {
        Ok(spec) => spec,
        Err(error) => {
            eprintln!("flowshot session: {error}");
            return 1;
        }
    };
    // Per-child telemetry (the spec's config + the surface its kind
    // names): the guard lives for the whole child, and the sentry panic
    // hook covers the child's process-global panics from here on. Typed
    // child failures are NOT captured here - they cross the process
    // boundary as `SessionResult::Failed` and the PARENT captures them
    // once (`ExecuteError::Child`), so one failure is one event.
    let config = load_config(spec.config_path.as_deref());
    let _telemetry = crate::telemetry::init(&config.telemetry, spec.kind.clone().into());
    let result = dispatch_child(&spec);
    let code = match &result {
        SessionResult::Failed { exit_code, .. } => *exit_code,
        _ => 0,
    };
    match serde_json::to_string(&result) {
        Ok(json) => {
            if let Err(error) = std::fs::write(&spec.result_path, json) {
                eprintln!("flowshot session: result write failed: {error}");
                return 1;
            }
        }
        Err(error) => {
            eprintln!("flowshot session: result serialize failed: {error}");
            return 1;
        }
    }
    code
}

fn dispatch_child(spec: &SessionSpec) -> SessionResult {
    match spec.kind {
        SessionKind::Overlay => super::overlay::overlay_child(spec),
        SessionKind::Launcher => super::launcher::launcher_child(spec),
        SessionKind::Settings => super::settings::settings_child(spec),
        SessionKind::Pin => super::pin::pin_child(spec),
        SessionKind::Consent => super::consent::consent_child(spec),
    }
}

/// The completion-kind wire tokens.
#[must_use]
pub fn kind_token(kind: flowshot_ui::CompletionKind) -> String {
    match kind {
        flowshot_ui::CompletionKind::Accept => "accept",
        flowshot_ui::CompletionKind::Copy => "copy",
        flowshot_ui::CompletionKind::Save => "save",
        flowshot_ui::CompletionKind::Pin => "pin",
        flowshot_ui::CompletionKind::Upload => "upload",
        flowshot_ui::CompletionKind::OpenWith => "open-with",
    }
    .to_owned()
}

/// Parses a completion-kind wire token.
#[must_use]
pub fn kind_from_token(token: &str) -> Option<flowshot_ui::CompletionKind> {
    Some(match token {
        "accept" => flowshot_ui::CompletionKind::Accept,
        "copy" => flowshot_ui::CompletionKind::Copy,
        "save" => flowshot_ui::CompletionKind::Save,
        "pin" => flowshot_ui::CompletionKind::Pin,
        "upload" => flowshot_ui::CompletionKind::Upload,
        "open-with" => flowshot_ui::CompletionKind::OpenWith,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_sessions_are_exempt_from_the_single_window_gate() {
        // The multi-pin architecture: pins coexist with each other and
        // with new captures.
        assert!(!takes_session_gate(&SessionKind::Pin));
    }

    #[test]
    fn exclusive_gui_sessions_take_the_gate() {
        for kind in [
            SessionKind::Overlay,
            SessionKind::Launcher,
            SessionKind::Settings,
            SessionKind::Consent,
        ] {
            assert!(takes_session_gate(&kind), "{kind:?} must be gated");
        }
    }

    #[test]
    fn guard_acquire_is_exclusive_and_drop_releases() {
        // One sequential body: SESSION_ACTIVE is process-global.
        let first = SessionGuard::acquire();
        assert!(first.is_ok());
        assert!(matches!(
            SessionGuard::acquire(),
            Err(ExecuteError::Usage(_))
        ));
        drop(first);
        let third = SessionGuard::acquire();
        assert!(third.is_ok());
        drop(third);
    }
}

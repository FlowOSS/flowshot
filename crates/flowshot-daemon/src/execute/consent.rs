//! The first-launch telemetry consent session: the daemon-startup gate
//! (parent side) and the dialog child leg with its config write-back.
//!
//! # Trigger rule (documented contract, `flowshot_ui::consent`)
//!
//! The RESIDENT DAEMON is the single prompt owner: [`Daemon::start`]
//! consults [`should_prompt`] with [`PromptSurface::DaemonStartup`] after
//! the bus-name acquisition and spawns the dialog as a DETACHED session
//! child. The one-shot `--no-daemon` path and every session child
//! (overlay/launcher/settings/pin/consent) NEVER spawn it - a consent
//! popup mid-capture is hostile, and a one-shot process is gone before
//! the user could answer. Deferral is safe: the dialog appears on the
//! next daemon-managed start (the auto-spawned helper counts - it IS a
//! daemon startup, and the detached spawn never blocks the capture it
//! may coincide with), and the settings window's Telemetry card offers
//! the same choice at any time.
//!
//! # Detached contract
//!
//! No command may block on consent, so the spawn is fire-and-forget:
//! the child is NEVER awaited by any command flow, and it deliberately
//! does NOT acquire the single-window-session gate (a prompt must never
//! fail a capture with "session already active"). A parked reaper thread
//! waits the child purely to reap it and delete the handoff files; the
//! ANSWER never travels back through the parent - the child persists it
//! itself (`record_choice`: read-modify-write of `[telemetry]` through
//! the migration-safe `Config::save`). If the daemon exits before the
//! dialog closes, the child is reparented to init (its write-back still
//! lands) and the two temp JSON files leak into `/tmp` for the system
//! tmpfiles policy to reclaim.
//!
//! [`Daemon::start`]: crate::daemon::Daemon::start

use std::path::{Path, PathBuf};
use std::process::Stdio;

use flowshot_core::Config;
use flowshot_core::config::TelemetryConfig;
use flowshot_core::tokens::DesignTokens;
use flowshot_ui::consent::{
    ConsentCallback, ConsentWindow, ConsentWindowOptions, PromptSurface, WINDOW_TITLE,
    should_prompt,
};
use flowshot_ui::pins::WindowCustomizer;
use winit::platform::wayland::WindowAttributesExtWayland;

use super::ExecuteError;
use super::session::{self, SessionKind, SessionResult, SessionSpec};

/// The consent-prompt wiring (the `ShortcutOptions` opt-in shape):
/// DISABLED by default so callers that do not opt in (tests, library
/// users) keep their exact startup behavior and never spawn a window
/// child; both daemon binaries enable it via [`Self::daemon_startup`].
#[derive(Debug, Clone, Default)]
pub struct ConsentPrompt {
    /// Whether the daemon-startup gate is wired at all.
    pub enabled: bool,
    /// The config path the session child loads and writes back (`None` =
    /// the platform default; mirrors the binaries' `--config` knob).
    pub config_path: Option<PathBuf>,
}

impl ConsentPrompt {
    /// The binaries' production wiring: the gate runs, and the child
    /// loads/writes `config_path` (`None` = the platform default).
    #[must_use]
    pub const fn daemon_startup(config_path: Option<PathBuf>) -> Self {
        Self {
            enabled: true,
            config_path,
        }
    }
}

/// The daemon-startup spawn decision: the wiring opt-in AND the consent
/// trigger rule on the daemon surface. Split from the spawn side effect
/// so the truth table is unit-testable without child processes.
#[must_use]
pub fn spawn_armed(wired: bool, telemetry: &TelemetryConfig) -> bool {
    wired && should_prompt(telemetry, PromptSurface::DaemonStartup)
}

/// The daemon-startup gate: spawns the consent dialog DETACHED when
/// armed (never awaited, log-only - a spawn failure re-arms the prompt
/// on the next daemon start).
pub fn prompt_first_launch(prompt: &ConsentPrompt, telemetry: &TelemetryConfig) {
    if !spawn_armed(prompt.enabled, telemetry) {
        tracing::debug!(
            wired = prompt.enabled,
            asked = telemetry.asked_on_first_launch,
            "consent prompt not armed; skipping"
        );
        return;
    }
    match spawn_detached(prompt.config_path.clone()) {
        Ok(()) => tracing::info!("first-launch consent dialog spawned (detached)"),
        Err(error) => tracing::warn!(
            %error,
            "consent dialog spawn failed; the next daemon start asks again"
        ),
    }
}

/// The spec the parent writes for a detached prompt (shared with the
/// wire round-trip test).
fn consent_spec(config_path: Option<PathBuf>, result_path: PathBuf) -> SessionSpec {
    SessionSpec {
        kind: SessionKind::Consent,
        result_path,
        image_path: None,
        config_path,
        request: crate::request::CaptureRequest::default(),
        color_mode: false,
        forward_to_daemon: false,
        pin: None,
    }
}

/// Parent side: spawns the consent session child and returns
/// immediately (the detached contract, module header).
fn spawn_detached(config_path: Option<PathBuf>) -> Result<(), ExecuteError> {
    let spec = consent_spec(config_path, session::temp_path("consent-result.json"));
    let result_path = spec.result_path.clone();
    let spec_path = session::temp_path("consent-spec.json");
    serde_json::to_writer(std::fs::File::create(&spec_path)?, &spec)
        .map_err(|error| ExecuteError::Task(format!("consent spec write failed: {error}")))?;
    let exe = std::env::current_exe()?;
    let child = std::process::Command::new(&exe)
        .arg("session")
        .arg("--spec")
        .arg(&spec_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()?;
    tracing::info!(exe = %exe.display(), pid = child.id(), "consent session child spawning");
    reap_detached(child, spec_path, result_path);
    Ok(())
}

/// The reaper: a parked thread that waits the child purely to reap it
/// and delete the handoff files (no command flow may await the prompt).
fn reap_detached(mut child: std::process::Child, spec_path: PathBuf, result_path: PathBuf) {
    let reaper = std::thread::Builder::new()
        .name("flowshot-consent-reaper".to_owned())
        .spawn(move || {
            match child.wait() {
                Ok(status) => tracing::debug!(%status, "consent session child exited"),
                Err(error) => tracing::warn!(%error, "consent reaper wait failed"),
            }
            let _ = std::fs::remove_file(&spec_path);
            let _ = std::fs::remove_file(&result_path);
        });
    if let Err(error) = reaper {
        tracing::warn!(%error, "consent reaper thread failed to start; temp files may leak");
    }
}

/// The child-side consent session (main thread): the dialog itself, with
/// the persistence seam wired to the `[telemetry]` write-back.
#[must_use]
pub fn consent_child(spec: &SessionSpec) -> SessionResult {
    let Some(config_path) = spec
        .config_path
        .clone()
        .or_else(|| crate::paths::default_config_path().ok())
    else {
        return SessionResult::Failed {
            error: "no config path resolves (HOME unset?)".to_owned(),
            exit_code: 1,
        };
    };
    let config = session::load_config(Some(config_path.as_path()));
    let options = ConsentWindowOptions {
        tokens: DesignTokens::default(),
        ui_config: config.ui.clone(),
        system_theme: super::settings::query_system_theme(),
        on_choice: Some(ConsentCallback::new(move |choice| {
            record_choice(&config_path, *choice);
        })),
        window_customizer: Some(WindowCustomizer::new(|attributes| {
            attributes.with_name("flowshot-consent", WINDOW_TITLE)
        })),
    };
    let window = match ConsentWindow::new(options) {
        Ok(window) => window,
        Err(error) => {
            return SessionResult::Failed {
                error: error.to_string(),
                exit_code: 1,
            };
        }
    };
    match window.run() {
        Ok(()) => SessionResult::Closed,
        Err(error) => SessionResult::Failed {
            error: error.to_string(),
            exit_code: 1,
        },
    }
}

/// The answer write-back: RE-READ the file (a settings Apply between the
/// daemon's startup snapshot and the answer must survive), replace ONLY
/// `[telemetry]`, and persist through the migration-safe `Config::save`
/// (creating the config tree first - the dialog fires exactly on fresh
/// installs, where the directory may not exist yet). A missing/corrupt
/// file degrades to defaults-plus-the-answer with a warning: the choice
/// MUST be recorded or the dialog would nag on every start.
fn record_choice(path: &Path, choice: TelemetryConfig) {
    let mut config = Config::load(path).unwrap_or_else(|error| {
        tracing::warn!(%error, "consent write-back load failed; recording the answer on defaults");
        Config::default()
    });
    config.telemetry = choice;
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(%error, "consent write-back could not create the config directory");
    }
    if let Err(error) = config.save(path) {
        tracing::error!(%error, "consent answer persistence failed; the next daemon start asks again");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "flowshot-consent-test-{}-{nonce}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn answered() -> TelemetryConfig {
        TelemetryConfig {
            enabled: true,
            include_technical_details: true,
            asked_on_first_launch: true,
        }
    }

    #[test]
    fn the_spawn_gate_arms_only_when_wired_and_the_question_is_unanswered() {
        // Given: the two independent arming inputs - the binary's wiring
        // opt-in and the config's answer state (fresh = never asked).
        let fresh = TelemetryConfig::default();
        // Then: the full truth table (the daemon consults the
        // DaemonStartup surface; any recorded answer silences it, and an
        // unwired caller never spawns even on a fresh install).
        assert!(spawn_armed(true, &fresh));
        assert!(!spawn_armed(false, &fresh));
        assert!(!spawn_armed(true, &answered()));
        assert!(!spawn_armed(false, &answered()));
    }

    #[test]
    fn the_consent_spec_round_trips_the_session_wire_vocabulary() {
        // Given: the spec the parent writes for a detached prompt.
        let spec = consent_spec(
            Some(PathBuf::from("/tmp/consent-config.toml")),
            PathBuf::from("/tmp/consent-result.json"),
        );
        // When: serialized (the parent's write) and parsed back (the
        // child's read).
        let json = serde_json::to_string(&spec).unwrap();
        let parsed: SessionSpec = serde_json::from_str(&json).unwrap();
        // Then: the wire token is exactly "consent" and the job survives.
        assert!(
            json.contains("\"kind\":\"consent\""),
            "unexpected wire token: {json}"
        );
        assert!(matches!(parsed.kind, SessionKind::Consent));
        assert_eq!(parsed.config_path, spec.config_path);
        assert_eq!(parsed.result_path, spec.result_path);
        assert_eq!(parsed.image_path, None);
    }

    #[test]
    fn the_write_back_re_reads_the_file_and_preserves_the_other_groups() {
        // Given: a seeded config file, then a POST-SPAWN edit simulating
        // a settings Apply landing between startup and the answer.
        let dir = scratch_dir("rmw");
        let path = dir.join("flowshot.toml");
        let mut seeded = Config::default();
        seeded.save.path = "/before".to_owned();
        std::fs::write(&path, seeded.to_toml_string().unwrap()).unwrap();
        let mut edited = Config::default();
        edited.save.path = "/after".to_owned();
        std::fs::write(&path, edited.to_toml_string().unwrap()).unwrap();
        // When: the dialog records the answer.
        record_choice(&path, answered());
        // Then: the answer landed AND the post-spawn edit survived (a
        // startup-snapshot write would have clobbered "/after").
        let reloaded = Config::load(&path).unwrap();
        assert_eq!(reloaded.telemetry, answered());
        assert_eq!(reloaded.save.path, "/after");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_write_back_creates_the_config_tree_on_a_fresh_install() {
        // Given: the first-launch shape - no config directory at all.
        let dir = scratch_dir("fresh");
        let path = dir.join("config").join("flowshot").join("flowshot.toml");
        // When: the dialog records the saved answer (the ONLY writing
        // path - a dismissal records nothing and re-arms the prompt).
        let saved = flowshot_ui::consent::ConsentModel::default().saved_choice();
        record_choice(&path, saved);
        // Then: the tree was created and the answer recorded, so the
        // dialog never asks again.
        let reloaded = Config::load(&path).unwrap();
        assert_eq!(reloaded.telemetry, saved);
        assert!(reloaded.telemetry.asked_on_first_launch);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_write_back_records_the_answer_over_a_corrupt_file() {
        // Given: an unparsable config file (the resilience contract).
        let dir = scratch_dir("corrupt");
        let path = dir.join("flowshot.toml");
        std::fs::write(&path, "{{{ not toml at all").unwrap();
        // When: the dialog records the answer.
        record_choice(&path, answered());
        // Then: the file is valid again and the choice survived - a load
        // failure must never re-arm the prompt.
        let reloaded = Config::load(&path).unwrap();
        assert_eq!(reloaded.telemetry, answered());
        std::fs::remove_dir_all(&dir).ok();
    }
}

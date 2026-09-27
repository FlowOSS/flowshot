//! The settings-window session (plan todo 36/38, `DaemonCommand::Settings`
//! and CLI `flowshot settings`): the window runs in a session CHILD (the
//! winit one-loop constraint). The todo-36 binary-layer checklist is wired
//! in the child: `app_id=flowshot-settings`, the config path, the Apply
//! notification (the executor re-reads the config on every execution, so
//! Apply needs no bus signal - the recorded todo-36 decision), the
//! clipboard bridge, and the ashpd system-theme query. `rfd` is not a
//! workspace dependency, so the `PathPicker` stays unset (Browse disabled
//! - documented, todo-39 packaging revisit).

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::pins::WindowCustomizer;
use flowshot_ui::settings::{ClipboardBridge, SettingsWindow, SettingsWindowOptions, ThemeMode};
use winit::platform::wayland::WindowAttributesExtWayland;

use super::session::{self, SessionKind, SessionResult, SessionSpec};
use super::{ExecCtx, ExecOutcome, ExecuteError};

/// The parent side: spawns the settings child and waits for it to close.
///
/// # Errors
///
/// [`ExecuteError::Daemon`] when no config path resolves; session-spawn
/// failures per [`session::spawn`].
pub async fn run(ctx: &ExecCtx) -> Result<ExecOutcome, ExecuteError> {
    let config_path = match ctx.config_path.clone() {
        Some(path) => path,
        None => crate::paths::default_config_path()?,
    };
    let heartbeat = super::Heartbeat::start(ctx);
    let result = session::spawn(SessionSpec {
        kind: SessionKind::Settings,
        result_path: std::path::PathBuf::new(),
        image_path: None,
        config_path: Some(config_path),
        request: crate::request::CaptureRequest::default(),
        color_mode: false,
        forward_to_daemon: false,
        pin: None,
    })
    .await;
    drop(heartbeat);
    match result? {
        SessionResult::Closed | SessionResult::Cancelled => Ok(ExecOutcome::Done(
            flowshot_actions::clipboard::PostCaptureReport::default(),
        )),
        SessionResult::Failed { error, exit_code } => Err(ExecuteError::Child { error, exit_code }),
        other => Err(ExecuteError::Task(format!(
            "the settings child reported an unexpected result: {other:?}"
        ))),
    }
}

/// The child-side settings session (main thread).
#[must_use]
pub fn settings_child(spec: &SessionSpec) -> SessionResult {
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
    let options = SettingsWindowOptions {
        config_path,
        system_theme: query_system_theme(),
        tokens: DesignTokens::default(),
        path_picker: None,
        clipboard: Some(clipboard_bridge()),
        on_applied: Some(flowshot_ui::settings::AppliedCallback::new(|config| {
            // The executor re-reads the config on every execution (the
            // todo-36 "re-read on next capture" decision - no ConfigChanged
            // signal exists on the frozen todo-32 bus vocabulary), so the
            // callback only logs the projection.
            tracing::info!(
                accent = %config.ui.accent_color,
                tray = config.daemon.tray,
                notifications = config.daemon.notifications,
                "settings applied"
            );
        })),
        window_customizer: Some(WindowCustomizer::new(|attributes| {
            attributes.with_name("flowshot-settings", "FlowShot Settings")
        })),
    };
    let window = match SettingsWindow::new(options) {
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

/// The system dark/light preference via the ashpd Settings portal
/// (best-effort: no portal or no answer = the `ThemeMode` default).
#[must_use]
pub fn query_system_theme() -> ThemeMode {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let Ok(runtime) = runtime else {
        return ThemeMode::default();
    };
    runtime
        .block_on(async {
            let settings = ashpd::desktop::settings::Settings::new().await.ok()?;
            settings.color_scheme().await.ok()
        })
        .map_or(ThemeMode::default(), |scheme| match scheme {
            ashpd::desktop::settings::ColorScheme::PreferLight => ThemeMode::Light,
            _ => ThemeMode::Dark,
        })
}

/// The Ctrl+C/V bridge for the egui text fields: writes go through the
/// actions-crate clipboard; reads return `None` (a data-control READER is
/// not in the actions crate's offer-serving surface - paste stays
/// disabled, recorded in issues.md for the packaging wave).
#[must_use]
pub fn clipboard_bridge() -> ClipboardBridge {
    ClipboardBridge::new(
        || None,
        |text| {
            let clipboard = flowshot_actions::Clipboard::wayland();
            if let Err(error) = clipboard.copy_text(text) {
                tracing::warn!(%error, "settings clipboard write failed");
            }
        },
    )
}

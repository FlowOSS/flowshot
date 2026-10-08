//! Global shortcuts: the `GlobalShortcuts` portal primary
//! path plus the compositor-bind fallback ladder.
//!
//! # The ladder
//!
//! 1. [`PortalMode::Auto`] (default): register the configured actions
//!    through [`portal`] (`ashpd` `GlobalShortcuts`). On success the daemon
//!    holds the `shortcuts` persistence reason (portal triggers need a
//!    RESIDENT daemon - Oracle r4 F-3), persists the re-registration set
//!    ([`persist`]), and emits the ONE-TIME autostart-recommendation
//!    notification on the first successful registration.
//! 2. Portal absent/denied/masked (bare wlroots per F14/xdp-wlr#240, or
//!    the [`detect::PORTAL_OVERRIDE_ENV`] QA harness): generate the
//!    paste-ready compositor snippets ([`fallback`]) - NO resident daemon
//!    needed, so the persistence reason stays released. The same text
//!    feeds the settings tab, the docs, and
//!    `flowshot --print-bind-help`.
//!
//! # Shortcut defaults (F12, rebindable in the settings tab)
//!
//! `Print` -> region capture, `Shift+Print` -> full, `Ctrl+Print` ->
//! active monitor ([`spec::ACTIVE_SCREEN`] sentinel; the executor
//! resolves the output under cursor).
//!
//! # Restore data (pinned reality, see [`persist`])
//!
//! `ashpd` 0.13.13's `GlobalShortcuts` has NO restore token and exposes no
//! session handle, so a portal session cannot survive a restart; the
//! config-dir file persists the RE-REGISTRATION set plus the
//! notified-once flag ("restore-data file created + reused across daemon
//! restarts" - the reuse is identical re-binding without re-nagging).

pub mod detect;
pub mod fallback;
pub mod persist;
pub mod portal;
pub mod spec;

pub use detect::{PortalMode, desktop_from_env, detect_desktop};
pub use fallback::{CompositorFlavor, bind_help};
pub use persist::{PersistedShortcut, RestoreData};
pub use spec::{ACTIVE_SCREEN, ShortcutSpec, default_shortcuts};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use flowshot_capture::DesktopEnv;
use tokio::sync::Notify;
use tokio::task::{AbortHandle, JoinHandle};

use crate::command::CommandSink;
use crate::error::DaemonError;
use crate::notify::{NotificationRecord, Notifier};
use crate::state::DaemonState;

/// Budget for the portal registration sequence. A portal MAY show a
/// confirmation dialog (GNOME), so this bounds a human, not a round-trip;
/// on expiry the registration task is aborted and the ladder falls back.
pub const DEFAULT_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Budget for the listener task to finish its graceful session close
/// during shutdown before it is aborted.
const LISTENER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Everything the shortcut host borrows from the running daemon (grouped
/// so [`start`] stays a two-argument call).
#[derive(Debug, Clone)]
pub struct ShortcutWiring {
    /// Where triggered commands go.
    pub sink: Arc<dyn CommandSink>,
    /// Persistence reasons + activity stamp.
    pub state: Arc<DaemonState>,
    /// The one-time autostart nudge goes through the daemon's gated
    /// notifier (`[daemon].notifications` applies).
    pub notifier: Arc<dyn Notifier>,
}

/// Shortcut-host configuration (`DaemonOptions::shortcuts`; tests inject
/// every field, the binary uses [`ShortcutOptions::production`]).
#[derive(Debug, Clone)]
pub struct ShortcutOptions {
    /// Master switch (default OFF so existing daemon callers are
    /// unaffected; the binary enables it).
    pub enabled: bool,
    /// The actions to register.
    pub specs: Vec<ShortcutSpec>,
    /// `FlowShot` config dir override (`None` = XDG resolution); the
    /// restore-data file lives here.
    pub config_dir: Option<PathBuf>,
    /// Desktop override (`None` = environment detection).
    pub desktop: Option<DesktopEnv>,
    /// Portal rung control (the env-override QA harness lives in
    /// [`PortalMode::from_env`]).
    pub portal: PortalMode,
    /// Portal registration budget.
    pub registration_timeout: Duration,
}

impl Default for ShortcutOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            specs: default_shortcuts(),
            config_dir: None,
            desktop: None,
            portal: PortalMode::Auto,
            registration_timeout: DEFAULT_REGISTRATION_TIMEOUT,
        }
    }
}

impl ShortcutOptions {
    /// Production defaults: enabled, default specs, portal unless the
    /// override env masks it, desktop detected lazily at registration.
    #[must_use]
    pub fn production() -> Self {
        Self {
            enabled: true,
            portal: PortalMode::from_env(),
            ..Self::default()
        }
    }
}

/// What the fallback rung produced (settings/docs surface it verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackInfo {
    /// The detected desktop.
    pub desktop: DesktopEnv,
    /// The snippet dialect chosen for it.
    pub flavor: CompositorFlavor,
    /// The full paste-ready help text.
    pub help: String,
}

/// A live portal registration: the listener task plus its teardown gear.
#[derive(Debug)]
pub struct PortalRegistration {
    cancel: Arc<Notify>,
    listener: JoinHandle<()>,
    listener_abort: AbortHandle,
    state: Arc<DaemonState>,
    data: RestoreData,
}

impl PortalRegistration {
    /// The persisted registration record (the settings seam).
    #[must_use]
    pub const fn restore_data(&self) -> &RestoreData {
        &self.data
    }

    /// Stops the listener (graceful: cancel -> bounded wait -> abort
    /// backstop) and releases the persistence reason. Idempotent flag
    /// clear: the clean listener path already released it, this covers the
    /// panic/abort paths.
    pub async fn shutdown(self) {
        self.cancel.notify_one();
        match tokio::time::timeout(LISTENER_SHUTDOWN_TIMEOUT, self.listener).await {
            Ok(Ok(())) => {}
            Ok(Err(join_error)) => {
                tracing::warn!(%join_error, "the shortcuts listener panicked during shutdown");
            }
            Err(_elapsed) => {
                tracing::warn!("the shortcuts listener did not stop promptly; aborting");
                self.listener_abort.abort();
            }
        }
        self.state.set_shortcuts_registered(false);
    }
}

/// The outcome of the ladder.
#[derive(Debug)]
pub enum Registration {
    /// Portal registration succeeded; the daemon holds the `shortcuts`
    /// persistence reason until [`PortalRegistration::shutdown`].
    Portal(PortalRegistration),
    /// Portal absent/denied/masked: compositor snippets are the path.
    Fallback(FallbackInfo),
    /// Shortcuts are off (`ShortcutOptions::enabled == false`).
    Disabled,
}

impl Registration {
    /// Whether the portal rung succeeded.
    #[must_use]
    pub const fn is_portal(&self) -> bool {
        matches!(self, Self::Portal(_))
    }

    /// The fallback details when the ladder fell through.
    #[must_use]
    pub const fn fallback(&self) -> Option<&FallbackInfo> {
        match self {
            Self::Fallback(info) => Some(info),
            Self::Portal(_) | Self::Disabled => None,
        }
    }

    /// Tears down a portal registration (no-op for the other rungs).
    pub async fn shutdown(self) {
        if let Self::Portal(portal) = self {
            portal.shutdown().await;
        }
    }
}

/// Runs the ladder. Infallible by design: every failure degrades to the
/// next rung with a log line (the daemon must never die over shortcuts -
/// the autostart-sync resilience rule).
#[must_use]
pub async fn start(options: &ShortcutOptions, wiring: ShortcutWiring) -> Registration {
    if !options.enabled {
        return Registration::Disabled;
    }
    let desktop = options.desktop.unwrap_or_else(detect_desktop);
    let restore_path = restore_path(options);
    let prior = restore_path.as_deref().and_then(RestoreData::load);

    if options.portal == PortalMode::Auto {
        match register_portal(options, &wiring).await {
            Ok(registration) => {
                if first_registration(prior.as_ref()) {
                    wiring
                        .notifier
                        .notify(NotificationRecord::ShortcutsRegistered);
                }
                if let Some(path) = &restore_path
                    && let Err(error) = registration.restore_data().save(path)
                {
                    tracing::warn!(%error, "could not persist the shortcut restore data (continuing)");
                }
                wiring.state.set_shortcuts_registered(true);
                tracing::info!(
                    count = registration.restore_data().shortcuts.len(),
                    "global shortcuts registered via the portal; the daemon now persists for them"
                );
                return Registration::Portal(registration);
            }
            Err(error) => tracing::warn!(
                %error,
                desktop = ?desktop,
                "GlobalShortcuts portal registration failed; falling back to compositor binds"
            ),
        }
    } else {
        tracing::info!(
            desktop = ?desktop,
            "portal shortcuts masked by override; using the compositor-bind fallback"
        );
    }

    let flavor = CompositorFlavor::for_desktop(desktop);
    let help = bind_help(flavor, &options.specs);
    tracing::info!(
        flavor = flavor.label(),
        "compositor-bind fallback active; `flowshot --print-bind-help` prints the paste-ready snippets"
    );
    Registration::Fallback(FallbackInfo {
        desktop,
        flavor,
        help,
    })
}

/// The portal rung: registration runs inside `tokio::spawn` (panic
/// containment for ashpd's internal `assert_eq!` - module pins) under the
/// registration budget; success spawns the `Activated` listener.
async fn register_portal(
    options: &ShortcutOptions,
    wiring: &ShortcutWiring,
) -> Result<PortalRegistration, DaemonError> {
    let specs = options.specs.clone();
    let task = tokio::spawn({
        let specs = specs.clone();
        async move { portal::register(&specs).await }
    });
    let task_abort = task.abort_handle();
    let parts = match tokio::time::timeout(options.registration_timeout, task).await {
        Ok(Ok(Ok(parts))) => parts,
        Ok(Ok(Err(error))) => return Err(error),
        Ok(Err(join_error)) => {
            return Err(DaemonError::ShortcutPortal(format!(
                "the registration task panicked: {join_error}"
            )));
        }
        Err(_elapsed) => {
            task_abort.abort();
            return Err(DaemonError::ShortcutPortal(format!(
                "no portal response within {}s (a confirmation dialog may be waiting)",
                options.registration_timeout.as_secs()
            )));
        }
    };
    let data = RestoreData::from_bound(&specs, &parts.bound, true, SystemTime::now());
    let ctx = portal::ListenerContext {
        commands: spec::command_map(&specs),
        sink: Arc::clone(&wiring.sink),
        state: Arc::clone(&wiring.state),
        cancel: Arc::new(Notify::new()),
    };
    let cancel = Arc::clone(&ctx.cancel);
    let listener = tokio::spawn(portal::listen(parts, ctx));
    Ok(PortalRegistration {
        cancel,
        listener_abort: listener.abort_handle(),
        listener,
        state: Arc::clone(&wiring.state),
        data,
    })
}

/// The pure one-time-notification decision: notify unless a prior restore
/// record says the nudge already happened.
fn first_registration(prior: Option<&RestoreData>) -> bool {
    prior.is_none_or(|data| !data.first_registration_notified)
}

fn restore_path(options: &ShortcutOptions) -> Option<PathBuf> {
    if let Some(dir) = &options.config_dir {
        return Some(restore_path_in(dir));
    }
    match crate::paths::default_shortcuts_restore_path() {
        Ok(path) => Some(path),
        Err(error) => {
            tracing::warn!(%error, "cannot resolve the config home; shortcut restore data will not persist");
            None
        }
    }
}

/// The restore-data path inside a `FlowShot` config dir (the
/// settings seam; the XDG-home twin lives in [`crate::paths`]).
#[must_use]
pub fn restore_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join(crate::paths::SHORTCUTS_RESTORE_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::RecordingSink;
    use crate::notify::RecordingNotifier;
    use std::time::Instant;

    fn wiring() -> (
        ShortcutWiring,
        RecordingSink,
        RecordingNotifier,
        Arc<DaemonState>,
    ) {
        let sink = RecordingSink::new();
        let notifier = RecordingNotifier::new();
        let state = Arc::new(DaemonState::new(false, Instant::now()));
        (
            ShortcutWiring {
                sink: Arc::new(sink.clone()),
                state: Arc::clone(&state),
                notifier: Arc::new(notifier.clone()),
            },
            sink,
            notifier,
            state,
        )
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("flowshot-shortcuts-{}-{tag}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("{error}"));
        dir
    }

    #[tokio::test]
    async fn disabled_options_register_nothing() {
        let (wiring, _sink, _notifier, state) = wiring();
        let registration = start(&ShortcutOptions::default(), wiring).await;
        assert!(matches!(registration, Registration::Disabled));
        assert!(!state.reasons().shortcuts);
    }

    #[tokio::test]
    async fn masked_portal_falls_back_to_the_desktop_dialect() {
        let (wiring, _sink, notifier, state) = wiring();
        let dir = temp_dir("masked");
        let options = ShortcutOptions {
            enabled: true,
            desktop: Some(DesktopEnv::Hyprland),
            portal: PortalMode::Disabled,
            config_dir: Some(dir.clone()),
            ..ShortcutOptions::default()
        };
        let registration = start(&options, wiring).await;
        let Registration::Fallback(info) = registration else {
            panic!("a masked portal must fall back");
        };
        assert_eq!(info.flavor, CompositorFlavor::Hyprland);
        assert_eq!(info.desktop, DesktopEnv::Hyprland);
        // The verbatim hyprland snippet form is in the help text.
        assert!(info.help.contains("bind = ,Print,exec,flowshot capture"));
        // The fallback needs NO resident daemon: the reason stays released,
        // nothing persists, and no nudge fires.
        assert!(!state.reasons().shortcuts);
        assert!(!restore_path_in(&dir).exists());
        assert_eq!(
            notifier.records(),
            [] as [crate::notify::NotificationRecord; 0]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn fallback_dialect_follows_the_desktop_table() {
        for (desktop, flavor) in [
            (DesktopEnv::Sway, CompositorFlavor::Sway),
            (DesktopEnv::Gnome, CompositorFlavor::Gnome),
            (DesktopEnv::Kde, CompositorFlavor::Kde),
            (DesktopEnv::Other, CompositorFlavor::Generic),
        ] {
            let (wiring, _sink, _notifier, _state) = wiring();
            let options = ShortcutOptions {
                enabled: true,
                desktop: Some(desktop),
                portal: PortalMode::Disabled,
                ..ShortcutOptions::default()
            };
            let registration = start(&options, wiring).await;
            let Registration::Fallback(info) = registration else {
                panic!("desktop {desktop:?} must fall back");
            };
            assert_eq!(info.flavor, flavor, "desktop {desktop:?}");
        }
    }

    #[test]
    fn one_time_notification_decision() {
        assert!(first_registration(None));
        let mut data = RestoreData::default();
        assert!(first_registration(Some(&data)));
        data.first_registration_notified = true;
        assert!(!first_registration(Some(&data)));
    }

    #[test]
    fn production_options_are_enabled_with_default_specs() {
        let options = ShortcutOptions::production();
        assert!(options.enabled);
        assert_eq!(options.specs, default_shortcuts());
        assert_eq!(options.registration_timeout, DEFAULT_REGISTRATION_TIMEOUT);
        // The portal rung follows the env override (pure decision table-
        // tested in detect.rs; the env itself is never mutated in tests).
        assert_eq!(options.portal, PortalMode::from_env());
    }
}

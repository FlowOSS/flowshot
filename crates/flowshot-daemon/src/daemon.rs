//! Daemon composition: bus connection, object registration, atomic name
//! acquisition, the lifecycle select loop, and deterministic teardown.
//!
//! Startup order matters (zbus caveat): the object server is registered
//! BEFORE the name is requested, so the winner can serve immediately -
//! the race-free single-instance property the design mandates (Oracle r4).
//! Teardown calls `Connection::close()` explicitly: zbus-4 with the
//! async-io reactor has NO drop-time close.

use std::sync::Arc;
use std::time::Duration;

use flowshot_core::Config;
use tokio::sync::Notify;
use zbus::Connection;

use crate::autostart::Autostart;
use crate::bus::{FlowShotInterface, OBJECT_PATH, SERVICE};
use crate::command::{CommandSink, LoggingSink};
use crate::error::DaemonError;
use crate::instance::{self, close_quietly};
use crate::lifecycle::{Clock, DaemonMode, LifecycleMonitor, LifecyclePolicy, TokioClock};
use crate::notify::{DesktopNotifier, GatedNotifier, Notifier};
use crate::shortcut::{Registration, ShortcutOptions, ShortcutWiring};
use crate::state::DaemonState;
use crate::tray::{TrayHandle, TrayOptions, TrayWiring};

/// Everything [`Daemon::start`] needs (grouped options object; tests and
/// the binary override individual fields).
#[derive(Debug)]
pub struct DaemonOptions {
    /// Supervised (always persists) or auto-spawned (smart idle exit).
    pub mode: DaemonMode,
    /// Idle grace for the auto-spawned mode.
    pub idle_grace: Duration,
    /// Loaded configuration (`[daemon]` group drives tray/notifications/
    /// `startup_launch`).
    pub config: Config,
    /// Shared state; handed out so the tray/shortcut/pin hosts can update
    /// persistence reasons on the live daemon.
    pub state: Arc<DaemonState>,
    /// `None` = session bus; `Some(address)` = a private bus (tests/QA).
    pub bus_address: Option<String>,
    /// `Exec=` line for the autostart entry; `None` skips autostart
    /// management (the caller owns it).
    pub autostart_exec: Option<String>,
    /// Command dispatch seam; defaults to [`LoggingSink`] (the CLI plugs
    /// the executing sink).
    pub command_sink: Option<Arc<dyn CommandSink>>,
    /// Notification seam; defaults to the config-gated desktop notifier.
    pub notifier: Option<Arc<dyn Notifier>>,
    /// Time source; defaults to [`TokioClock`] (tests inject accelerated
    /// clocks).
    pub clock: Option<Arc<dyn Clock>>,
    /// Global-shortcut host configuration. Defaults to DISABLED
    /// so callers that do not opt in keep their exact startup behavior; the
    /// binary enables it via [`ShortcutOptions::production`].
    pub shortcuts: ShortcutOptions,
    /// Tray host configuration. Seeded from `[daemon].tray` by the
    /// constructors; the tray module owns the `tray` persistence reason
    /// from registration onward.
    pub tray: TrayOptions,
}

impl DaemonOptions {
    /// Supervised foreground options (`flowshot daemon` / the systemd
    /// user unit): always persists, init-agnostic.
    #[must_use]
    pub fn supervised(config: Config) -> Self {
        Self::new(DaemonMode::Supervised, config)
    }

    /// Auto-spawned helper options: exits after the idle grace when no
    /// persistence reason holds.
    #[must_use]
    pub fn auto_spawned(config: Config) -> Self {
        Self::new(DaemonMode::AutoSpawned, config)
    }

    fn new(mode: DaemonMode, config: Config) -> Self {
        // The tray flag is seeded FALSE: the tray module sets it
        // only after a successful StatusNotifierWatcher registration (an
        // enabled config without a host on the bus must not pin the
        // daemon).
        let state = Arc::new(DaemonState::new(false, TokioClock.now()));
        let tray = TrayOptions::from_config(&config);
        Self {
            mode,
            idle_grace: crate::DEFAULT_IDLE_GRACE,
            config,
            state,
            bus_address: None,
            autostart_exec: None,
            command_sink: None,
            notifier: None,
            clock: None,
            shortcuts: ShortcutOptions::default(),
            tray,
        }
    }
}

/// What startup produced.
#[derive(Debug)]
pub enum Startup {
    /// This process owns the bus name and can serve.
    Running(Daemon),
    /// Another daemon already holds the name; nothing was registered.
    AlreadyRunning,
}

/// Why [`Daemon::run`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownReason {
    /// The auto-spawned idle grace elapsed with no persistence reason.
    IdleExit,
    /// The tray menu's `Quit` entry fired.
    Quit,
    /// `SIGINT` (Ctrl-C).
    Interrupted,
    /// `SIGTERM` (the supervisor stopped the unit).
    Terminated,
}

/// A running daemon: the name-holding connection plus the lifecycle
/// monitor.
#[derive(Debug)]
pub struct Daemon {
    connection: Connection,
    monitor: LifecycleMonitor,
    notifier: Arc<dyn Notifier>,
    state: Arc<DaemonState>,
    shortcuts: Registration,
    tray: TrayHandle,
    quit: Arc<Notify>,
}

impl Daemon {
    /// Connects, registers the object, and atomically acquires
    /// `org.flowoss.FlowShot`.
    ///
    /// # Errors
    ///
    /// [`DaemonError::Dbus`] on connection/registration/name failure.
    pub async fn start(options: DaemonOptions) -> Result<Startup, DaemonError> {
        let clock = options.clock.unwrap_or_else(|| Arc::new(TokioClock));
        let sink = options
            .command_sink
            .unwrap_or_else(|| Arc::new(LoggingSink) as Arc<dyn CommandSink>);
        let notifier = options.notifier.unwrap_or_else(|| {
            Arc::new(GatedNotifier::new(
                Arc::new(DesktopNotifier::portal()),
                options.config.daemon.notifications,
            )) as Arc<dyn Notifier>
        });
        let quit = Arc::new(Notify::new());
        let connection = instance::connect(options.bus_address.as_deref()).await?;
        let interface = FlowShotInterface::new(
            Arc::clone(&sink),
            Arc::clone(&options.state),
            Arc::clone(&clock),
        );
        connection
            .object_server()
            .at(OBJECT_PATH, interface)
            .await?;

        if instance::request_ownership(&connection, SERVICE).await?
            == instance::Ownership::HeldByOther
        {
            close_quietly(connection).await;
            return Ok(Startup::AlreadyRunning);
        }
        tracing::info!(service = SERVICE, "bus name acquired; serving");

        if let Some(exec) = &options.autostart_exec {
            sync_autostart(options.config.daemon.startup_launch, exec);
        }
        report_ready_to_supervisor();

        // The shortcut ladder runs AFTER readiness reporting - a
        // portal confirmation dialog may hold registration for up to its
        // budget while the bus service is already serving.
        let shortcuts = crate::shortcut::start(
            &options.shortcuts,
            ShortcutWiring {
                sink: Arc::clone(&sink),
                state: Arc::clone(&options.state),
                notifier: Arc::clone(&notifier),
            },
        )
        .await;

        // The tray host shares the daemon's connection; watcher
        // registration is async inside its own task, so start() is cheap.
        let tray = crate::tray::start(
            &options.tray,
            TrayWiring {
                connection: connection.clone(),
                sink,
                state: Arc::clone(&options.state),
                notifier: Arc::clone(&notifier),
                clock: Arc::clone(&clock),
                quit: Arc::clone(&quit),
                config: options.config.clone(),
            },
        )
        .await;

        let monitor = LifecycleMonitor::new(
            LifecyclePolicy {
                mode: options.mode,
                idle_grace: options.idle_grace,
            },
            Arc::clone(&options.state),
            clock,
        );
        Ok(Startup::Running(Self {
            connection,
            monitor,
            notifier,
            state: options.state,
            shortcuts,
            tray,
            quit,
        }))
    }

    /// Serves until the lifecycle exits (auto-spawned idle) or a shutdown
    /// signal arrives, then closes the connection deterministically.
    ///
    /// # Errors
    ///
    /// Never in practice; the `Result` keeps teardown fallible-shaped for
    /// future close diagnostics.
    pub async fn run(self) -> Result<ShutdownReason, DaemonError> {
        let Self {
            connection,
            monitor,
            notifier: _notifier,
            state: _state,
            shortcuts,
            tray,
            quit,
        } = self;
        let reason = tokio::select! {
            _idle = monitor.run_until_exit() => ShutdownReason::IdleExit,
            signal = shutdown_signal() => signal,
            () = quit.notified() => ShutdownReason::Quit,
        };
        tracing::info!(reason = ?reason, "daemon shutting down");
        tray.shutdown().await;
        shortcuts.shutdown().await;
        close_quietly(connection).await;
        Ok(reason)
    }

    /// The notification seam (the tray and CLI dispatch toasts through it).
    #[must_use]
    pub fn notifier(&self) -> Arc<dyn Notifier> {
        Arc::clone(&self.notifier)
    }

    /// The shared state (persistence reasons, pin registry).
    #[must_use]
    pub fn state(&self) -> Arc<DaemonState> {
        Arc::clone(&self.state)
    }
}

fn sync_autostart(startup_launch: bool, exec: &str) {
    let sync = Autostart::from_env().and_then(|autostart| autostart.sync(startup_launch, exec));
    if let Err(error) = sync {
        tracing::warn!(%error, "autostart entry sync failed (continuing)");
    }
}

#[cfg(feature = "systemd")]
fn report_ready_to_supervisor() {
    match crate::systemd::notify_ready() {
        Ok(crate::systemd::NotifyOutcome::Sent) => {
            tracing::info!("sd_notify READY=1 sent");
        }
        Ok(crate::systemd::NotifyOutcome::NoSocket) => {
            tracing::debug!("NOTIFY_SOCKET unset; skipping sd_notify");
        }
        Ok(crate::systemd::NotifyOutcome::AbstractUnsupported) => {
            tracing::warn!("NOTIFY_SOCKET is abstract-namespace; readiness not reported");
        }
        Err(error) => tracing::warn!(%error, "sd_notify failed (continuing)"),
    }
}

#[cfg(not(feature = "systemd"))]
fn report_ready_to_supervisor() {}

#[cfg(unix)]
async fn shutdown_signal() -> ShutdownReason {
    use tokio::signal::unix::{SignalKind, signal};
    let Ok(mut terminate) = signal(SignalKind::terminate()) else {
        tracing::warn!("could not install the SIGTERM handler; only SIGINT will shut down");
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(%error, "the SIGINT handler failed too; the daemon can only be killed");
            std::future::pending::<()>().await;
        }
        return ShutdownReason::Interrupted;
    };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::warn!(%error, "the SIGINT handler failed; waiting for SIGTERM");
                terminate.recv().await;
                return ShutdownReason::Terminated;
            }
            ShutdownReason::Interrupted
        }
        _ = terminate.recv() => ShutdownReason::Terminated,
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> ShutdownReason {
    let _ = tokio::signal::ctrl_c().await;
    ShutdownReason::Interrupted
}

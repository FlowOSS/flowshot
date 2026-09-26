//! SNI tray host (plan todo 33): the `org.kde.StatusNotifierItem` surface
//! on the daemon's existing zbus-4 connection.
//!
//! # Pieces
//!
//! - [`spec`] - the SNI + `com.canonical.dbusmenu` wire vocabulary;
//! - [`icon`] - the procedural ARGB32 tray glyph (idle/attention sets from
//!   the brand accent token);
//! - [`menu`] - the F12-parity menu model and the pure id -> action
//!   dispatch table;
//! - [`outputs`] - the live output probe behind the per-monitor submenu
//!   (todo 6 `CaptureThread`, headless-safe);
//! - `item` / `dbusmenu` - the two registered bus objects;
//! - `watcher` - `StatusNotifierWatcher` registration with the
//!   absent-watcher degrade + late-arrival retry.
//!
//! # Lifecycle coupling (todo 32)
//!
//! The tray persistence reason is owned HERE, not by the config seed:
//! `state.set_tray(true)` happens only after a successful
//! `RegisterStatusNotifierItem`, and a vanished watcher releases it - an
//! auto-spawned daemon with `[daemon].tray = true` but no tray host on
//! the bus still idle-exits (task contract: "tray registered = lifecycle
//! persistence reason").
//!
//! # Deviations (recorded)
//!
//! - The plan cited `ksni`; no SNI crate exists in the workspace table or
//!   the lock and the root manifest is orchestrator-owned, so the protocol
//!   is hand-rolled on zbus 4 (task-brief sanctioned, ksni wire shapes as
//!   the reference).
//! - `About` is a version toast until the todo-36 settings stack provides
//!   a real surface.

pub mod icon;
pub mod menu;
pub mod outputs;
pub mod spec;

mod dbusmenu;
mod item;
mod shared;
mod watcher;

pub use outputs::{OutputProbe, WaylandOutputProbe};

use std::sync::Arc;

use flowshot_core::Config;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use zbus::Connection;

use crate::command::CommandSink;
use crate::lifecycle::Clock;
use crate::notify::Notifier;
use crate::state::DaemonState;
use dbusmenu::DbusMenu;
use item::StatusNotifierItem;
use shared::TrayCore;
use spec::{ITEM_PATH, MENU_PATH, TrayStatus};

/// Tray-host configuration (`DaemonOptions::tray`; seeded from config via
/// [`TrayOptions::from_config`], tests inject the probe).
#[derive(Debug, Clone)]
pub struct TrayOptions {
    /// Master switch - the `[daemon].tray` gate (default false = lean
    /// on-demand daemon, Amendment #3).
    pub enabled: bool,
    /// `#RRGGBB` accent for the procedural glyph (`[ui].accent_color`).
    pub accent_color: String,
    /// Output-probe override (`None` = [`WaylandOutputProbe`]).
    pub probe: Option<Arc<dyn OutputProbe>>,
}

impl Default for TrayOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            accent_color: flowshot_core::tokens::Palette::default().accent,
            probe: None,
        }
    }
}

impl TrayOptions {
    /// The production gate: `[daemon].tray` plus the `[ui]` accent token.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self {
            enabled: config.daemon.tray,
            accent_color: config.ui.accent_color.clone(),
            probe: None,
        }
    }
}

/// Everything the tray host borrows from the running daemon.
#[derive(Debug)]
pub struct TrayWiring {
    /// The daemon's bus connection (the SNI objects live on it).
    pub connection: Connection,
    /// Where menu commands go.
    pub sink: Arc<dyn CommandSink>,
    /// Persistence reasons + activity stamp.
    pub state: Arc<DaemonState>,
    /// The About toast goes through the daemon's gated notifier.
    pub notifier: Arc<dyn Notifier>,
    /// Activity-stamp time source.
    pub clock: Arc<dyn Clock>,
    /// Fired by the menu's `Quit` entry (the daemon's run loop exits).
    pub quit: Arc<Notify>,
}

/// A started (or deliberately inert) tray host. [`TrayHandle::shutdown`]
/// is the deterministic teardown the daemon's run loop calls.
#[derive(Debug)]
pub struct TrayHandle {
    connection: Connection,
    state: Arc<DaemonState>,
    core: Option<Arc<TrayCore>>,
    watcher_task: Option<tokio::task::AbortHandle>,
}

impl TrayHandle {
    fn inactive(connection: Connection, state: Arc<DaemonState>) -> Self {
        Self {
            connection,
            state,
            core: None,
            watcher_task: None,
        }
    }

    /// Whether the SNI + menu objects are on the bus (the watcher
    /// registration itself is async and reflected in the lifecycle's
    /// `tray` persistence reason).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.core.is_some()
    }

    /// The attention-state seam: swaps the `Status` property and emits
    /// `NewStatus`. Nothing drives it in v1 (capture-failure UX lands with
    /// the todo-36/38 surfaces); QA and tests exercise it.
    pub fn set_status(&self, status: TrayStatus) {
        if let Some(core) = &self.core {
            Arc::clone(core).set_status(status);
        }
    }

    /// Aborts the watcher task, removes the bus objects, and releases the
    /// tray persistence reason.
    pub async fn shutdown(self) {
        if let Some(task) = self.watcher_task {
            task.abort();
        }
        if self.core.is_some() {
            if let Err(error) = self
                .connection
                .object_server()
                .remove::<StatusNotifierItem, _>(ITEM_PATH)
                .await
            {
                tracing::warn!(%error, "could not remove the tray item object");
            }
            if let Err(error) = self
                .connection
                .object_server()
                .remove::<DbusMenu, _>(MENU_PATH)
                .await
            {
                tracing::warn!(%error, "could not remove the tray menu object");
            }
        }
        self.state.set_tray(false);
    }
}

/// Starts the tray host: registers the SNI + menu objects, kicks the
/// initial output probe, and spawns the watcher-registration task.
/// Infallible by design - every failure degrades to an inert handle with
/// a warn log (plan: "absent SNI host -> zero daemon impact").
pub async fn start(options: &TrayOptions, wiring: TrayWiring) -> TrayHandle {
    let connection = wiring.connection.clone();
    let state = Arc::clone(&wiring.state);
    if !options.enabled {
        state.set_tray(false);
        tracing::info!("tray disabled by the [daemon].tray config");
        return TrayHandle::inactive(connection, state);
    }
    let Ok(runtime) = Handle::try_current() else {
        tracing::warn!("no tokio runtime context; tray disabled");
        state.set_tray(false);
        return TrayHandle::inactive(connection, state);
    };
    let core = Arc::new(TrayCore::new(
        wiring,
        icon::icon_set(&options.accent_color),
        options
            .probe
            .clone()
            .unwrap_or_else(|| Arc::new(WaylandOutputProbe)),
        runtime,
    ));
    // The object-server borrow must END before the error arms move
    // `connection` into the inert handle (scrutinee temporaries of an
    // `if let` live through its body).
    let item_registered = connection
        .object_server()
        .at(ITEM_PATH, StatusNotifierItem::new(Arc::clone(&core)))
        .await;
    if let Err(error) = item_registered {
        tracing::warn!(%error, "could not register the StatusNotifierItem object; tray disabled");
        state.set_tray(false);
        return TrayHandle::inactive(connection, state);
    }
    let menu_registered = connection
        .object_server()
        .at(MENU_PATH, DbusMenu::new(Arc::clone(&core)))
        .await;
    if let Err(error) = menu_registered {
        tracing::warn!(%error, "could not register the dbusmenu object; tray disabled");
        let rolled_back = connection
            .object_server()
            .remove::<StatusNotifierItem, _>(ITEM_PATH)
            .await;
        if let Err(remove_error) = rolled_back {
            tracing::warn!(%remove_error, "could not roll back the tray item object");
        }
        state.set_tray(false);
        return TrayHandle::inactive(connection, state);
    }
    core.refresh_outputs();
    let task = tokio::spawn(watcher::registration_task(
        connection.clone(),
        Arc::clone(&state),
    ));
    tracing::info!(item = ITEM_PATH, menu = MENU_PATH, "tray host started");
    TrayHandle {
        connection,
        state,
        core: Some(core),
        watcher_task: Some(task.abort_handle()),
    }
}

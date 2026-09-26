//! Shared tray state behind both bus objects: the icon sets, the cached
//! output probe, the menu revision, and the side-effect router every
//! activation runs through.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use flowshot_core::geometry::OutputInfo;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use zbus::Connection;

use super::TrayWiring;
use super::icon::IconSet;
use super::menu::{
    MenuNode, ROOT_ID, SCREEN_SUBMENU_ID, TrayAction, action_for, build_menu, find_node,
};
use super::outputs::OutputProbe;
use super::spec::{ITEM_INTERFACE, ITEM_PATH, MENU_INTERFACE, MENU_PATH, TrayStatus};
use crate::command::CommandSink;
use crate::lifecycle::Clock;
use crate::notify::{NotificationRecord, Notifier};
use crate::state::DaemonState;
use crate::strings;

/// Shared tray state behind both bus objects.
#[derive(Debug)]
pub(super) struct TrayCore {
    connection: Connection,
    sink: Arc<dyn CommandSink>,
    state: Arc<DaemonState>,
    notifier: Arc<dyn Notifier>,
    clock: Arc<dyn Clock>,
    quit: Arc<Notify>,
    icons: IconSet,
    probe: Arc<dyn OutputProbe>,
    outputs: Mutex<Vec<OutputInfo>>,
    revision: AtomicU32,
    status: Mutex<TrayStatus>,
    runtime: Handle,
    refreshing: AtomicBool,
}

impl TrayCore {
    /// Groups the borrowed daemon pieces (`wiring`) with the tray-owned
    /// assets; four independent environment facts (see Smell-2 note: the
    /// wiring object already groups six).
    pub(super) fn new(
        wiring: TrayWiring,
        icons: IconSet,
        probe: Arc<dyn OutputProbe>,
        runtime: Handle,
    ) -> Self {
        let TrayWiring {
            connection,
            sink,
            state,
            notifier,
            clock,
            quit,
        } = wiring;
        Self {
            connection,
            sink,
            state,
            notifier,
            clock,
            quit,
            icons,
            probe,
            outputs: Mutex::new(Vec::new()),
            revision: AtomicU32::new(0),
            status: Mutex::new(TrayStatus::default()),
            runtime,
            refreshing: AtomicBool::new(false),
        }
    }

    pub(super) fn icons(&self) -> &IconSet {
        &self.icons
    }

    pub(super) fn status(&self) -> TrayStatus {
        *self.lock_status()
    }

    /// The attention-state seam: swaps `Status` and emits `NewStatus`
    /// (nothing drives it in v1; capture-failure UX lands with todo 36/38).
    pub(super) fn set_status(self: &Arc<Self>, status: TrayStatus) {
        *self.lock_status() = status;
        let core = Arc::clone(self);
        self.runtime.spawn(async move {
            if let Err(error) = core
                .connection
                .emit_signal(
                    None::<&str>,
                    ITEM_PATH,
                    ITEM_INTERFACE,
                    "NewStatus",
                    &status.as_str(),
                )
                .await
            {
                tracing::warn!(%error, "NewStatus emission failed");
            }
        });
    }

    pub(super) fn revision(&self) -> u32 {
        self.revision.load(Ordering::Acquire)
    }

    pub(super) fn menu_nodes(&self) -> Vec<MenuNode> {
        build_menu(&self.lock_outputs())
    }

    pub(super) fn has_id(&self, id: i32) -> bool {
        id == ROOT_ID || find_node(&self.menu_nodes(), id).is_some()
    }

    /// Runs one menu id through the dispatch table (the todo-33
    /// [`CommandSink`] seam).
    pub(super) fn dispatch(&self, id: i32) {
        match action_for(id) {
            TrayAction::Command(command) => {
                self.state.touch(self.clock.now());
                tracing::info!(command = %command, "tray command accepted");
                self.sink.dispatch(command);
            }
            TrayAction::About => {
                self.notifier
                    .notify(NotificationRecord::About(strings::about_body(env!(
                        "CARGO_PKG_VERSION"
                    ))));
            }
            TrayAction::Quit => {
                tracing::info!("tray quit requested; the daemon is shutting down");
                self.quit.notify_one();
            }
            TrayAction::None => tracing::debug!(id, "tray menu id carries no action"),
        }
    }

    /// Probes outputs in the background (deduplicated), bumps the menu
    /// revision and emits `LayoutUpdated` for the per-monitor submenu.
    pub(super) fn refresh_outputs(self: &Arc<Self>) {
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let core = Arc::clone(self);
        self.runtime.spawn(async move {
            let probe = Arc::clone(&core.probe);
            let outputs = tokio::task::spawn_blocking(move || probe.probe())
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "the output probe task failed; the submenu stays empty");
                    Vec::new()
                });
            let count = outputs.len();
            *core.lock_outputs() = outputs;
            let revision = core.revision.fetch_add(1, Ordering::AcqRel) + 1;
            core.refreshing.store(false, Ordering::Release);
            tracing::debug!(outputs = count, revision, "tray output cache refreshed");
            if let Err(error) = core
                .connection
                .emit_signal(
                    None::<&str>,
                    MENU_PATH,
                    MENU_INTERFACE,
                    "LayoutUpdated",
                    &(revision, SCREEN_SUBMENU_ID),
                )
                .await
            {
                tracing::warn!(%error, "LayoutUpdated emission failed");
            }
        });
    }

    fn lock_outputs(&self) -> MutexGuard<'_, Vec<OutputInfo>> {
        self.outputs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_status(&self) -> MutexGuard<'_, TrayStatus> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

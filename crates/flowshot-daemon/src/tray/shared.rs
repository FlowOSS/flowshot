//! Shared tray state behind both bus objects: the icon sets, the cached
//! output probe, the menu revision, and the side-effect router every
//! activation runs through.

use std::path::PathBuf;
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
    config: flowshot_core::config::Config,
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
            config,
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
            config,
        }
    }

    pub(super) fn icons(&self) -> &IconSet {
        &self.icons
    }

    pub(super) fn status(&self) -> TrayStatus {
        *self.lock_status()
    }

    /// The attention-state seam: swaps `Status` and emits `NewStatus`
    /// (nothing drives it in v1; capture-failure UX lands with the
    /// settings/executor surfaces).
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

    /// Runs one menu id through the dispatch table (the
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
            TrayAction::OpenSavePath => {
                self.state.touch(self.clock.now());
                tracing::info!("tray Open Save Path requested");
                self.open_save_path();
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

    /// Opens the configured save path (or platform pictures dir) via the
    /// `OpenURI` portal. Spawns asynchronously; errors become notifications.
    fn open_save_path(&self) {
        let save_path = self.config.save.path.clone();
        let notifier = Arc::clone(&self.notifier);
        self.runtime.spawn(async move {
            let path = if save_path.is_empty() {
                // Platform pictures directory (XDG user-dirs fallback)
                match pictures_dir() {
                    Ok(dir) => dir,
                    Err(error) => {
                        tracing::warn!(%error, "failed to resolve pictures directory");
                        notifier.notify(NotificationRecord::Error(
                            "Failed to open save path: could not resolve pictures directory"
                                .to_owned(),
                        ));
                        return;
                    }
                }
            } else {
                PathBuf::from(save_path)
            };

            // Open via OpenURI portal
            match open_uri_portal(&path).await {
                Ok(()) => tracing::debug!(?path, "opened save path via portal"),
                Err(error) => {
                    tracing::warn!(%error, ?path, "failed to open save path via portal");
                    notifier.notify(NotificationRecord::Error(format!(
                        "Failed to open save path: {error}"
                    )));
                }
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

fn pictures_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME not set".to_owned())?;
    let user_dirs = home.join(".config").join("user-dirs.dirs");
    if let Ok(contents) = std::fs::read_to_string(&user_dirs) {
        for line in contents.lines() {
            let Some(value) = line.strip_prefix("XDG_PICTURES_DIR=") else {
                continue;
            };
            let trimmed = value.trim_matches('"');
            let expanded = trimmed.replace("$HOME", &home.to_string_lossy());
            return Ok(PathBuf::from(expanded));
        }
    }
    Ok(home.join("Pictures"))
}

/// Opens a directory via the XDG `OpenURI` portal.
async fn open_uri_portal(path: &PathBuf) -> Result<(), String> {
    // Verify it's a directory
    let metadata = std::fs::metadata(path).map_err(|e| format!("Failed to access path: {e}"))?;
    if !metadata.is_dir() {
        return Err("Path is not a directory".to_owned());
    }

    // Convert to file:// URI
    let uri = format!(
        "file://{}",
        path.canonicalize()
            .map_err(|e| format!("Failed to canonicalize path: {e}"))?
            .display()
    );

    // Parse URI (Url percent-encodes the raw path, e.g. spaces), then wrap
    // in ashpd 0.13's own Uri type the portal call consumes.
    let url = url::Url::parse(&uri).map_err(|e| format!("Invalid URI {uri}: {e}"))?;
    let portal_uri =
        ashpd::Uri::parse(url.as_str()).map_err(|e| format!("Invalid URI {uri}: {e}"))?;

    // Open via portal
    ashpd::desktop::open_uri::OpenFileRequest::default()
        .send_uri(&portal_uri)
        .await
        .map_err(|e| format!("OpenURI portal call failed: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_dir_resolves_with_home_fallback() {
        // This test verifies the pictures_dir function doesn't panic
        // Actual resolution depends on environment
        let _result = pictures_dir();
    }

    #[tokio::test]
    async fn open_uri_portal_fails_gracefully_for_nonexistent_path() {
        let result = open_uri_portal(&PathBuf::from("/nonexistent/path/that/should/fail")).await;
        assert!(result.is_err(), "should fail for nonexistent path");
    }
}

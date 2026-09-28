//! `StatusNotifierWatcher` registration: the handshake that makes the tray
//! visible, and the degrade path that keeps the daemon alive without it.
//!
//! Contract: an ABSENT watcher disables the tray with a
//! warn log - never an error, never a crash, zero daemon impact. The
//! registration task keeps listening for `NameOwnerChanged` so a watcher
//! that appears LATER (a panel restarting after the daemon) still picks
//! the item up, and a vanishing watcher releases the tray persistence
//! reason (the lifecycle flag).

use std::sync::Arc;

use futures::StreamExt;
use zbus::{Connection, fdo};

use super::spec::{ITEM_PATH, WATCHER_INTERFACE, WATCHER_PATH, WATCHER_SERVICE};
use crate::state::DaemonState;

/// Runs until aborted: registers with the watcher when one is present and
/// re-evaluates on every watcher owner change.
pub(super) async fn registration_task(connection: Connection, state: Arc<DaemonState>) {
    let Ok(dbus) = fdo::DBusProxy::new(&connection).await else {
        tracing::warn!("the bus exposes no org.freedesktop.DBus; tray stays disabled");
        return;
    };
    attempt_registration(&connection, &state).await;
    let mut owner_changes = match dbus
        .receive_name_owner_changed_with_args(&[(0, WATCHER_SERVICE)])
        .await
    {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(%error, "cannot watch for a late StatusNotifierWatcher; tray state is final for this run");
            return;
        }
    };
    while let Some(signal) = owner_changes.next().await {
        let Ok(args) = signal.args() else {
            continue;
        };
        if args.new_owner().as_ref().is_some() {
            attempt_registration(&connection, &state).await;
        } else {
            state.set_tray(false);
            tracing::warn!(
                watcher = WATCHER_SERVICE,
                "the StatusNotifierWatcher vanished; tray disabled until one reappears"
            );
        }
    }
}

async fn attempt_registration(connection: &Connection, state: &DaemonState) {
    let Some(unique_name) = connection.unique_name() else {
        tracing::warn!("the connection has no unique name (p2p?); tray registration skipped");
        return;
    };
    match connection
        .call_method(
            Some(WATCHER_SERVICE),
            WATCHER_PATH,
            Some(WATCHER_INTERFACE),
            "RegisterStatusNotifierItem",
            &unique_name.as_str(),
        )
        .await
    {
        Ok(_reply) => {
            state.set_tray(true);
            tracing::info!(
                watcher = WATCHER_SERVICE,
                item = %format!("{unique_name}{ITEM_PATH}"),
                "tray registered with the StatusNotifierWatcher"
            );
        }
        Err(zbus::Error::MethodError(name, detail, _reply))
            if name.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
        {
            state.set_tray(false);
            tracing::warn!(
                "no StatusNotifierWatcher on the bus; tray disabled (retrying when one appears)"
            );
            tracing::debug!(error = %name, detail = ?detail, "watcher registration rejection detail");
        }
        Err(error) => {
            state.set_tray(false);
            tracing::warn!(%error, "StatusNotifierWatcher registration failed; tray disabled (retrying on watcher changes)");
        }
    }
}

//! The `org.freedesktop.portal.GlobalShortcuts` client (the
//! primary path) on `ashpd` 0.13.13.
//!
//! PINNED API REALITY (vendored 0.13.13 source read before coding):
//!
//! - `GlobalShortcuts::new()` -> `create_session(CreateSessionOptions)` ->
//!   `bind_shortcuts(session, &[NewShortcut], None, BindShortcutsOptions)`;
//!   0.13 moved the call options into (default-able) option structs and the
//!   parent-window identifier stays `None` (the daemon is headless).
//! - `bind_shortcuts` returns a `Request` whose response is ALREADY
//!   resolved (`Proxy::request` joins `prepare_response` with the call), so
//!   `response()` is safe immediately after the await.
//! - There is NO `restore_token`/`persist_mode` (unlike `ScreenCast`) and
//! - `create_session()` carries an internal `assert_eq!` on the session
//!   path convention: a non-conforming portal PANICS inside ashpd. The
//!   facade therefore runs registration inside `tokio::spawn` (a panic
//!   arrives as `JoinError`, never as a daemon crash).
//! - The connection is a process-global `OnceLock` singleton bound to the
//!   session bus (`proxy.rs` `static SESSION`): correct in production; the
//!   stub-portal test steers it via `DBUS_SESSION_BUS_ADDRESS` before first
//!   use.
//! - `Activated` is broadcast for EVERY session on the interface, so the
//!   listener dispatches only ids present in this daemon's command map.

use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use ashpd::desktop::global_shortcuts::{
    Activated, BindShortcutsOptions, GlobalShortcuts, NewShortcut,
};
use ashpd::desktop::{CreateSessionOptions, Session};
use futures::{Stream, StreamExt};
use tokio::sync::Notify;

use super::spec::ShortcutSpec;
use crate::command::{CommandSink, DaemonCommand};
use crate::error::DaemonError;
use crate::state::DaemonState;

/// A successful registration: the LIVE `Activated` subscription (added
/// before registration reports success, so no trigger can slip between the
/// bind and the subscribe), the session proxy for the graceful close, and
/// the bound shortcut reply (id, description, portal-assigned trigger
/// description).
pub struct PortalParts {
    pub(super) session: Session<GlobalShortcuts>,
    pub(super) activated: Pin<Box<dyn Stream<Item = Activated> + Send>>,
    /// What the portal bound (may be a subset/reorder of the request).
    pub bound: Vec<(String, String, String)>,
}

impl fmt::Debug for PortalParts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PortalParts")
            .field("session", &self.session)
            .field("activated", &"<signal stream>")
            .field("bound", &self.bound)
            .finish()
    }
}

/// Runs the portal registration sequence.
///
/// # Errors
///
/// [`DaemonError::ShortcutPortal`] when the portal is absent, the request
/// is denied, or the wire call fails (classification per the module pins).
pub async fn register(specs: &[ShortcutSpec]) -> Result<PortalParts, DaemonError> {
    let shortcuts = GlobalShortcuts::new()
        .await
        .map_err(|error| classify(&error))?;
    let session = shortcuts
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|error| classify(&error))?;
    let requested: Vec<NewShortcut> = specs
        .iter()
        .map(|spec| {
            NewShortcut::new(spec.id.clone(), spec.description.clone())
                .preferred_trigger(spec.trigger.as_str())
        })
        .collect();
    let request = shortcuts
        .bind_shortcuts(&session, &requested, None, BindShortcutsOptions::default())
        .await
        .map_err(|error| classify(&error))?;
    let reply = request.response().map_err(|error| classify(&error))?;
    let bound: Vec<(String, String, String)> = reply
        .shortcuts()
        .iter()
        .map(|shortcut| {
            (
                shortcut.id().to_owned(),
                shortcut.description().to_owned(),
                shortcut.trigger_description().to_owned(),
            )
        })
        .collect();
    let activated = shortcuts
        .receive_activated()
        .await
        .map_err(|error| classify(&error))?;
    tracing::info!(count = bound.len(), "portal GlobalShortcuts session bound");
    Ok(PortalParts {
        session,
        activated: Box::pin(activated),
        bound,
    })
}

/// What the `Activated` listener needs (grouped: the listener future takes
/// exactly two arguments).
#[derive(Debug, Clone)]
pub struct ListenerContext {
    /// Shortcut id -> command (ids outside the map are ignored).
    pub commands: HashMap<String, DaemonCommand>,
    /// Where triggered commands go (the daemon seam; the CLI wiring plugs
    /// execution).
    pub sink: Arc<dyn CommandSink>,
    /// Activity stamp + persistence reason owner.
    pub state: Arc<DaemonState>,
    /// Notified once by [`super::PortalRegistration::shutdown`].
    pub cancel: Arc<Notify>,
}

/// The `Activated` listen loop: dispatches mapped shortcut ids until the
/// stream ends (portal vanished) or `cancel` fires, then closes the portal
/// session best-effort and releases the persistence reason. Runs inside
/// `tokio::spawn` (the facade owns the handle).
pub async fn listen(parts: PortalParts, ctx: ListenerContext) {
    let PortalParts {
        session,
        mut activated,
        ..
    } = parts;
    loop {
        tokio::select! {
            event = activated.next() => {
                let Some(event) = event else {
                    tracing::warn!("the portal Activated stream ended; shortcuts will no longer trigger");
                    break;
                };
                dispatch_activated(&ctx, event.shortcut_id());
            }
            () = ctx.cancel.notified() => break,
        }
    }
    if let Err(error) = session.close().await {
        tracing::debug!(%error, "portal session close reported an error (the connection close cleans up)");
    }
    ctx.state.set_shortcuts_registered(false);
    tracing::info!("global shortcuts listener stopped");
}

/// The pure trigger dispatch (unit-tested without a bus): known ids touch
/// the activity stamp and land on the sink; foreign ids (the portal
/// broadcasts every session's activations) are ignored.
pub fn dispatch_activated(ctx: &ListenerContext, shortcut_id: &str) {
    ctx.state.touch(Instant::now());
    if let Some(command) = ctx.commands.get(shortcut_id) {
        tracing::info!(shortcut = %shortcut_id, command = %command, "global shortcut activated");
        ctx.sink.dispatch(command.clone());
    } else {
        tracing::debug!(
            shortcut = %shortcut_id,
            "ignoring Activated for a shortcut this daemon did not register"
        );
    }
}

/// Maps `ashpd` failures onto the typed daemon error (the fallback ladder
/// consumes the message; the variant keeps it non-fatal).
fn classify(error: &ashpd::Error) -> DaemonError {
    const NAME_HAS_NO_OWNER: &str = "org.freedesktop.DBus.Error.NameHasNoOwner";
    match error {
        ashpd::Error::PortalNotFound(_) => DaemonError::ShortcutPortal(
            "the portal frontend does not implement GlobalShortcuts".to_owned(),
        ),
        ashpd::Error::Response(response) => {
            DaemonError::ShortcutPortal(format!("the portal denied the registration ({response})"))
        }
        ashpd::Error::Zbus(ashpd::zbus::Error::MethodError(name, _, _))
            if name.as_str() == NAME_HAS_NO_OWNER =>
        {
            DaemonError::ShortcutPortal(
                "no portal frontend owns org.freedesktop.portal.Desktop (is xdg-desktop-portal running?)".to_owned(),
            )
        }
        // ashpd::Error is #[non_exhaustive] -> documented catch-all.
        other => DaemonError::ShortcutPortal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::RecordingSink;
    use crate::shortcut::spec::{command_map, default_shortcuts};
    use std::time::Duration;

    fn context(sink: Arc<RecordingSink>, state: Arc<DaemonState>) -> ListenerContext {
        ListenerContext {
            commands: command_map(&default_shortcuts()),
            sink,
            state,
            cancel: Arc::new(Notify::new()),
        }
    }

    fn sixty_secs_ago() -> Instant {
        Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(|| panic!("the monotonic clock cannot represent a 60s past"))
    }

    #[test]
    fn known_id_dispatches_the_mapped_command_and_touches_activity() {
        let sink = Arc::new(RecordingSink::new());
        let state = Arc::new(DaemonState::new(false, Instant::now()));
        state.touch(sixty_secs_ago());
        let ctx = context(Arc::clone(&sink), Arc::clone(&state));

        dispatch_activated(&ctx, "capture-full");

        assert_eq!(sink.commands(), vec![DaemonCommand::CaptureFull]);
        assert!(state.idle_for(Instant::now()) < Duration::from_secs(60));
    }

    #[test]
    fn foreign_ids_are_ignored_but_still_count_as_activity() {
        let sink = Arc::new(RecordingSink::new());
        let state = Arc::new(DaemonState::new(false, Instant::now()));
        state.touch(sixty_secs_ago());
        let ctx = context(Arc::clone(&sink), Arc::clone(&state));

        dispatch_activated(&ctx, "some-other-apps-shortcut");

        assert!(sink.commands().is_empty());
        assert!(state.idle_for(Instant::now()) < Duration::from_secs(60));
    }

    #[test]
    fn every_default_id_dispatches_its_command() {
        let sink = Arc::new(RecordingSink::new());
        let state = Arc::new(DaemonState::new(false, Instant::now()));
        let ctx = context(Arc::clone(&sink), Arc::clone(&state));
        for spec in default_shortcuts() {
            dispatch_activated(&ctx, &spec.id);
        }
        let dispatched = sink.commands();
        assert_eq!(dispatched.len(), 3);
        assert_eq!(dispatched[1], DaemonCommand::CaptureFull);
    }

    #[test]
    fn classification_covers_denied_absent_and_catch_all() {
        let denied = classify(&ashpd::Error::Response(
            ashpd::desktop::ResponseError::Cancelled,
        ));
        assert!(denied.to_string().contains("denied"), "message: {denied}");
        let absent = classify(&ashpd::Error::PortalNotFound(
            "org.freedesktop.portal.GlobalShortcuts"
                .try_into()
                .unwrap_or_else(|error| panic!("{error}")),
        ));
        assert!(
            absent.to_string().contains("does not implement"),
            "message: {absent}"
        );
        let message =
            ashpd::zbus::Message::method_call("/org/freedesktop/portal/desktop", "CreateSession")
                .and_then(|builder| builder.build(&()))
                .unwrap_or_else(|error| panic!("{error}"));
        let no_owner = classify(&ashpd::Error::Zbus(ashpd::zbus::Error::MethodError(
            "org.freedesktop.DBus.Error.NameHasNoOwner"
                .try_into()
                .unwrap_or_else(|error| panic!("{error}")),
            None,
            message,
        )));
        assert!(
            no_owner.to_string().contains("no portal frontend"),
            "message: {no_owner}"
        );
        let other = classify(&ashpd::Error::NoResponse);
        assert!(matches!(other, DaemonError::ShortcutPortal(_)));
    }
}

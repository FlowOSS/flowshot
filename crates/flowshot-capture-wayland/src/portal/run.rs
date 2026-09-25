//! Shared portal execution plumbing.
//!
//! # Execution model
//!
//! The portal backends reuse the crate's one-shot worker discipline
//! ([`spawn_worker`]): every operation runs on a short-lived worker thread.
//! Unlike the Wayland backends, the portal chain is async `D-Bus` (ashpd is
//! built on `zbus`, whose workspace feature set is the `tokio` reactor), so
//! the worker closure owns a private current-thread `tokio` runtime
//! ([`build_runtime`]) and drives the portal calls with `block_on`. The
//! `PipeWire` main loop (`ScreenCast`) is plain blocking C-loop code and runs
//! directly on the worker thread between runtime phases. Keeping the runtime
//! thread-local to the worker makes the backends runtime-agnostic from the
//! caller's perspective: the shared [`CaptureBackend`] contract stays
//! drivable from any executor (the trait doctests use `futures::executor`).
//!
//! Cancellation/timeout teardown: when a deadline wins, the portal future is
//! dropped mid-call. Any already-created portal session dies with the worker:
//! dropping the runtime closes the `zbus` connection, and the portal cancels
//! sessions whose caller disappeared - the same "connection close is the
//! universal cleanup" guarantee the Wayland backends rely on.
//!
//! [`spawn_worker`]: crate::worker::spawn_worker
//! [`CaptureBackend`]: flowshot_capture::CaptureBackend

use std::future::Future;
use std::time::{Duration, Instant};

use flowshot_core::geometry::OutputInfo;
use wayland_client::Connection;

use super::error::{PortalBackendError, PortalDenial, PortalErrorKind};
use crate::error::{BackendError, socket_connect_error};
use crate::icc::wait::collect_deadline_with;
use crate::session::CaptureState;

/// Per-phase portal deadline (draft F27 parity with the other backends'
/// per-phase budgets): a non-interactive Screenshot request, the `PipeWire`
/// first-frame wait, and the availability probe each get this budget.
pub(crate) const PORTAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Budget for the `ScreenCast` handshake (`CreateSession` + `SelectSources` +
/// `Start`).
///
/// Longer than [`PORTAL_TIMEOUT`] by design: implementations run their
/// interactive source picker INSIDE these calls (`XDPH` blocks
/// `SelectSources` on `hyprland-share-picker`, GNOME shows its dialog at
/// `Start`), so a human choosing a source gets a human-scale budget. The
/// run is still bounded - never a hang.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// Which outputs a portal capture run covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Selection {
    /// Every enumerated output, in registry-name order.
    All,
    /// The single output with this connector name.
    Named(String),
}

/// Builds the worker-thread `tokio` runtime the portal `D-Bus` calls run on.
///
/// # Errors
///
/// The backend's `Io` error when the operating system refuses the runtime
/// (driver setup failure).
pub(crate) fn build_runtime<E: BackendError>() -> Result<tokio::runtime::Runtime, E> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(E::from)
}

/// Enumerates the session's outputs on a one-shot Wayland connection.
///
/// The portals deliver images and streams without geometry metadata this
/// crate trusts (a Screenshot composite carries no per-output layout, and
/// `ScreenCast` stream positions are implementation-defined), so both portal
/// backends anchor placement on the same `wl_output` + `xdg-output`
/// enumeration the native backends use.
///
/// # Errors
///
/// The backend's connect, timeout, transport, or protocol error.
pub(crate) fn collect_outputs<E: PortalBackendError>() -> Result<Vec<OutputInfo>, E> {
    let conn = Connection::connect_to_env().map_err(socket_connect_error)?;
    let mut queue = conn.new_event_queue::<CaptureState>();
    let mut state = CaptureState::new();
    collect_deadline_with::<E>(
        &conn,
        &mut queue,
        &mut state,
        Instant::now() + PORTAL_TIMEOUT,
    )?;
    Ok(state.snapshot().outputs)
}

/// Runs `work` under a hard budget, mapping an expired deadline onto the
/// backend's timeout error and dropping (cancelling) the work future.
///
/// Must run inside the worker's `tokio` runtime ([`build_runtime`]).
pub(crate) async fn with_deadline<E, T>(
    budget: Duration,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, E>
where
    E: BackendError,
{
    tokio::select! {
        result = work => result,
        () = tokio::time::sleep(budget) => {
            tracing::warn!(?budget, "portal phase exceeded its deadline; cancelling");
            Err(E::timeout(budget))
        }
    }
}

/// Maps an ashpd portal call result onto the shared portal vocabulary: the
/// `org.freedesktop.portal.Request` response-status state machine.
///
/// Status `0` arrives as `Ok` (ashpd resolves the request future only after
/// the `Response` signal); status `1` surfaces as
/// `Error::Response(Cancelled)` and status `2` as `Error::Response(Other)` -
/// both are user-facing denials, never transport failures. Everything else
/// (including future `ashpd::Error` variants - the enum is
/// `#[non_exhaustive]`) is a `D-Bus` failure.
pub(crate) fn classify<T>(result: Result<T, ashpd::Error>) -> Result<T, PortalErrorKind> {
    use ashpd::desktop::ResponseError;
    match result {
        Ok(value) => Ok(value),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Err(PortalErrorKind::Denied {
            reason: PortalDenial::Cancelled,
        }),
        Err(ashpd::Error::Response(ResponseError::Other)) => Err(PortalErrorKind::Denied {
            reason: PortalDenial::Other,
        }),
        Err(source) => Err(PortalErrorKind::Dbus { source }),
    }
}

/// Resolves the selection against the enumerated outputs.
///
/// # Errors
///
/// [`PortalErrorKind::NoOutputs`] when the session has no outputs and
/// [`PortalErrorKind::OutputNotFound`] for a connector name the session does
/// not advertise (listing the available connectors).
pub(crate) fn select_outputs(
    outputs: Vec<OutputInfo>,
    selection: &Selection,
) -> Result<Vec<OutputInfo>, PortalErrorKind> {
    if outputs.is_empty() {
        return Err(PortalErrorKind::NoOutputs);
    }
    match selection {
        Selection::All => Ok(outputs),
        Selection::Named(name) => {
            let available = outputs
                .iter()
                .map(|output| output.connector.clone())
                .collect();
            let found = outputs
                .into_iter()
                .find(|output| &output.connector == name)
                .ok_or_else(|| PortalErrorKind::OutputNotFound {
                    requested: name.clone(),
                    available,
                })?;
            Ok(vec![found])
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_capture::BackendKind;
    use flowshot_core::geometry::{Logical, LogicalRect, PhysicalPx, PhysicalSize, Transform};

    use super::super::error::PortalScreenshotError;
    use super::*;

    fn output(connector: &str) -> OutputInfo {
        OutputInfo::new(
            connector,
            connector,
            LogicalRect::new(Logical(0.0), Logical(0.0), Logical(4.0), Logical(3.0)),
            PhysicalSize::new(PhysicalPx(4), PhysicalPx(3)),
            1.0,
            Transform::Normal,
        )
        .unwrap()
    }

    // ---- response-status state machine (injected fake responses) ----

    #[test]
    fn status_zero_passes_the_value_through() {
        let classified = classify(Ok::<_, ashpd::Error>("frame"));
        assert_eq!(classified.unwrap(), "frame");
    }

    #[test]
    fn status_one_maps_to_denied_cancelled() {
        let classified = classify(Err::<(), _>(ashpd::Error::Response(
            ashpd::desktop::ResponseError::Cancelled,
        )));
        assert!(matches!(
            classified.unwrap_err(),
            PortalErrorKind::Denied {
                reason: PortalDenial::Cancelled
            }
        ));
    }

    #[test]
    fn status_two_maps_to_denied_other() {
        let classified = classify(Err::<(), _>(ashpd::Error::Response(
            ashpd::desktop::ResponseError::Other,
        )));
        assert!(matches!(
            classified.unwrap_err(),
            PortalErrorKind::Denied {
                reason: PortalDenial::Other
            }
        ));
    }

    #[test]
    fn transport_failures_map_to_dbus_not_denial() {
        // NoResponse is a constructible non-response failure; a dismissed
        // request must never be confused with a broken transport.
        let classified = classify(Err::<(), _>(ashpd::Error::NoResponse));
        match classified.unwrap_err() {
            PortalErrorKind::Dbus { source } => {
                assert!(matches!(source, ashpd::Error::NoResponse));
            }
            other => panic!("expected Dbus, got {other:?}"),
        }
    }

    #[test]
    fn timeout_budget_expires_on_a_pending_request() {
        // The timeout leg of the state machine: a portal call that never
        // answers must surface the backend's Timeout, deterministically (the
        // pending future cannot win the race).
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let budget = Duration::from_millis(20);
        let result = runtime.block_on(with_deadline::<PortalScreenshotError, ()>(
            budget,
            std::future::pending(),
        ));
        match result.unwrap_err().kind() {
            PortalErrorKind::Timeout { timeout } => assert_eq!(*timeout, budget),
            other => panic!("expected Timeout, got {other:?}"),
        }
        // The CaptureError lift tags the timeout with the backend kind.
        let lifted = PortalScreenshotError::timeout(budget);
        assert!(matches!(
            flowshot_capture::CaptureError::from(lifted),
            flowshot_capture::CaptureError::Timeout {
                backend: BackendKind::PortalScreenshot
            }
        ));
    }

    #[test]
    fn completed_work_wins_over_the_budget() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let result = runtime.block_on(with_deadline::<PortalScreenshotError, _>(
            Duration::from_secs(15),
            std::future::ready(Ok(7u32)),
        ));
        assert_eq!(result.unwrap(), 7);
    }

    // ---- selection ----

    #[test]
    fn empty_session_is_a_typed_error() {
        let err = select_outputs(vec![], &Selection::All).unwrap_err();
        assert!(matches!(err, PortalErrorKind::NoOutputs));
    }

    #[test]
    fn named_selection_filters_and_rejects_unknown_connectors() {
        let outputs = vec![output("HDMI-A-1"), output("DP-3")];
        let selected = select_outputs(outputs.clone(), &Selection::Named("DP-3".to_owned()));
        assert_eq!(selected.unwrap().len(), 1);
        let err = select_outputs(outputs, &Selection::Named("DP-99".to_owned())).unwrap_err();
        match err {
            PortalErrorKind::OutputNotFound {
                requested,
                available,
            } => {
                assert_eq!(requested, "DP-99");
                assert_eq!(available, vec!["HDMI-A-1".to_owned(), "DP-3".to_owned()]);
            }
            other => panic!("expected OutputNotFound, got {other:?}"),
        }
    }

    #[test]
    fn all_selection_keeps_enumeration_order() {
        let outputs = vec![output("HDMI-A-1"), output("DP-3")];
        let selected = select_outputs(outputs.clone(), &Selection::All).unwrap();
        assert_eq!(selected, outputs);
    }
}

//! Deadline-bounded Wayland dispatch for the one-shot capture connections.
//!
//! The long-lived probe thread dispatches through `calloop`; the capture
//! runners instead wait synchronously with a hard deadline, because a capture
//! must surface a backend `Timeout` (mapped to [`CaptureError::Timeout`])
//! instead of hanging when the compositor stalls - or withholds frames behind
//! a pending permission popup.
//!
//! The machinery is generic over the backend error ([`BackendError`]) so both
//! the `ext-image-copy-capture-v1` ([`IccError`]) and `wlr-screencopy-v1`
//! ([`ScreencopyError`]) runners share one poll loop; the `_with` functions
//! take the error type explicitly, and the bare-named wrappers pin it to
//! [`IccError`] for the ICC and cursor runners.
//!
//! [`CaptureError::Timeout`]: flowshot_capture::CaptureError::Timeout
//! [`ScreencopyError`]: crate::error::ScreencopyError

use std::io::ErrorKind;
use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, PollTimeout};
use wayland_client::backend::WaylandError;
use wayland_client::{Connection, EventQueue};

use crate::error::{BackendError, IccError};
use crate::session::CaptureState;

/// Deadline for each capture phase group: session collection and every
/// per-output capture each get a fresh 10 seconds (a stalled compositor
/// surfaces as [`CaptureError::Timeout`], never a hang).
///
/// [`CaptureError::Timeout`]: flowshot_capture::CaptureError::Timeout
pub(crate) const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

/// [`collect_deadline_with`] pinned to [`IccError`].
pub(crate) fn collect_deadline(
    conn: &Connection,
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    deadline: Instant,
) -> Result<(), IccError> {
    collect_deadline_with::<IccError>(conn, queue, state, deadline)
}

/// Collects the registry, output geometry, and capture-manager binds with a
/// deadline on every round-trip (the startup [`collect_session`] uses
/// unbounded round-trips; a frozen compositor must not hang a capture run).
///
/// [`collect_session`]: crate::session::collect_session
///
/// # Errors
///
/// The backend's timeout, transport, or protocol error for any bounded
/// round-trip that fails.
pub(crate) fn collect_deadline_with<E>(
    conn: &Connection,
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    deadline: Instant,
) -> Result<(), E>
where
    E: BackendError,
{
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    state.registry = Some(registry);
    roundtrip_deadline_with::<E>(conn, queue, state, deadline)?;
    state.attach_all_xdg_outputs(&qh);
    roundtrip_deadline_with::<E>(conn, queue, state, deadline)?;
    roundtrip_deadline_with::<E>(conn, queue, state, deadline)?;
    Ok(())
}

/// [`dispatch_until_with`] pinned to [`IccError`].
pub(crate) fn dispatch_until(
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    deadline: Instant,
    satisfied: impl Fn(&CaptureState) -> bool,
) -> Result<(), IccError> {
    dispatch_until_with::<IccError>(queue, state, deadline, satisfied)
}

/// Dispatches events until `satisfied` holds or the deadline expires.
///
/// The wait uses `prepare_read` + bounded `poll` + `read` (the pattern
/// `calloop-wayland-source` uses internally), so a frozen compositor surfaces
/// as a backend timeout instead of hanging.
///
/// # Errors
///
/// The backend's `Timeout` when the deadline expires first, plus transport and
/// protocol errors from the connection.
pub(crate) fn dispatch_until_with<E>(
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    deadline: Instant,
    satisfied: impl Fn(&CaptureState) -> bool,
) -> Result<(), E>
where
    E: BackendError,
{
    loop {
        queue.dispatch_pending(state)?;
        if satisfied(state) {
            return Ok(());
        }
        queue.flush()?;
        let Some(remaining) = wait_budget(Instant::now(), deadline) else {
            return Err(E::timeout(CAPTURE_TIMEOUT));
        };
        let Some(guard) = queue.prepare_read() else {
            return Err(E::internal(
                "another reader is active on the one-shot capture connection",
            ));
        };
        let timeout = PollTimeout::try_from(remaining)
            .map_err(|_| E::internal("wait budget exceeds the poll timeout range"))?;
        let fd = guard.connection_fd();
        let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
        match nix::poll::poll(&mut fds, timeout) {
            Ok(_) => {}
            // Dropping the guard cancels the prepared read; retrying is safe.
            Err(nix::Error::EINTR) => continue,
            Err(error) => return Err(E::from(std::io::Error::from(error))),
        }
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(E::from(error)),
        }
    }
}

/// The remaining wait budget, or `None` when the deadline has expired.
///
/// Pure so the timeout decision is testable with injected instants (no
/// sleeping, no live socket).
fn wait_budget(now: Instant, deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(now)
        .filter(|remaining| !remaining.is_zero())
}

/// A display round-trip bounded by the deadline (the callback's `done` event
/// clears the pending flag).
///
/// # Errors
///
/// The backend's timeout, transport, or protocol error when the bounded
/// round-trip fails.
pub(crate) fn roundtrip_deadline_with<E>(
    conn: &Connection,
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    deadline: Instant,
) -> Result<(), E>
where
    E: BackendError,
{
    state.roundtrip_pending = true;
    let qh = queue.handle();
    let _callback = conn.display().sync(&qh, ());
    dispatch_until_with::<E>(queue, state, deadline, |state| !state.roundtrip_pending)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn wait_budget_reports_remaining_time_and_expiry() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(5);
        let remaining = wait_budget(now, deadline).unwrap();
        assert!(remaining > Duration::from_secs(4));
        assert!(remaining <= Duration::from_secs(5));
        assert_eq!(wait_budget(now, now), None);
        assert_eq!(wait_budget(now + Duration::from_secs(1), now), None);
    }
}

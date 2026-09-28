//! Optional `sd_notify(3)` handshake (cargo feature `systemd`).
//!
//! The daemon is INIT-AGNOSTIC - `flowshot daemon` runs in
//! the foreground under ANY supervisor; this module only speaks the
//! readiness datagram when systemd asked for it (`NOTIFY_SOCKET` set).
//! Hand-rolled (~20 lines) instead of adding the `sd-notify` crate: it is
//! absent from the workspace table and `Cargo.lock`, and the protocol is
//! one datagram. Abstract-namespace sockets (`@`-prefixed) are not
//! supported without `libc` binding; systemd's user manager uses a
//! filesystem socket, so this is a documented no-op path.

use std::os::unix::net::UnixDatagram;
use std::path::Path;

use crate::error::DaemonError;

/// What the readiness handshake did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyOutcome {
    /// `READY=1` was sent to the supervisor's socket.
    Sent,
    /// `NOTIFY_SOCKET` is unset - not running under systemd notification.
    NoSocket,
    /// The socket is in the abstract namespace (unsupported without
    /// `libc`); readiness is simply not reported.
    AbstractUnsupported,
}

/// Reports readiness to the supervisor named by `NOTIFY_SOCKET`.
///
/// # Errors
///
/// [`DaemonError::Io`] when the datagram cannot be sent,
/// [`DaemonError::NonUnicodePath`] for a non-UTF-8 socket path.
pub fn notify_ready() -> Result<NotifyOutcome, DaemonError> {
    let Some(socket) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(NotifyOutcome::NoSocket);
    };
    if socket.to_str().is_some_and(|value| value.starts_with('@')) {
        return Ok(NotifyOutcome::AbstractUnsupported);
    }
    notify_ready_at(Path::new(&socket))
}

/// Sends `READY=1` to `socket` (the injectable twin of [`notify_ready`]).
///
/// # Errors
///
/// [`DaemonError::Io`] when the datagram cannot be sent.
pub fn notify_ready_at(socket: &Path) -> Result<NotifyOutcome, DaemonError> {
    let datagram = UnixDatagram::unbound()?;
    datagram.send_to(b"READY=1", socket)?;
    Ok(NotifyOutcome::Sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixDatagram as StdDatagram;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn ready_datagram_arrives_at_the_socket() {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "flowshot-sdnotify-test-{}-{unique}.sock",
            std::process::id()
        ));
        let listener = StdDatagram::bind(&path).unwrap_or_else(|error| panic!("{error}"));

        let outcome = notify_ready_at(&path).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(outcome, NotifyOutcome::Sent);

        let mut buffer = [0u8; 16];
        let received = listener
            .recv(&mut buffer)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(&buffer[..received], b"READY=1");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn sending_to_an_unbound_path_is_a_typed_io_error() {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "flowshot-sdnotify-absent-{}-{unique}.sock",
            std::process::id()
        ));
        let error = notify_ready_at(&path)
            .err()
            .unwrap_or_else(|| panic!("an absent socket must fail"));
        assert!(matches!(error, DaemonError::Io(_)));
    }
}

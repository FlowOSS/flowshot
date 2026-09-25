//! Typed errors for connecting to and probing a Wayland session.

use std::time::Duration;

use thiserror::Error;

/// Why establishing the Wayland capture session failed.
#[derive(Debug, Error)]
pub enum ConnectError {
    /// The Wayland compositor socket could not be connected. The `hint`
    /// explains the most likely cause based on the process environment.
    #[error("could not connect to the Wayland compositor: {source} (hint: {hint})")]
    Socket {
        /// The underlying connection error from `wayland-client`.
        source: wayland_client::ConnectError,
        /// Human-readable remediation hint derived from the environment.
        hint: String,
    },
    /// The socket connected, but session setup (registry probe, output
    /// collection, event loop construction) failed.
    #[error("Wayland session setup failed: {0}")]
    Setup(#[from] ProbeError),
    /// The capture thread did not report session setup completion in time.
    #[error("the Wayland capture thread did not finish session setup within {timeout:?}")]
    StartupTimeout {
        /// The startup deadline that expired.
        timeout: Duration,
    },
    /// The operating system refused to spawn the capture thread.
    #[error("the Wayland capture thread could not be spawned: {0}")]
    ThreadSpawn(#[from] std::io::Error),
}

/// Why an in-session probe or snapshot request failed.
#[derive(Debug, Error)]
pub enum ProbeError {
    /// The compositor sent a protocol error or the connection broke while
    /// dispatching events.
    #[error("Wayland protocol error during session probe: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    /// The `calloop` event loop infrastructure could not be set up or ran
    /// into a polling error.
    #[error("the capture event loop failed: {0}")]
    EventLoop(#[from] calloop::Error),
    /// The capture thread did not answer a request in time.
    #[error("the Wayland capture thread did not answer within {timeout:?}")]
    Timeout {
        /// The reply deadline that expired.
        timeout: Duration,
    },
    /// The capture thread exited (or its channel closed) before answering.
    #[error("the Wayland capture thread closed the session before answering")]
    ThreadClosed,
}

/// Wraps a `wayland-client` connect failure with an environment-based hint.
pub(crate) fn socket_connect_error(source: wayland_client::ConnectError) -> ConnectError {
    let hint = connect_hint(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
    );
    ConnectError::Socket { source, hint }
}

/// Builds the remediation hint for a failed socket connection.
///
/// Pure function of the two environment values `wayland-client` consults, so
/// the message logic is testable without touching the process environment.
fn connect_hint(
    wayland_display: Option<&std::ffi::OsStr>,
    runtime_dir: Option<&std::ffi::OsStr>,
) -> String {
    match (wayland_display, runtime_dir) {
        (_, None) => "XDG_RUNTIME_DIR is not set; a Wayland session must export it \
                      (this process is probably not running inside the session)"
            .to_owned(),
        (None, _) => "WAYLAND_DISPLAY is not set; are you running inside a Wayland session? \
                      (under X11 or a TTY, Wayland capture is unavailable)"
            .to_owned(),
        (Some(display), Some(dir)) => format!(
            "WAYLAND_DISPLAY={} does not name a live compositor socket under XDG_RUNTIME_DIR={}; \
             is the compositor running?",
            display.to_string_lossy(),
            dir.to_string_lossy()
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn hint_names_missing_runtime_dir_first() {
        let hint = connect_hint(Some("wayland-1".as_ref()), None);
        assert!(hint.contains("XDG_RUNTIME_DIR is not set"), "{hint}");
    }

    #[test]
    fn hint_names_missing_wayland_display() {
        let hint = connect_hint(None, Some("/run/user/1000".as_ref()));
        assert!(hint.contains("WAYLAND_DISPLAY is not set"), "{hint}");
    }

    #[test]
    fn hint_echoes_both_values_when_set() {
        let hint = connect_hint(
            Some("nonexistent".as_ref()),
            Some("/run/user/1000".as_ref()),
        );
        assert!(hint.contains("WAYLAND_DISPLAY=nonexistent"), "{hint}");
        assert!(hint.contains("/run/user/1000"), "{hint}");
    }

    #[test]
    fn socket_error_display_includes_hint() {
        let err = ConnectError::Socket {
            source: wayland_client::ConnectError::NoCompositor,
            hint: "check the session".to_owned(),
        };
        let text = err.to_string();
        assert!(text.contains("check the session"), "{text}");
        assert!(text.contains("Could not find wayland compositor"), "{text}");
    }
}

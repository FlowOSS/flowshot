//! The Hyprland IPC socket client: layer 2 of the cursor-position ladder
//! ([`crate::resolve`]).
//!
//! Hyprland answers the `cursorpos` request on its per-instance v1 IPC
//! socket (`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`)
//! with the cursor position in global logical coordinates - the same space
//! and value `hyprctl cursorpos` prints. `hyprctl` is itself only a client
//! of this socket; `FlowShot` talks to the socket directly and NEVER spawns an
//! external process (draft F28 no-shell-out).
//!
//! # Wire protocol
//!
//! Live-verified on Hyprland 0.56.2 and pinned against the upstream sources
//! (`src/ipc/s1/S1.cpp` request parser, `src/ipc/s1/Commands.cpp`
//! `cursorPosRequest`):
//!
//! - The request is the raw byte string with NO trailing newline - a newline
//!   yields the reply `unknown request`.
//! - The `j/` prefix selects the JSON output format, so `j/cursorpos`
//!   replies `{"x": <int>, "y": <int>}` wrapped in whitespace; the bare
//!   `cursorpos` request replies the text format `x, y` instead.
//! - Coordinates are floored to integers compositor-side
//!   (`untransformedPosition().floor()`), in global logical layout space.
//! - The server closes the connection after the reply, so reading to EOF
//!   with a socket timeout is bounded; write and read carry the target's
//!   timeout, and connect is prompt by nature (a Unix socket either
//!   answers, or `ENOENT`/`ECONNREFUSED` fails it immediately) - one-shot,
//!   no polling.
//!
//! # Degradation
//!
//! This layer is a fallback, never a requirement: every failure is a typed
//! [`HyprlandIpcError`] the ladder degrades with a warning, and layer 3
//! (overlay first motion) catches every desktop.

use std::io::{Read, Write};
use std::os::unix::net::{SocketAddr, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use flowshot_core::geometry::LogicalPoint;
use serde::Deserialize;
use thiserror::Error;

use crate::cursor::round_pair;

/// The request bytes: the `j/` prefix selects JSON output, `cursorpos` is
/// the command, and a trailing newline would be rejected (`unknown request`).
const REQUEST: &[u8] = b"j/cursorpos";

/// Upper bound on the reply size; the real reply is a few dozen bytes.
const REPLY_CAP: u64 = 4 * 1024;

/// Default bound for each socket phase (connect, write, read).
const IPC_TIMEOUT: Duration = Duration::from_millis(500);

/// Why a Hyprland IPC `cursorpos` query failed.
#[derive(Debug, Error)]
pub(crate) enum HyprlandIpcError {
    /// A socket operation failed: connect, write, or the bounded read
    /// (including a read timeout when the compositor did not answer).
    #[error("the Hyprland IPC socket could not complete the request: {0}")]
    Io(#[from] std::io::Error),
    /// The reply was not a JSON cursor position (e.g. `unknown request`
    /// from a Hyprland build with a different v1 dialect).
    #[error(
        "the Hyprland IPC reply was not a JSON cursor position (reply starts {reply_prefix:?}): {source}"
    )]
    Parse {
        /// The first characters of the raw reply, for diagnostics.
        reply_prefix: String,
        /// The JSON parse failure.
        source: serde_json::Error,
    },
}

/// A connection target for the Hyprland v1 IPC socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HyprlandIpc {
    runtime_dir: PathBuf,
    instance_signature: String,
    timeout: Duration,
}

/// The JSON reply shape of `j/cursorpos` (upstream `cursorPosRequest`,
/// `FORMAT_JSON` branch). The wire values are integers; `f64` accepts integer
/// and float renderings alike and rounds half away from zero.
#[derive(Deserialize)]
struct CursorPosReply {
    x: f64,
    y: f64,
}

impl HyprlandIpc {
    /// Builds the target from the process environment: `XDG_RUNTIME_DIR`
    /// and `HYPRLAND_INSTANCE_SIGNATURE`, both set and non-empty on any
    /// live Hyprland session (Hyprland exports them unconditionally).
    /// `None` outside one - the natural layer-2 gate.
    pub(crate) fn from_env() -> Option<Self> {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty())?;
        let instance_signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
            .ok()
            .filter(|value| !value.is_empty())?;
        Some(Self::new(
            Path::new(&runtime_dir),
            &instance_signature,
            IPC_TIMEOUT,
        ))
    }

    /// Creates a target for
    /// `<runtime_dir>/hypr/<instance_signature>/.socket.sock` with a bound
    /// on every socket phase.
    pub(crate) fn new(runtime_dir: &Path, instance_signature: &str, timeout: Duration) -> Self {
        Self {
            runtime_dir: runtime_dir.to_path_buf(),
            instance_signature: instance_signature.to_owned(),
            timeout,
        }
    }

    /// The v1 IPC socket path this target queries.
    pub(crate) fn socket_path(&self) -> PathBuf {
        self.runtime_dir
            .join("hypr")
            .join(&self.instance_signature)
            .join(".socket.sock")
    }

    /// Queries the cursor position once: connect, send the request, read
    /// the capped reply to EOF, parse the JSON. Blocking - the ladder runs
    /// it on a worker thread. Write and read are bounded by the target's
    /// timeout; nothing polls or waits unbounded.
    pub(crate) fn cursor_pos(&self) -> Result<(i32, i32), HyprlandIpcError> {
        let address = SocketAddr::from_pathname(self.socket_path())?;
        let stream = UnixStream::connect_addr(&address)?;
        stream.set_write_timeout(Some(self.timeout))?;
        stream.set_read_timeout(Some(self.timeout))?;
        (&stream).write_all(REQUEST)?;
        let mut reply = String::new();
        stream.take(REPLY_CAP).read_to_string(&mut reply)?;
        parse_cursorpos_reply(&reply)
    }
}

/// Parses one `cursorpos` JSON reply into the global logical integer
/// position (half away from zero, saturating - the `hyprctl`-compatible
/// rounding shared with the ICC layer).
fn parse_cursorpos_reply(reply: &str) -> Result<(i32, i32), HyprlandIpcError> {
    let parsed: CursorPosReply =
        serde_json::from_str(reply.trim()).map_err(|source| HyprlandIpcError::Parse {
            reply_prefix: reply.chars().take(48).collect(),
            source,
        })?;
    Ok(round_pair(LogicalPoint::from_raw(parsed.x, parsed.y)))
}

/// Shared fake-socket fixture for this module's and [`crate::resolve`]'s
/// tests.
#[cfg(test)]
pub(crate) mod test_support {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{HyprlandIpc, REQUEST};
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// A temporary `XDG_RUNTIME_DIR`-shaped directory whose
    /// `hypr/<signature>/.socket.sock` is served by a thread answering ONE
    /// request with `reply` (or stalling past the client timeout when
    /// `reply` is `None`), then closing - no live compositor involved.
    /// Returns the configured target, the directory to remove after the
    /// test, and the server handle to join.
    pub(crate) fn spawn_fake_socket(
        tag: &str,
        reply: Option<&'static str>,
        timeout: Duration,
    ) -> (HyprlandIpc, PathBuf, std::thread::JoinHandle<()>) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "flowshot-hypr-ipc-{tag}-{}-{unique}",
            std::process::id()
        ));
        let signature = "test-instance";
        let socket_dir = dir.join("hypr").join(signature);
        std::fs::create_dir_all(&socket_dir).unwrap();
        let listener = UnixListener::bind(socket_dir.join(".socket.sock")).unwrap();
        let server = std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            // The client keeps its end open until it has read the reply, so
            // the server reads the fixed-length request instead of waiting
            // for EOF.
            let mut request = [0u8; REQUEST.len()];
            if (&stream).read_exact(&mut request).is_ok() {
                assert_eq!(&request, REQUEST, "the client must send the raw v1 request");
            }
            match reply {
                Some(reply) => {
                    let _ = (&stream).write_all(reply.as_bytes());
                }
                None => std::thread::sleep(timeout * 4),
            }
            // Dropping the stream closes the connection: the client sees EOF.
        });
        (HyprlandIpc::new(&dir, signature, timeout), dir, server)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::test_support::spawn_fake_socket;
    use super::*;

    /// The exact reply bytes observed live on Hyprland 0.56.2 (`j/cursorpos`).
    const LIVE_REPLY: &str = "\n{\n    \"x\": 2603,\n    \"y\": 1043\n}\n";

    fn query(reply: Option<&'static str>, tag: &str) -> Result<(i32, i32), HyprlandIpcError> {
        let (ipc, dir, server) = spawn_fake_socket(tag, reply, Duration::from_millis(500));
        let result = ipc.cursor_pos();
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn socket_path_follows_the_hyprland_layout() {
        let ipc = HyprlandIpc::new(
            Path::new("/run/user/1000"),
            "sig_123",
            Duration::from_millis(50),
        );
        assert_eq!(
            ipc.socket_path(),
            PathBuf::from("/run/user/1000/hypr/sig_123/.socket.sock")
        );
    }

    #[test]
    fn fake_socket_serves_the_canned_json_reply() {
        // Given a fake v1 socket serving the live reply shape
        // When the layer queries it
        // Then the global logical integer position arrives.
        assert_eq!(query(Some(LIVE_REPLY), "happy").unwrap(), (2603, 1043));
    }

    #[test]
    fn float_values_round_half_away_from_zero() {
        assert_eq!(
            query(Some("{\"x\":100.5,\"y\":-20.6}"), "floats").unwrap(),
            (101, -21)
        );
    }

    #[test]
    fn missing_socket_is_a_typed_io_error() {
        let dir =
            std::env::temp_dir().join(format!("flowshot-hypr-ipc-absent-{}", std::process::id()));
        let ipc = HyprlandIpc::new(&dir, "nope", Duration::from_millis(50));
        assert!(matches!(ipc.cursor_pos(), Err(HyprlandIpcError::Io(_))));
    }

    #[test]
    fn silent_server_times_out_instead_of_hanging() {
        let timeout = Duration::from_millis(50);
        let (ipc, dir, server) = spawn_fake_socket("stall", None, timeout);
        let started = std::time::Instant::now();
        let result = ipc.cursor_pos();
        let elapsed = started.elapsed();
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(result, Err(HyprlandIpcError::Io(_))), "{result:?}");
        assert!(
            elapsed < Duration::from_secs(2),
            "the read must be bounded, took {elapsed:?}"
        );
    }

    #[test]
    fn non_json_reply_is_a_typed_parse_error_quoting_the_reply() {
        let error = query(Some("unknown request"), "garbage").unwrap_err();
        let HyprlandIpcError::Parse { reply_prefix, .. } = &error else {
            panic!("expected a parse error, got {error:?}");
        };
        assert_eq!(reply_prefix, "unknown request");
        assert!(error.to_string().contains("unknown request"));
    }

    #[test]
    fn parse_accepts_the_live_reply_shape() {
        assert_eq!(parse_cursorpos_reply(LIVE_REPLY).unwrap(), (2603, 1043));
        assert_eq!(
            parse_cursorpos_reply("{\"x\":3200,\"y\":720}").unwrap(),
            (3200, 720)
        );
    }

    #[test]
    fn parse_rejects_incomplete_or_empty_replies() {
        assert!(parse_cursorpos_reply("{\"x\":1}").is_err());
        assert!(parse_cursorpos_reply("{}").is_err());
        assert!(parse_cursorpos_reply("").is_err());
        assert!(parse_cursorpos_reply("2603, 1043").is_err());
    }
}

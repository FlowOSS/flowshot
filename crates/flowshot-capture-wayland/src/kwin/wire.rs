//! The `ScreenShot2` wire transport: connection targets, method calls with
//! the fd-IN pipe argument, reply classification, and the raw payload
//! readback.
//!
//! # Pipe protocol (pinned to the fetched `KWin` source)
//!
//! The CLIENT creates the pipe and supplies the write end as an INPUT
//! argument; `KWin` sends the metadata vardict reply FIRST, then writes the
//! raw `QImage` bits from a worker thread with poll-based backpressure and
//! closes the fd (end-of-file). The client therefore: holds its write end
//! until the call completes, drops it (so `KWin`'s close produces EOF),
//! parses the metadata, then reads exactly `stride * height` bytes under a
//! deadline.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::time::{Duration, Instant};

use nix::fcntl::OFlag;
use nix::poll::{PollFd, PollFlags, PollTimeout};
use zbus::zvariant::Value;
use zbus::{Connection, Message};

use super::error::{DecodeError, KwinError};
use super::meta::{RawFrameMeta, parse_metadata};
use super::{IFACE, KwinInteractiveKind, PATH, SERVICE};

/// `KWin`'s cancellation error name (interactive picker dismissed).
const ERROR_CANCELLED: &str = "org.kde.KWin.ScreenShot2.Error.Cancelled";
/// Prefix of `KWin`'s named `ScreenShot2` error family.
const ERROR_PREFIX: &str = "org.kde.KWin.ScreenShot2.Error.";

/// Which `D-Bus` peer the capture chain talks to.
pub(crate) enum KwinBus {
    /// The session bus (production).
    Session,
    /// A pre-built private peer connection (the stub-test seam; the p2p
    /// handshake needs both sides built concurrently, so the stub harness
    /// builds it).
    #[cfg(test)]
    Peer(Connection),
}

/// Which `ScreenShot2` method one capture run calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
    /// `CaptureActiveScreen(options, fd)`.
    ActiveScreen,
    /// `CaptureActiveWindow(options, fd)`.
    ActiveWindow,
    /// `CaptureArea(x, y, width, height, options, fd)`, logical coordinates.
    Area {
        /// Logical x origin.
        x: i32,
        /// Logical y origin.
        y: i32,
        /// Logical width.
        width: u32,
        /// Logical height.
        height: u32,
    },
    /// `CaptureInteractive(kind, options, fd)` - reserved, human picker.
    Interactive(KwinInteractiveKind),
}

/// Creates the client-supplied payload pipe (`O_CLOEXEC`, the `KWin` side
/// dup's with `F_DUPFD_CLOEXEC`).
///
/// # Errors
///
/// [`KwinError::Io`] when the operating system refuses the pipe.
pub(crate) fn open_pipe() -> Result<(OwnedFd, OwnedFd), KwinError> {
    nix::unistd::pipe2(OFlag::O_CLOEXEC)
        .map_err(|error| KwinError::from(std::io::Error::from(error)))
}

/// Establishes the `D-Bus` connection for the target bus.
///
/// # Errors
///
/// [`KwinError::Dbus`] when the connection or its handshake fails.
pub(crate) async fn connect(bus: KwinBus) -> Result<Connection, KwinError> {
    match bus {
        KwinBus::Session => Ok(Connection::session().await?),
        #[cfg(test)]
        KwinBus::Peer(connection) => Ok(connection),
    }
}

/// Issues the `ScreenShot2` method call for `request`, supplying the pipe's
/// write end as the fd-IN argument.
///
/// # Errors
///
/// [`KwinError::Cancelled`] / [`KwinError::KwinReply`] for `KWin`'s named
/// error family and [`KwinError::Dbus`] for every other failure.
pub(crate) async fn call_capture(
    connection: &Connection,
    request: &Request,
    paint_cursor: bool,
    write_end: BorrowedFd<'_>,
) -> Result<Message, KwinError> {
    let options = capture_options(request, paint_cursor);
    let pipe = zbus::zvariant::Fd::from(write_end);
    let called = match request {
        Request::ActiveScreen => call(connection, "CaptureActiveScreen", &(options, pipe)).await,
        Request::ActiveWindow => call(connection, "CaptureActiveWindow", &(options, pipe)).await,
        Request::Area {
            x,
            y,
            width,
            height,
        } => {
            call(
                connection,
                "CaptureArea",
                &(*x, *y, *width, *height, options, pipe),
            )
            .await
        }
        Request::Interactive(kind) => {
            call(
                connection,
                "CaptureInteractive",
                &(kind.wire_value(), options, pipe),
            )
            .await
        }
    };
    called.map_err(classify_dbus)
}

async fn call<B>(connection: &Connection, member: &str, body: &B) -> Result<Message, zbus::Error>
where
    B: serde::Serialize + zbus::zvariant::Type,
{
    connection
        .call_method(Some(SERVICE), PATH, Some(IFACE), member, body)
        .await
}

/// Builds the `options` vardict: cursor inclusion follows `paint_cursor`,
/// native resolution is always requested (physical-first rule), and window
/// captures include decorations. `include-shadow` and `hide-caller-windows`
/// keep their `KWin` defaults (both true).
fn capture_options(request: &Request, paint_cursor: bool) -> HashMap<&'static str, Value<'static>> {
    let mut options = HashMap::new();
    options.insert("include-cursor", Value::Bool(paint_cursor));
    options.insert("native-resolution", Value::Bool(true));
    if matches!(
        request,
        Request::ActiveWindow | Request::Interactive(KwinInteractiveKind::Window)
    ) {
        options.insert("include-decoration", Value::Bool(true));
    }
    options
}

/// Maps a `zbus` failure onto the backend vocabulary: `KWin`'s named error
/// family becomes typed variants, everything else stays a transport failure.
fn classify_dbus(error: zbus::Error) -> KwinError {
    if let zbus::Error::MethodError(name, detail, _) = &error
        && let Some(classified) = kwin_error_reply(name.as_str(), detail.as_deref())
    {
        return classified;
    }
    KwinError::Dbus { source: error }
}

/// Classifies a `D-Bus` error name from the `ScreenShot2` error family;
/// names outside the family are not `KWin`-specific failures.
fn kwin_error_reply(name: &str, detail: Option<&str>) -> Option<KwinError> {
    if name == ERROR_CANCELLED {
        return Some(KwinError::Cancelled);
    }
    name.starts_with(ERROR_PREFIX)
        .then(|| KwinError::KwinReply {
            name: name.to_owned(),
            message: detail.map(str::to_owned),
        })
}

/// Parses the reply body as the metadata vardict.
///
/// # Errors
///
/// [`KwinError::Decode`] when the body is not the contract's `a{sv}` or the
/// metadata violates the contract.
pub(crate) fn reply_metadata(reply: &Message) -> Result<RawFrameMeta, KwinError> {
    let body = reply.body();
    let values: HashMap<String, Value<'_>> = body
        .deserialize()
        .map_err(|source| DecodeError::MalformedReply { source })?;
    Ok(parse_metadata(&values)?)
}

/// Reads exactly `expected` payload bytes from the pipe under a budget:
/// `poll` for readability, `read` until complete; end-of-file before
/// completion is a truncated payload (`KWin`'s writer aborted).
///
/// # Errors
///
/// [`KwinError::Timeout`] when the budget expires,
/// [`DecodeError::TruncatedPayload`] on early end-of-file, and
/// [`KwinError::Io`] on pipe failures.
pub(crate) fn read_payload(
    read_end: BorrowedFd<'_>,
    expected: usize,
    budget: Duration,
) -> Result<Vec<u8>, KwinError> {
    let deadline = Instant::now() + budget;
    let mut data = vec![0u8; expected];
    let mut filled = 0;
    while filled < expected {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(KwinError::Timeout { timeout: budget })?;
        let timeout = PollTimeout::try_from(remaining).unwrap_or(PollTimeout::MAX);
        let mut polled = [PollFd::new(read_end, PollFlags::POLLIN)];
        match nix::poll::poll(&mut polled, timeout) {
            Ok(0) => return Err(KwinError::Timeout { timeout: budget }),
            Ok(_) => {}
            Err(nix::Error::EINTR) => continue,
            Err(error) => return Err(KwinError::from(std::io::Error::from(error))),
        }
        match nix::unistd::read(read_end.as_raw_fd(), &mut data[filled..]) {
            Ok(0) => {
                return Err(KwinError::from(DecodeError::TruncatedPayload {
                    expected,
                    received: filled,
                }));
            }
            Ok(count) => filled += count,
            Err(nix::Error::EINTR | nix::Error::EAGAIN) => {}
            Err(error) => return Err(KwinError::from(std::io::Error::from(error))),
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn cancelled_error_name_maps_to_the_typed_cancellation() {
        assert!(matches!(
            kwin_error_reply(ERROR_CANCELLED, None),
            Some(KwinError::Cancelled)
        ));
    }

    #[test]
    fn named_kwin_errors_map_to_kwin_replies_keeping_name_and_detail() {
        let classified = kwin_error_reply(
            "org.kde.KWin.ScreenShot2.Error.InvalidArea",
            Some("Invalid area requested"),
        );
        match classified {
            Some(KwinError::KwinReply { name, message }) => {
                assert!(name.ends_with("InvalidArea"));
                assert_eq!(message.as_deref(), Some("Invalid area requested"));
            }
            other => panic!("expected KwinReply, got {other:?}"),
        }
    }

    #[test]
    fn foreign_error_names_are_not_kwin_replies() {
        assert!(kwin_error_reply("org.freedesktop.DBus.Error.UnknownObject", None).is_none());
        assert!(kwin_error_reply("org.kde.KWin.ScreenShot2.ErrorX", None).is_none());
    }

    #[test]
    fn window_requests_include_decorations_area_requests_do_not() {
        let window = capture_options(&Request::ActiveWindow, true);
        assert_eq!(window.get("include-decoration"), Some(&Value::Bool(true)));
        assert_eq!(window.get("include-cursor"), Some(&Value::Bool(true)));
        let area = capture_options(
            &Request::Area {
                x: 0,
                y: 0,
                width: 4,
                height: 3,
            },
            false,
        );
        assert!(!area.contains_key("include-decoration"));
        assert_eq!(area.get("include-cursor"), Some(&Value::Bool(false)));
        assert_eq!(area.get("native-resolution"), Some(&Value::Bool(true)));
    }
}

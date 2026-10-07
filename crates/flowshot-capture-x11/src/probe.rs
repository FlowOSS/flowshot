//! Session probing: is this an X11 session `FlowShot` can capture, and what
//! does the server offer the capture path?
//!
//! [`probe_x11`] answers the negotiation question (a
//! [`CapabilityProbe`](flowshot_capture::CapabilityProbe) advertising
//! [`BackendKind::X11`](flowshot_capture::BackendKind::X11) or `None`);
//! [`query_caps`] records the extension versions ([`X11Caps`]) the capture
//! path branches on: RANDR for output enumeration, XFIXES for cursor-image
//! capture, MIT-SHM for the fd-passing fast path.

use std::fmt;

use flowshot_capture::{BackendKind, CapabilityProbe, DesktopEnv};
use x11rb::cookie::Cookie;
use x11rb::errors::{ConnectionError, ReplyError};
use x11rb::protocol::{randr, shm, xfixes};
use x11rb::rust_connection::RustConnection;

use crate::connect::X11Connection;
use crate::error::X11Error;

/// The RANDR version this crate asks the server for (`GetMonitors` needs
/// 1.5; the server answers with its own version when older).
const RANDR_CLIENT_VERSION: (u32, u32) = (1, 5);

/// The oldest RANDR this backend works with: the pre-1.5 enumeration
/// fallback calls `GetScreenResourcesCurrent`, a RANDR **1.3** request
/// (the previously advertised 1.2 floor sat below what the code actually
/// issues - a 1.2 server passed the probe, then every enumeration failed
/// as a protocol error).
const RANDR_MIN_VERSION: (u32, u32) = (1, 3);

/// The XFIXES version this crate asks for (`GetCursorImage` itself is 1.0;
/// the answer is the server's own version when older).
pub(crate) const XFIXES_CLIENT_VERSION: (u32, u32) = (5, 0);

/// A server-reported X extension version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionVersion {
    /// Major version.
    pub major: u32,
    /// Minor version.
    pub minor: u32,
}

impl ExtensionVersion {
    /// Creates a version from its parts.
    #[must_use]
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Returns `true` when this version is at least `major`.`minor`.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture_x11::ExtensionVersion;
    ///
    /// let randr = ExtensionVersion::new(1, 5);
    /// assert!(randr.at_least(1, 2));
    /// assert!(!randr.at_least(1, 6));
    /// ```
    #[must_use]
    pub const fn at_least(self, major: u32, minor: u32) -> bool {
        self.major > major || (self.major == major && self.minor >= minor)
    }
}

impl fmt::Display for ExtensionVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// The X extension capabilities of a connected session.
///
/// Queried once per connection ([`query_caps`]); the capture path branches
/// on these: XFIXES drives cursor-image capture (`None` degrades to no
/// cursor painting, never a capture failure), and the MIT-SHM version gates
/// the fd-passing fast path (`None` falls back to plain `GetImage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X11Caps {
    /// The RANDR version; at least 1.3 for this backend to work at all.
    pub randr: ExtensionVersion,
    /// The XFIXES version, when the server provides XFIXES.
    pub xfixes: Option<ExtensionVersion>,
    /// The MIT-SHM version, when the server provides MIT-SHM.
    pub shm: Option<ExtensionVersion>,
}

/// Queries the extension versions the X11 capture path needs.
///
/// # Errors
///
/// Returns [`X11Error::MissingExtension`] when the server does not provide
/// RANDR at all, and [`X11Error::Protocol`] when the RANDR version query
/// fails on the wire. Missing or failing XFIXES/MIT-SHM queries are *not*
/// errors: they yield `None` fields (logged at debug level) because the
/// capture path degrades without them.
pub fn query_caps(conn: &RustConnection) -> Result<X11Caps, X11Error> {
    let randr = query_randr(conn)?;
    let xfixes = query_xfixes(conn);
    let shm = query_shm(conn);
    Ok(X11Caps { randr, xfixes, shm })
}

/// Probes the process environment for a capturable X11 session.
///
/// Returns a probe advertising [`BackendKind::X11`] when `DISPLAY` is set,
/// the X server is reachable, and RANDR >= 1.3 is present; `None` otherwise.
/// Every `None` reason is logged at debug level - probing runs on sessions
/// that are legitimately not X11, so `None` is an answer, not a failure.
///
/// The desktop is reported as [`DesktopEnv::Other`]: the shared vocabulary
/// names Wayland desktops only, and negotiation never filters on it.
#[must_use]
pub fn probe_x11() -> Option<CapabilityProbe> {
    if std::env::var_os("DISPLAY").is_none_or(|display| display.is_empty()) {
        tracing::debug!("DISPLAY is not set; X11 capture is unavailable");
        return None;
    }
    let connection = match X11Connection::connect() {
        Ok(connection) => connection,
        Err(error) => {
            tracing::debug!(%error, "X11 session probe could not connect");
            return None;
        }
    };
    match query_caps(connection.conn()) {
        Ok(caps)
            if caps
                .randr
                .at_least(RANDR_MIN_VERSION.0, RANDR_MIN_VERSION.1) =>
        {
            tracing::info!(
                randr = %caps.randr,
                xfixes = ?caps.xfixes,
                shm = ?caps.shm,
                "X11 session probed; xcb GetImage capture is available"
            );
            Some(CapabilityProbe::new(DesktopEnv::Other, [BackendKind::X11]))
        }
        Ok(caps) => {
            tracing::debug!(
                randr = %caps.randr,
                "RANDR is older than 1.3; X11 capture is unavailable"
            );
            None
        }
        Err(error) => {
            tracing::debug!(%error, "X11 extension query failed; X11 capture is unavailable");
            None
        }
    }
}

/// RANDR is mandatory: absence is a typed [`X11Error::MissingExtension`].
fn query_randr(conn: &RustConnection) -> Result<ExtensionVersion, X11Error> {
    let (major, minor) = RANDR_CLIENT_VERSION;
    let cookie = randr::query_version(conn, major, minor)
        .map_err(|error| missing_extension_or(randr::X11_EXTENSION_NAME, error))?;
    let reply = cookie.reply()?;
    Ok(ExtensionVersion::new(
        reply.major_version,
        reply.minor_version,
    ))
}

/// XFIXES is optional: any failure means "no cursor-image capture".
fn query_xfixes(conn: &RustConnection) -> Option<ExtensionVersion> {
    let (major, minor) = XFIXES_CLIENT_VERSION;
    let queried = xfixes::query_version(conn, major, minor)
        .map_err(ReplyError::from)
        .and_then(Cookie::reply);
    match queried {
        Ok(reply) => Some(ExtensionVersion::new(
            reply.major_version,
            reply.minor_version,
        )),
        Err(error) => {
            tracing::debug!(%error, "XFIXES is unavailable; cursor-image capture will be disabled");
            None
        }
    }
}

/// MIT-SHM is optional: any failure means "plain `GetImage` capture".
fn query_shm(conn: &RustConnection) -> Option<ExtensionVersion> {
    let queried = shm::query_version(conn)
        .map_err(ReplyError::from)
        .and_then(Cookie::reply);
    match queried {
        Ok(reply) => Some(ExtensionVersion::new(
            u32::from(reply.major_version),
            u32::from(reply.minor_version),
        )),
        Err(error) => {
            tracing::debug!(%error, "MIT-SHM is unavailable; capture will use plain GetImage");
            None
        }
    }
}

/// Maps "the server does not have this extension" onto the typed
/// [`X11Error::MissingExtension`] and passes everything else through.
fn missing_extension_or(name: &'static str, error: ConnectionError) -> X11Error {
    match error {
        ConnectionError::UnsupportedExtension => X11Error::MissingExtension { name },
        other => X11Error::from(other),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn at_least_compares_major_then_minor() {
        let version = ExtensionVersion::new(1, 5);
        assert!(version.at_least(1, 2));
        assert!(version.at_least(1, 5));
        assert!(!version.at_least(1, 6));
        assert!(version.at_least(0, 99));
        assert!(!version.at_least(2, 0));
        assert!(ExtensionVersion::new(2, 0).at_least(1, 99));
    }

    #[test]
    fn display_shows_dotted_version() {
        assert_eq!(ExtensionVersion::new(1, 5).to_string(), "1.5");
        assert_eq!(ExtensionVersion::new(1, 15).to_string(), "1.15");
    }
}

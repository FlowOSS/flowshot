//! Connection helper: one `x11rb` connection plus the preferred screen's
//! identity.
//!
//! `x11rb::connect` reports the preferred screen number exactly once, beside
//! the connection, and the [`RustConnection`] itself never remembers it -
//! every screen-scoped request (RANDR enumeration, the `RESOURCE_MANAGER`
//! read, capture) needs that screen's root window. [`X11Connection`] pins
//! connection, screen number, and root window together at connect time so
//! callers can never pair one screen's root with another's connection.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::Window;
use x11rb::rust_connection::RustConnection;

use crate::error::{X11Error, connect_error};

/// A connected X11 session and its preferred screen.
///
/// Create with [`X11Connection::connect`]; hand the inner
/// [`RustConnection`] to the `x11rb` request functions via
/// [`X11Connection::conn`].
#[derive(Debug)]
pub struct X11Connection {
    conn: RustConnection,
    screen: usize,
    root: Window,
}

impl X11Connection {
    /// Connects to the X server named by the `DISPLAY` environment variable
    /// (`x11rb::connect(None)`) and resolves the preferred screen's root
    /// window.
    ///
    /// # Errors
    ///
    /// Returns [`X11Error::Connect`] - carrying an environment-derived
    /// remediation hint - when the server socket is unreachable, and
    /// [`X11Error::Internal`] when the server's setup does not contain the
    /// screen `x11rb` just reported (a server violating its own handshake).
    pub fn connect() -> Result<Self, X11Error> {
        let (conn, screen) = x11rb::connect(None).map_err(connect_error)?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .ok_or(X11Error::Internal(
                "the X server's setup does not contain the screen it reported at connect",
            ))?
            .root;
        Ok(Self { conn, screen, root })
    }

    /// The underlying `x11rb` connection.
    #[must_use]
    pub const fn conn(&self) -> &RustConnection {
        &self.conn
    }

    /// The preferred screen's number (the `.screen` part of `DISPLAY`).
    #[must_use]
    pub const fn screen(&self) -> usize {
        self.screen
    }

    /// The preferred screen's root window.
    #[must_use]
    pub const fn root(&self) -> Window {
        self.root
    }
}

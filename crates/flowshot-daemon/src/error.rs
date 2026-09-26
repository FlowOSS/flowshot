//! Typed daemon errors (Amendment #4: `thiserror`, no stringly errors
//! across crate boundaries; `anyhow` lives only in the binary).

use std::path::PathBuf;

/// Everything that can fail in the daemon library.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// Session-bus connection, name request, or method call failed.
    #[error("D-Bus failure: {0}")]
    Dbus(#[from] zbus::Error),

    /// The well-known name is held by a process that does not answer
    /// `Invoke` - a foreign holder, a stale registration, or a broken
    /// daemon (it replied with an unknown-object error or never replied
    /// at all; `detail` distinguishes).
    #[error(
        "the bus name {service} is held by a process that does not answer Invoke \
         ({detail}); if no FlowShot daemon is visibly running, the registration is \
         stale - restart the session bus or log out and back in"
    )]
    ForeignNameHolder {
        /// The contested well-known name.
        service: String,
        /// The underlying reply/error detail.
        detail: String,
    },

    /// The name owner exited between the ownership probe and the forward;
    /// retrying the whole handshake is the remedy.
    #[error("the running FlowShot daemon released the bus name mid-handshake; retry the command")]
    OwnerVanished,

    /// A `D-Bus` argument could not be parsed into its typed form.
    #[error("invalid argument: {0}")]
    InvalidArgs(String),

    /// Notification delivery failed (bus absent, no notification daemon).
    #[error("notification delivery failed: {0}")]
    Notify(String),

    /// The `OpenURI` portal call failed.
    #[error("OpenURI portal failed: {0}")]
    Portal(String),

    /// `GlobalShortcuts` portal registration failed (absent, denied,
    /// broken, panicked, or timed out). Non-fatal by design: the todo-34
    /// ladder logs it and continues with the compositor-bind fallback.
    #[error("global shortcuts portal unavailable: {0}")]
    ShortcutPortal(String),

    /// Shortcut restore-data (de)serialization failed.
    #[error("shortcut restore data: {0}")]
    ShortcutPersist(#[from] serde_json::Error),

    /// Filesystem operation failed (autostart entry, config I/O).
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),

    /// Config parsing failed.
    #[error("configuration failure: {0}")]
    Config(#[from] flowshot_core::ConfigError),

    /// A required environment variable is missing or not valid Unicode.
    #[error("environment variable {0} is missing or not valid Unicode")]
    Env(&'static str),

    /// A path carried non-UTF-8 components where text was required.
    #[error("path is not valid Unicode: {0:?}")]
    NonUnicodePath(PathBuf),
}

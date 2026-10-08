//! Error types for the actions crate.

use std::path::PathBuf;

use thiserror::Error;

/// Errors produced during export operations.
#[derive(Debug, Error)]
pub enum ExportError {
    /// Filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Image encoding failed.
    #[error("image encoding error: {0}")]
    ImageEncode(String),
    /// Unsupported image format.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    /// Portal (D-Bus) error.
    #[error("portal error: {0}")]
    Portal(String),
    /// User cancelled the save dialog.
    #[error("save cancelled by user")]
    Cancelled,
    /// The configured save directory does not exist and could not be created.
    #[error("save directory does not exist: {}", .0.display())]
    DirectoryNotFound(PathBuf),
}

/// Errors produced during clipboard operations.
#[derive(Debug, Error)]
pub enum ClipboardError {
    /// Transport failure of the underlying display-server clipboard:
    /// Wayland `zwlr_data_control` (`wl-clipboard-rs`) or X11 (`x11rb`).
    /// The boxed source keeps each transport's own error type and its
    /// `Display` quality.
    #[error("clipboard transport error: {0}")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Encoding the capture for the clipboard failed.
    #[error("clipboard image encoding error: {0}")]
    Encode(#[from] ExportError),
    /// X11-specific clipboard failure with no transport source: ICCCM
    /// timestamp acquisition timed out, or the server refused/lost the
    /// `CLIPBOARD` selection ownership.
    #[error("x11 clipboard error: {0}")]
    X11(String),
    /// Neither a Wayland nor an X11 session is reachable from this
    /// environment (`WAYLAND_DISPLAY` and `DISPLAY` both unset or empty).
    #[error("no display session: neither WAYLAND_DISPLAY (Wayland) nor DISPLAY (X11) is set")]
    NoSession,
}

impl ClipboardError {
    /// Box a transport error from any clipboard backend.
    pub(crate) fn transport<E>(err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Transport(Box::new(err))
    }
}

impl From<wl_clipboard_rs::copy::Error> for ClipboardError {
    fn from(err: wl_clipboard_rs::copy::Error) -> Self {
        Self::transport(err)
    }
}

/// Errors produced during upload operations.
#[derive(Debug, Error)]
pub enum UploadError {
    /// The upload provider is not configured (empty `client_id`).
    #[error("upload provider not configured: set [upload].client_id in settings")]
    ConfigurationMissing,
    /// The provider rejected the request as rate-limited (HTTP 429).
    #[error("upload rate-limited by provider; try again later")]
    RateLimited,
    /// The provider returned a response that could not be parsed.
    #[error("invalid upload response: {0}")]
    InvalidResponse(String),
    /// HTTP transport error (non-2xx, non-429).
    #[error("upload HTTP error (status {status}): {message}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Error message from the provider or transport.
        message: String,
    },
    /// Filesystem I/O error (history file, etc.).
    #[error("upload I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// History file could not be read or written.
    #[error("upload history error: {0}")]
    History(String),
    /// The portal (D-Bus) call to open the delete URL failed.
    #[error("upload delete portal error: {0}")]
    Portal(String),
}

// ---- pins ----
/// Errors produced by the pin action helpers ([`crate::pin`]).
#[derive(Debug, Error)]
pub enum PinError {
    /// The pin pixel buffer does not match its declared dimensions.
    #[error("pin image {width}x{height} px needs {expected} RGBA bytes, got {actual}")]
    InvalidBuffer {
        /// Declared width in physical pixels.
        width: u32,
        /// Declared height in physical pixels.
        height: u32,
        /// Byte count the declared dimensions require.
        expected: usize,
        /// Byte count actually provided.
        actual: usize,
    },
    /// The save pipeline failed.
    #[error("pin save failed: {0}")]
    Export(#[from] ExportError),
    /// The clipboard hand-off failed.
    #[error("pin copy failed: {0}")]
    Clipboard(#[from] ClipboardError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_display_keeps_boxed_source_message() {
        let err = ClipboardError::transport(std::io::Error::other("socket gone"));
        assert_eq!(err.to_string(), "clipboard transport error: socket gone");
    }

    #[test]
    fn transport_source_chain_survives_boxing() {
        let err = ClipboardError::transport(std::io::Error::other("boom"));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn wayland_error_converts_without_display_loss() {
        let source = wl_clipboard_rs::copy::Error::NoSeats;
        let message = source.to_string();
        let err = ClipboardError::from(source);
        assert_eq!(
            err.to_string(),
            format!("clipboard transport error: {message}")
        );
    }

    #[test]
    fn x11_variant_names_the_failure() {
        let err = ClipboardError::X11("server refused CLIPBOARD ownership".to_owned());
        assert_eq!(
            err.to_string(),
            "x11 clipboard error: server refused CLIPBOARD ownership"
        );
    }
}

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
    /// Wayland data-control transport failure (`wl-clipboard-rs`).
    #[error("clipboard transport error: {0}")]
    Transport(#[from] wl_clipboard_rs::copy::Error),
    /// Encoding the capture for the clipboard failed.
    #[error("clipboard image encoding error: {0}")]
    Encode(#[from] ExportError),
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

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

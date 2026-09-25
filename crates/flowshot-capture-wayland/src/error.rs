//! Error types for the capture-wayland crate.

use thiserror::Error;

/// Capture errors.
#[derive(Error, Debug)]
pub enum CaptureWaylandError {
    /// An error occurred during Wayland capture.
    #[error("Wayland capture error: {0}")]
    Capture(String),
}

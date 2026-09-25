//! Error types for the capture crate.

use thiserror::Error;

/// Capture errors.
#[derive(Error, Debug)]
pub enum CaptureError {
    /// An error occurred during capture.
    #[error("Capture error: {0}")]
    Capture(String),
}

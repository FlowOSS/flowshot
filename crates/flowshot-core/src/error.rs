//! Error types for the core crate.

use thiserror::Error;

/// Core errors.
#[derive(Error, Debug)]
pub enum CoreError {
    /// An error occurred during capture.
    #[error("Capture error: {0}")]
    Capture(String),
}

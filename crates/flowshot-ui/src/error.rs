//! Error types for the UI crate.

use thiserror::Error;

/// UI errors.
#[derive(Error, Debug)]
pub enum UiError {
    /// An error occurred during UI rendering.
    #[error("UI rendering error: {0}")]
    Rendering(String),
}

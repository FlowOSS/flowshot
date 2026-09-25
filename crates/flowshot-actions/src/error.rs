//! Error types for the actions crate.

use thiserror::Error;

/// Action errors.
#[derive(Error, Debug)]
pub enum ActionsError {
    /// An error occurred during action execution.
    #[error("Action error: {0}")]
    Execution(String),
}

#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Core types and utilities for `FlowShot`.

pub mod config;
pub mod error;
pub mod geometry;
pub mod scene;
pub mod tokens;
pub mod types;

pub use config::{CONFIG_VERSION, Config, ConfigError};
pub use scene::{DEFAULT_UNDO_LIMIT, Scene, SceneError, ToolObject, UndoStack};
pub use tokens::DesignTokens;

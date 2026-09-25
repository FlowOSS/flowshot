#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Action handlers for `FlowShot`.
//!
//! This crate implements the post-capture actions: saving to disk,
//! copying to clipboard, uploading, pinning, and opening with external
//! applications.

pub mod error;
pub mod export;

pub use error::ExportError;

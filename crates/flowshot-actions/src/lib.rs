#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Action handlers for `FlowShot`.
//!
//! This crate implements the post-capture actions: saving to disk,
//! copying to clipboard, uploading, pinning, and opening with external
//! applications.

pub mod clipboard;
pub mod error;
pub mod export;
pub mod upload;

pub use clipboard::Clipboard;
pub use error::{ClipboardError, ExportError, UploadError};
pub use upload::{Imgur, UploadHistory, UploadMeta, UploadRecord, UploadResult, Uploader};

//! Pluggable image uploader.
//!
//! The [`Uploader`] trait abstracts the upload provider; [`Imgur`] is the
//! reference implementation (Imgur API v3). Upload results are recorded
//! in a JSONL history file ([`UploadHistory`]) so the user can revoke
//! past uploads via their delete tokens.
//!
//! # Configuration gate
//!
//! The `[upload].client_id` config ships **empty** by design: no
//! freeloading on a shared anonymous pool. An upload attempt
//! with an empty client id returns [`UploadError::ConfigurationMissing`]
//! immediately — the caller surfaces a settings deep-link hint.

mod delete;
mod history;
mod imgur;

pub use delete::open_delete_url;
pub use history::{UploadHistory, UploadRecord};
pub use imgur::Imgur;

use std::fmt::Debug;

use crate::error::UploadError;

/// Metadata about the image being uploaded.
#[derive(Debug, Clone)]
pub struct UploadMeta {
    /// Original filename (for the provider's title/display).
    pub filename: String,
}

/// Successful upload result.
#[derive(Debug, Clone)]
pub struct UploadResult {
    /// Public URL of the uploaded image.
    pub url: String,
    /// Delete token (provider-specific; for Imgur, the `deletehash`).
    pub delete_hash: String,
}

/// Pluggable upload backend.
///
/// Production uses [`Imgur`]; tests use mock implementations.
#[async_trait::async_trait]
pub trait Uploader: Debug + Send + Sync {
    /// Upload raw image bytes with associated metadata.
    ///
    /// # Errors
    ///
    /// [`UploadError::ConfigurationMissing`] when the provider is not
    /// configured, [`UploadError::RateLimited`] on HTTP 429,
    /// [`UploadError::InvalidResponse`] on unparseable responses,
    /// [`UploadError::Http`] on other non-2xx statuses.
    async fn upload(&self, bytes: &[u8], meta: &UploadMeta) -> Result<UploadResult, UploadError>;
}

//! Delete an uploaded image by opening its revoke URL.

use crate::error::UploadError;

/// Open the Imgur delete URL for the given token in the user's browser.
///
/// # Errors
///
/// Returns [`UploadError::Portal`] if the `OpenURI` portal call fails.
pub async fn open_delete_url(delete_hash: &str) -> Result<(), UploadError> {
    #[cfg(unix)]
    {
        use ashpd::desktop::open_uri::OpenFileRequest;

        let url_str = format!("https://imgur.com/delete/{delete_hash}");
        let url = url::Url::parse(&url_str)
            .map_err(|e| UploadError::Portal(format!("invalid delete URL: {e}")))?;
        let portal_uri = ashpd::Uri::parse(url.as_str())
            .map_err(|e| UploadError::Portal(format!("invalid delete URL: {e}")))?;
        OpenFileRequest::default()
            .send_uri(&portal_uri)
            .await
            .map_err(|e| UploadError::Portal(e.to_string()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = delete_hash;
        Ok(())
    }
}

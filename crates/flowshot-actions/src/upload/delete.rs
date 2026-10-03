//! Delete an uploaded image by opening its revoke URL.
//!
//! Imgur's delete mechanism is a GET to `https://imgur.com/delete/{deletehash}`.
//! We open this URL in the user's browser via the XDG `OpenURI` portal
//! (ashpd), so the user can confirm the deletion on the Imgur page.

use ashpd::desktop::open_uri::OpenFileRequest;

use crate::error::UploadError;

/// Open the Imgur delete URL for the given token in the user's browser.
///
/// # Errors
///
/// Returns [`UploadError::Portal`] if the `OpenURI` portal call fails.
pub async fn open_delete_url(delete_hash: &str) -> Result<(), UploadError> {
    let url_str = format!("https://imgur.com/delete/{delete_hash}");
    let url = url::Url::parse(&url_str)
        .map_err(|e| UploadError::Portal(format!("invalid delete URL: {e}")))?;
    // ashpd 0.13 consumes its own `Uri` type (it dropped `url`); a
    // serialized `Url` always satisfies its minimal parse.
    let portal_uri = ashpd::Uri::parse(url.as_str())
        .map_err(|e| UploadError::Portal(format!("invalid delete URL: {e}")))?;
    OpenFileRequest::default()
        .send_uri(&portal_uri)
        .await
        .map_err(|e| UploadError::Portal(e.to_string()))?;
    Ok(())
}

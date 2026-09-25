//! Open-with-app action via the XDG `OpenURI` portal.

use std::fs::File;
use std::os::fd::AsFd as _;
use std::path::Path;

use ashpd::desktop::open_uri::OpenFileRequest;

use crate::error::ExportError;

/// Open the given file with the default application via the
/// `org.freedesktop.portal.OpenURI` D-Bus portal.
///
/// # Errors
///
/// Returns [`ExportError::Portal`] if the portal call fails.
pub async fn open_with_app(path: &Path) -> Result<(), ExportError> {
    let file = File::open(path)?;
    OpenFileRequest::default()
        .send_file(&file.as_fd())
        .await
        .map_err(|e| ExportError::Portal(e.to_string()))?;
    Ok(())
}

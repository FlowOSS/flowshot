//! The production seam implementations the post-capture pipeline consumes
//! (todo 38): the pictures-directory dialog stand-in (no `rfd` in the
//! workspace) and the daemon-notifier bridge.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use flowshot_actions::ExportError;
use flowshot_actions::export::{FileDialogSink, NotifySink};

use crate::notify::{NotificationRecord, Notifier};

/// The production save-path dialog stand-in: `rfd` is not a workspace
/// dependency, and the README contract for an empty `[save].path` is "the
/// platform pictures directory" - so the dialog resolves to exactly that
/// (XDG `user-dirs.dirs`, `$HOME/Pictures` fallback), never prompting.
#[derive(Debug, Clone, Copy)]
pub struct PicturesDirDialog;

impl FileDialogSink for PicturesDirDialog {
    fn pick_save_path(&self, default_name: &str) -> Result<Option<PathBuf>, ExportError> {
        Ok(Some(pictures_dir()?.join(default_name)))
    }
}

fn pictures_dir() -> Result<PathBuf, ExportError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| ExportError::DirectoryNotFound(PathBuf::from("$HOME")))?;
    let user_dirs = home.join(".config").join("user-dirs.dirs");
    if let Ok(contents) = std::fs::read_to_string(&user_dirs) {
        for line in contents.lines() {
            let Some(value) = line.strip_prefix("XDG_PICTURES_DIR=") else {
                continue;
            };
            let trimmed = value.trim_matches('"');
            let expanded = trimmed.replace("$HOME", &home.to_string_lossy());
            return Ok(PathBuf::from(expanded));
        }
    }
    Ok(home.join("Pictures"))
}

/// Bridges the actions-crate notification seam onto the daemon's
/// [`Notifier`] (portal toasts, `[daemon].notifications`-gated by the
/// pipeline's `notifications_enabled`).
#[derive(Debug)]
pub(super) struct NotifyBridge {
    notifier: Arc<dyn Notifier>,
}

impl NotifyBridge {
    pub(super) fn new(notifier: Arc<dyn Notifier>) -> Self {
        Self { notifier }
    }
}

impl NotifySink for NotifyBridge {
    fn on_saved(&self, path: &Path) {
        self.notifier
            .notify(NotificationRecord::Saved(path.to_path_buf()));
    }

    fn on_error(&self, message: &str) {
        self.notifier
            .notify(NotificationRecord::Error(message.to_owned()));
    }

    fn on_success(&self, saved_path: Option<&Path>) {
        self.notifier.notify(match saved_path {
            Some(path) => NotificationRecord::Saved(path.to_path_buf()),
            None => NotificationRecord::Captured,
        });
    }
}

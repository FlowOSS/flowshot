//! The production seam implementations the post-capture pipeline consumes:
//! the pictures-directory dialog stand-in (no `rfd` in the
//! workspace), the daemon-notifier bridge, and the clipboard
//! hold-release bridge.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use flowshot_actions::ExportError;
use flowshot_actions::clipboard::{Clipboard, OfferLossHook};
use flowshot_actions::export::{FileDialogSink, NotifySink};

use crate::notify::{NotificationRecord, Notifier};
use crate::state::DaemonState;

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

/// The clipboard hold-release bridge for one post-capture run: the
/// session clipboard plus the "offer lost" latch.
///
/// X11: the backend's serving thread observes `SelectionClear` when
/// another client takes the clipboard and fires the loss hook, which
/// latches the loss and releases the daemon's `clipboard-offer`
/// persistence reason - the auto-spawned daemon then idle-exits after
/// its grace instead of staying pinned by an offer it no longer owns
/// (the F1 review concern; live-verified 2026-10-04).
///
/// Wayland: `wl-clipboard-rs` has no replaced callback - the hook never
/// fires and the conservative never-clear behavior stands (documented
/// asymmetry, `flowshot_actions::clipboard::OfferLossHook`).
///
/// The latch covers the ordering hazard of a supersede DURING the
/// pipeline run (the hook can fire before the pipeline reports
/// `Copied`): the post-run `held(true)` must not re-pin an already-lost
/// offer.
pub(super) struct ClipboardHoldRelease {
    /// The session clipboard, wired with the loss hook.
    pub clipboard: Clipboard,
    /// Set once the served offer was lost to another client.
    pub lost: Arc<AtomicBool>,
}

/// Builds the session clipboard for one post-capture run.
///
/// # Errors
///
/// [`flowshot_actions::ClipboardError::NoSession`] when neither
/// `WAYLAND_DISPLAY` nor `DISPLAY` is set.
pub(super) fn clipboard_for_run(
    state: Option<&Arc<DaemonState>>,
) -> Result<ClipboardHoldRelease, flowshot_actions::ClipboardError> {
    let lost = Arc::new(AtomicBool::new(false));
    let hook = hold_release_hook(state.cloned(), Arc::clone(&lost));
    Ok(ClipboardHoldRelease {
        clipboard: Clipboard::for_session_with_loss_hook(hook)?,
        lost,
    })
}

/// The hook body (split out for the headless transition tests): latch
/// the loss, then release the persistence reason.
fn hold_release_hook(state: Option<Arc<DaemonState>>, lost: Arc<AtomicBool>) -> OfferLossHook {
    Arc::new(move || {
        lost.store(true, Ordering::Release);
        if let Some(state) = &state {
            tracing::info!(
                "clipboard offer lost to another client; releasing the clipboard-offer hold"
            );
            state.set_clipboard_offer_held(false);
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::time::Instant;

    #[test]
    fn loss_hook_latches_and_releases_the_hold() {
        // Given: a daemon pinned by its clipboard offer.
        let state = Arc::new(DaemonState::new(false, Instant::now()));
        state.set_clipboard_offer_held(true);
        let lost = Arc::new(AtomicBool::new(false));
        let hook = hold_release_hook(Some(Arc::clone(&state)), Arc::clone(&lost));
        // When: the backend reports the offer lost to another client.
        hook();
        // Then: the persistence reason is released and the latch is set.
        assert!(!state.reasons().clipboard_offer);
        assert!(!state.reasons().any());
        assert!(lost.load(Ordering::Acquire));
    }

    #[test]
    fn loss_hook_without_daemon_state_only_latches() {
        // The one-shot CLI process has no daemon state: the hook must
        // still latch (and must not panic).
        let lost = Arc::new(AtomicBool::new(false));
        let hook = hold_release_hook(None, Arc::clone(&lost));
        hook();
        assert!(lost.load(Ordering::Acquire));
    }
}

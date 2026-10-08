//! The production seam implementations the post-capture pipeline consumes:
//! the pictures-directory dialog stand-in (no `rfd` in the
//! workspace), the daemon-notifier bridge, and the clipboard
//! hold-release bridge.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use flowshot_actions::ClipboardError;
use flowshot_actions::ExportError;
use flowshot_actions::clipboard::{Clipboard, ClipboardBackend, ClipboardOffer, OfferLossHook};
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
pub(in crate::execute) struct ClipboardHoldRelease {
    /// The session clipboard, wired with the loss hook.
    pub clipboard: Clipboard,
    /// Set once the served offer was lost to another client.
    pub lost: Arc<AtomicBool>,
}

/// The inert clipboard for runs with no usable display session (headless
/// CI, ssh-spawned daemons, containers): every serve fails with the typed
/// [`ClipboardError::NoSession`], which the pipeline records as a failed
/// Copy outcome while save/upload/notify/pin still run - `main`'s
/// pre-session-routing semantics, where clipboard construction was
/// infallible and a copy failure was just another outcome.
#[derive(Debug)]
struct NoSessionClipboard;

impl ClipboardBackend for NoSessionClipboard {
    fn serve(&self, _offer: ClipboardOffer) -> Result<(), ClipboardError> {
        Err(ClipboardError::NoSession)
    }
}

/// Builds the session clipboard for one post-capture run.
///
/// Infallible by contract: a daemon with no display session gets the inert
/// [`NoSessionClipboard`] instead of an early return, so one unavailable
/// clipboard can never discard an already-successful capture (a hoisted
/// fallible construction once aborted the whole pipeline headless,
/// discarding successful captures and reddening the untouched
/// `upload_e2e` tests on CI).
pub(in crate::execute) fn clipboard_for_run(
    state: Option<&Arc<DaemonState>>,
) -> ClipboardHoldRelease {
    let lost = Arc::new(AtomicBool::new(false));
    let hook = hold_release_hook(state.cloned(), Arc::clone(&lost));
    let clipboard = match Clipboard::for_session_with_loss_hook(hook) {
        Ok(clipboard) => clipboard,
        Err(error) => {
            tracing::debug!(%error, "clipboard unavailable; copy degrades to a recorded outcome");
            Clipboard::new(NoSessionClipboard)
        }
    };
    ClipboardHoldRelease { clipboard, lost }
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

/// Pins the daemon's `clipboard-offer` persistence reason after a
/// successfully served offer - unless the loss latch says the offer was
/// already superseded during the copy: the hook released the reason, and
/// re-pinning would resurrect a hold nothing owns. Shared by the capture
/// pipeline and the color pick so both clipboard owners age out the same
/// way (X11: the hook fires on `SelectionClear`; Wayland: no replaced
/// callback, the documented conservative never-clear stands).
pub(in crate::execute) fn hold_offer_unless_lost(
    state: Option<&Arc<DaemonState>>,
    lost: &AtomicBool,
) {
    if lost.load(Ordering::Acquire) {
        return;
    }
    if let Some(state) = state {
        state.set_clipboard_offer_held(true);
    }
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

    #[test]
    fn hold_offer_unless_lost_pins_only_a_surviving_offer() {
        // A served offer with no takeover pins the daemon (the reason the
        // color pick and the capture pipeline both hold for).
        let state = Arc::new(DaemonState::new(false, Instant::now()));
        hold_offer_unless_lost(Some(&state), &AtomicBool::new(false));
        assert!(state.reasons().clipboard_offer);
        // Superseded DURING the copy: the hook already released, so the
        // pin must NOT resurrect the hold (the color-path regression).
        let raced = Arc::new(DaemonState::new(false, Instant::now()));
        hold_offer_unless_lost(Some(&raced), &AtomicBool::new(true));
        assert!(!raced.reasons().clipboard_offer);
        // One-shot CLI (no daemon state): never panics, nothing to pin.
        hold_offer_unless_lost(None, &AtomicBool::new(false));
    }

    #[test]
    fn no_session_clipboard_fails_typed_through_the_facade() {
        // The inert backend headless `clipboard_for_run` degrades to: the
        // facade must surface the typed NoSession error - the value the
        // pipeline records as a failed Copy outcome - not panic or hang.
        let clipboard = Clipboard::new(NoSessionClipboard);
        let result = clipboard.copy_text("flowshot");
        assert!(matches!(result, Err(ClipboardError::NoSession)));
    }
}

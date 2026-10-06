//! Post-capture action executor.
//!
//! Runs an effective action sequence (see [`super::actions`]) against a
//! captured image: save (the export pipeline), clipboard copy,
//! `copy-path` with the uri-list-appended rule, open-with, upload, and
//! the `notify` toast — all gated and recorded best-effort. Pin reports
//! [`ActionOutcome::Deferred`] until its module lands; the daemon drives
//! this executor.

use std::fmt::{self, Display};
use std::path::{Path, PathBuf};

use flowshot_core::config::SaveConfig;
use image::DynamicImage;

use super::Clipboard;
use super::actions::{Action, execution_order};
use crate::export::{FileDialogSink, NotifySink, open_with_app, save};
use crate::upload::{UploadHistory, UploadMeta, UploadRecord, Uploader};

/// What happened for one executed action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionOutcome {
    /// Image copied to the clipboard.
    Copied,
    /// Image saved to disk.
    Saved(PathBuf),
    /// Saved file's path copied to the clipboard.
    PathCopied(PathBuf),
    /// `copy-path` ran without a successful save: warned + no-op.
    PathCopyNoSave,
    /// Saved file opened with the default application.
    OpenedWith,
    /// `open-with` ran without a successful save: warned + no-op.
    OpenWithNoSave,
    /// Image uploaded successfully.
    Uploaded {
        /// Public URL of the uploaded image.
        url: String,
    },
    /// Success toast requested (`notify` action, notifications enabled).
    Notified,
    /// `notify` suppressed by the `[daemon].notifications` gate.
    NotificationGated,
    /// Action module not landed yet (pin).
    Deferred(Action),
    /// Action failed; the sequence continued best-effort.
    Failed {
        /// The action that failed.
        action: Action,
        /// Underlying error message.
        message: String,
    },
}

/// Result of a full post-capture run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostCaptureReport {
    /// Path of the last successful save, if any.
    pub saved_path: Option<PathBuf>,
    /// Per-action outcomes in execution order.
    pub outcomes: Vec<ActionOutcome>,
}

/// Everything the executor needs (grouped context object).
pub struct PostCapture<'a> {
    /// The captured (and edited) image.
    pub image: &'a DynamicImage,
    /// `[save]` configuration.
    pub save_config: &'a SaveConfig,
    /// `[daemon].notifications` gate — gates ALL notifications.
    pub notifications_enabled: bool,
    /// Clipboard facade (daemon-owned backend in production).
    pub clipboard: &'a Clipboard,
    /// File-dialog seam.
    pub dialog: &'a dyn FileDialogSink,
    /// Notification seam (the daemon wires it to `notify-rust`).
    pub notify: &'a dyn NotifySink,
    /// Upload backend. `None` = upload action deferred.
    pub uploader: Option<&'a dyn Uploader>,
    /// Upload history for recording successful uploads. `None` = skip
    /// history recording.
    pub upload_history: Option<&'a UploadHistory>,
    /// Whether to copy the upload URL to the clipboard after success.
    pub copy_upload_url: bool,
}

impl fmt::Debug for PostCapture<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostCapture")
            .field("save_config", &self.save_config)
            .field("notifications_enabled", &self.notifications_enabled)
            .field("has_uploader", &self.uploader.is_some())
            .field("copy_upload_url", &self.copy_upload_url)
            .finish_non_exhaustive()
    }
}

/// Run the post-capture action sequence.
///
/// Ordering: [`execution_order`] — every `copy-path` runs after the last
/// `save`. Semantics:
///
/// - best-effort: a failing action is recorded as
///   [`ActionOutcome::Failed`] and the rest still run;
/// - `copy-path` / `open-with` without a successful save warn + no-op;
/// - when `copy-path` runs after a `copy` in the same sequence, the
///   path's `text/uri-list` + `text/plain` are APPENDED to a fresh
///   combined image+path offer (data-control offers replace, so the
///   image is re-served in the same offer);
/// - ALL notifications (including save's own success toast) pass the
///   `notifications_enabled` gate; the `notify` action is an explicit
///   success-toast request on top.
#[must_use = "the report carries saved paths and per-action outcomes"]
pub async fn run_post_capture(actions: &[Action], ctx: &PostCapture<'_>) -> PostCaptureReport {
    let gated = GatedNotify {
        inner: ctx.notify,
        enabled: ctx.notifications_enabled,
    };
    let mut report = PostCaptureReport::default();
    let mut image_on_clipboard = false;

    for action in execution_order(actions) {
        match action {
            Action::Save => match save(ctx.image, ctx.save_config, ctx.dialog, &gated) {
                Ok(path) => {
                    report.saved_path = Some(path.clone());
                    report.outcomes.push(ActionOutcome::Saved(path));
                }
                Err(err) => report.outcomes.push(failed(Action::Save, err)),
            },
            Action::Copy => match ctx.clipboard.copy_capture(ctx.image, ctx.save_config) {
                Ok(()) => {
                    image_on_clipboard = true;
                    report.outcomes.push(ActionOutcome::Copied);
                }
                Err(err) => report.outcomes.push(failed(Action::Copy, err)),
            },
            Action::CopyPath => {
                let Some(path) = report.saved_path.as_deref() else {
                    tracing::warn!(action = "copy-path", "no save occurred; skipping path copy");
                    report.outcomes.push(ActionOutcome::PathCopyNoSave);
                    continue;
                };
                let result = if image_on_clipboard {
                    ctx.clipboard
                        .copy_capture_and_path(ctx.image, ctx.save_config, path)
                } else {
                    ctx.clipboard.copy_file_path(path)
                };
                match result {
                    Ok(()) => {
                        report
                            .outcomes
                            .push(ActionOutcome::PathCopied(path.to_path_buf()));
                    }
                    Err(err) => report.outcomes.push(failed(Action::CopyPath, err)),
                }
            }
            Action::OpenWith => {
                let Some(path) = report.saved_path.as_deref() else {
                    tracing::warn!(action = "open-with", "no save occurred; skipping open-with");
                    report.outcomes.push(ActionOutcome::OpenWithNoSave);
                    continue;
                };
                match open_with_app(path).await {
                    Ok(()) => report.outcomes.push(ActionOutcome::OpenedWith),
                    Err(err) => report.outcomes.push(failed(Action::OpenWith, err)),
                }
            }
            Action::Notify => {
                if ctx.notifications_enabled {
                    ctx.notify.on_success(report.saved_path.as_deref());
                    report.outcomes.push(ActionOutcome::Notified);
                } else {
                    tracing::debug!(action = "notify", "notifications disabled; toast gated");
                    report.outcomes.push(ActionOutcome::NotificationGated);
                }
            }
            Action::Upload => {
                handle_upload(ctx, &mut report).await;
            }
            Action::Pin => {
                tracing::debug!(action = "pin", "action module not landed yet; deferred");
                report.outcomes.push(ActionOutcome::Deferred(Action::Pin));
            }
        }
    }
    report
}

fn failed(action: Action, err: impl Display) -> ActionOutcome {
    ActionOutcome::Failed {
        action,
        message: err.to_string(),
    }
}

async fn handle_upload(ctx: &PostCapture<'_>, report: &mut PostCaptureReport) {
    let Some(uploader) = ctx.uploader else {
        tracing::debug!(action = "upload", "no uploader configured; deferred");
        report
            .outcomes
            .push(ActionOutcome::Deferred(Action::Upload));
        return;
    };
    let bytes = match crate::export::encode_png(ctx.image) {
        Ok(b) => b,
        Err(err) => {
            report.outcomes.push(failed(Action::Upload, err));
            return;
        }
    };
    let meta = UploadMeta {
        filename: "capture.png".to_owned(),
    };
    match uploader.upload(&bytes, &meta).await {
        Ok(result) => {
            if let Some(history) = ctx.upload_history {
                let record = UploadRecord {
                    url: result.url.clone(),
                    delete_hash: result.delete_hash,
                    timestamp: chrono::Utc::now(),
                    filename: meta.filename,
                };
                if let Err(err) = history.append(&record) {
                    tracing::warn!(%err, "failed to record upload in history");
                }
            }
            if ctx.copy_upload_url
                && let Err(err) = ctx.clipboard.copy_text(&result.url)
            {
                tracing::warn!(%err, "failed to copy upload URL to clipboard");
            }
            report
                .outcomes
                .push(ActionOutcome::Uploaded { url: result.url });
        }
        Err(err) => report.outcomes.push(failed(Action::Upload, err)),
    }
}

/// Applies the `[daemon].notifications` gate to ALL notifications routed
/// through the sink, including the save pipeline's own.
struct GatedNotify<'a> {
    inner: &'a dyn NotifySink,
    enabled: bool,
}

impl NotifySink for GatedNotify<'_> {
    fn on_saved(&self, path: &Path) {
        if self.enabled {
            self.inner.on_saved(path);
        }
    }

    fn on_error(&self, message: &str) {
        if self.enabled {
            self.inner.on_error(message);
        }
    }

    fn on_success(&self, saved_path: Option<&Path>) {
        if self.enabled {
            self.inner.on_success(saved_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::MockClipboard;
    use crate::clipboard::offer::{ClipboardOffer, MIME_PNG, MIME_TEXT_PLAIN, MIME_URI_LIST};
    use crate::error::ExportError;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Mutex, PoisonError};

    #[derive(Debug)]
    struct MockDialog {
        response: Result<Option<PathBuf>, ExportError>,
    }

    impl FileDialogSink for MockDialog {
        fn pick_save_path(&self, _default_name: &str) -> Result<Option<PathBuf>, ExportError> {
            match &self.response {
                Ok(path) => Ok(path.clone()),
                Err(err) => Err(ExportError::Io(std::io::Error::other(err.to_string()))),
            }
        }
    }

    #[derive(Debug, Default)]
    struct RecordingNotify {
        saved: Mutex<Vec<PathBuf>>,
        success: AtomicUsize,
        errors: AtomicUsize,
    }

    impl RecordingNotify {
        fn saved_paths(&self) -> Vec<PathBuf> {
            self.saved
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
        fn success_count(&self) -> usize {
            self.success.load(Ordering::SeqCst)
        }
    }

    impl NotifySink for RecordingNotify {
        fn on_saved(&self, path: &Path) {
            self.saved
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(path.to_path_buf());
        }
        fn on_error(&self, _message: &str) {
            self.errors.fetch_add(1, Ordering::SeqCst);
        }
        fn on_success(&self, _saved_path: Option<&Path>) {
            self.success.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn unique_tempdir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "flowshot-actions-clipboard-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
            counter
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Fixed fixture set for one executor run.
    struct Fixture {
        image: DynamicImage,
        config: SaveConfig,
        mock: MockClipboard,
        clipboard: Clipboard,
        dialog: MockDialog,
        notify: RecordingNotify,
    }

    impl Fixture {
        fn new(config: SaveConfig, dialog: Result<Option<PathBuf>, ExportError>) -> Self {
            let mock = MockClipboard::new();
            Self {
                image: DynamicImage::new_rgb8(2, 2),
                config,
                clipboard: Clipboard::new(mock.clone()),
                mock,
                dialog: MockDialog { response: dialog },
                notify: RecordingNotify::default(),
            }
        }

        fn context(&self) -> PostCapture<'_> {
            PostCapture {
                image: &self.image,
                save_config: &self.config,
                notifications_enabled: true,
                clipboard: &self.clipboard,
                dialog: &self.dialog,
                notify: &self.notify,
                uploader: None,
                upload_history: None,
                copy_upload_url: false,
            }
        }
    }

    fn save_config(dir: &Path) -> SaveConfig {
        SaveConfig {
            path: dir.to_string_lossy().into_owned(),
            path_fixed: true,
            ..SaveConfig::default()
        }
    }

    #[tokio::test]
    async fn copy_action_offers_encoded_png() {
        let fixture = Fixture::new(SaveConfig::default(), Ok(None));
        let report = run_post_capture(&[Action::Copy], &fixture.context()).await;
        assert_eq!(report.outcomes, [ActionOutcome::Copied]);
        let offer = fixture.mock.last_offer().unwrap_or_default();
        assert_eq!(offer.entries.len(), 1);
        assert_eq!(offer.entries[0].mime, MIME_PNG);
        assert!(offer.entries[0].data.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[tokio::test]
    async fn save_then_copy_path_offers_plain_and_uri_list() {
        let dir = unique_tempdir();
        let fixture = Fixture::new(save_config(&dir), Ok(None));
        let report = run_post_capture(&[Action::Save, Action::CopyPath], &fixture.context()).await;
        let ActionOutcome::Saved(saved) = &report.outcomes[0] else {
            panic!("expected Saved, got {:?}", report.outcomes[0]);
        };
        assert!(saved.starts_with(&dir));
        assert!(saved.exists());
        assert_eq!(report.outcomes[1], ActionOutcome::PathCopied(saved.clone()));
        let offer = fixture.mock.last_offer().unwrap_or_default();
        let mimes: Vec<&str> = offer.entries.iter().map(|e| e.mime.as_str()).collect();
        assert_eq!(mimes, [MIME_TEXT_PLAIN, MIME_URI_LIST]);
        assert_eq!(
            offer.entries[0].data.as_ref(),
            saved.as_os_str().as_encoded_bytes()
        );
        let uri = String::from_utf8_lossy(&offer.entries[1].data);
        assert!(uri.starts_with("file://"), "uri was: {uri}");
        assert!(uri.ends_with("\r\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn copy_path_after_copy_appends_uri_list_to_image_offer() {
        let dir = unique_tempdir();
        let fixture = Fixture::new(save_config(&dir), Ok(None));
        let report = run_post_capture(
            &[Action::Copy, Action::Save, Action::CopyPath],
            &fixture.context(),
        )
        .await;
        assert_eq!(report.outcomes[0], ActionOutcome::Copied);
        let ActionOutcome::Saved(saved) = &report.outcomes[1] else {
            panic!("expected Saved, got {:?}", report.outcomes[1]);
        };
        assert!(saved.exists());
        assert!(matches!(report.outcomes[2], ActionOutcome::PathCopied(_)));
        // Combined offer: image first, path entries appended.
        let offer = fixture.mock.last_offer().unwrap_or_default();
        let mimes: Vec<&str> = offer.entries.iter().map(|e| e.mime.as_str()).collect();
        assert_eq!(mimes, [MIME_PNG, MIME_TEXT_PLAIN, MIME_URI_LIST]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn copy_path_without_save_warns_and_no_ops() {
        let fixture = Fixture::new(SaveConfig::default(), Ok(None));
        let report = run_post_capture(&[Action::CopyPath], &fixture.context()).await;
        assert_eq!(report.outcomes, [ActionOutcome::PathCopyNoSave]);
        assert_eq!(fixture.mock.offers(), [] as [ClipboardOffer; 0]);
    }

    #[tokio::test]
    async fn copy_path_is_reordered_after_save() {
        let dir = unique_tempdir();
        let fixture = Fixture::new(save_config(&dir), Ok(None));
        // Configured order has copy-path FIRST; execution order moves it
        // after the save so the path exists.
        let report = run_post_capture(&[Action::CopyPath, Action::Save], &fixture.context()).await;
        assert!(matches!(report.outcomes[0], ActionOutcome::Saved(_)));
        assert!(matches!(report.outcomes[1], ActionOutcome::PathCopied(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn failed_save_makes_copy_path_no_op() {
        let dialog_err = Err(ExportError::Io(std::io::Error::other("dialog exploded")));
        let fixture = Fixture::new(SaveConfig::default(), dialog_err);
        // Empty config.path forces the dialog, which fails.
        let report = run_post_capture(&[Action::Save, Action::CopyPath], &fixture.context()).await;
        assert!(matches!(
            report.outcomes[0],
            ActionOutcome::Failed {
                action: Action::Save,
                ..
            }
        ));
        assert_eq!(report.outcomes[1], ActionOutcome::PathCopyNoSave);
        assert!(report.saved_path.is_none());
    }

    #[tokio::test]
    async fn open_with_without_save_warns_and_no_ops() {
        let fixture = Fixture::new(SaveConfig::default(), Ok(None));
        let report = run_post_capture(&[Action::OpenWith], &fixture.context()).await;
        assert_eq!(report.outcomes, [ActionOutcome::OpenWithNoSave]);
    }

    #[tokio::test]
    async fn notify_action_requests_toast_when_enabled() {
        let fixture = Fixture::new(SaveConfig::default(), Ok(None));
        let report = run_post_capture(&[Action::Notify], &fixture.context()).await;
        assert_eq!(report.outcomes, [ActionOutcome::Notified]);
        assert_eq!(fixture.notify.success_count(), 1);
    }

    #[tokio::test]
    async fn notifications_gate_suppresses_notify_and_save_toasts() {
        let dir = unique_tempdir();
        let fixture = Fixture::new(save_config(&dir), Ok(None));
        let mut ctx = fixture.context();
        ctx.notifications_enabled = false;
        let report = run_post_capture(&[Action::Save, Action::Notify], &ctx).await;
        assert!(matches!(report.outcomes[0], ActionOutcome::Saved(_)));
        assert_eq!(report.outcomes[1], ActionOutcome::NotificationGated);
        // Gate covers ALL notifications: save's own toast stayed silent.
        assert_eq!(fixture.notify.saved_paths(), [] as [PathBuf; 0]);
        assert_eq!(fixture.notify.success_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn save_notifications_pass_when_enabled() {
        let dir = unique_tempdir();
        let fixture = Fixture::new(save_config(&dir), Ok(None));
        let report = run_post_capture(&[Action::Save], &fixture.context()).await;
        assert!(matches!(report.outcomes[0], ActionOutcome::Saved(_)));
        assert_eq!(fixture.notify.saved_paths().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn pin_is_deferred_and_upload_deferred_without_uploader() {
        let fixture = Fixture::new(SaveConfig::default(), Ok(None));
        let report = run_post_capture(&[Action::Pin, Action::Upload], &fixture.context()).await;
        assert_eq!(
            report.outcomes,
            [
                ActionOutcome::Deferred(Action::Pin),
                ActionOutcome::Deferred(Action::Upload)
            ]
        );
    }

    #[tokio::test]
    async fn clipboard_failure_is_recorded_and_sequence_continues() {
        let fixture = Fixture::new(SaveConfig::default(), Ok(None));
        fixture.mock.set_failing(true);
        let report = run_post_capture(&[Action::Copy, Action::Notify], &fixture.context()).await;
        assert!(matches!(
            report.outcomes[0],
            ActionOutcome::Failed {
                action: Action::Copy,
                ..
            }
        ));
        assert_eq!(report.outcomes[1], ActionOutcome::Notified);
    }
}

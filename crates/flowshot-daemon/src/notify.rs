//! Notification vocabulary and seams (plan todo 32; Flameshot's
//! `showDesktopNotification` / `showAbortNotification` folded into the
//! `[daemon].notifications` gate per Amendment #3).
//!
//! Layering: consumers (the actions pipeline bridge, the todo-33 tray, the
//! todo-35 capture executor) talk to the [`Notifier`] trait; the
//! production implementation ([`DesktopNotifier`]) owns `notify-rust` and
//! the click-action -> `OpenURI` portal wiring; [`GatedNotifier`] applies
//! the config gate; [`RecordingNotifier`] captures dispatches in tests.

mod desktop;

pub use desktop::{DesktopNotifier, PortalUriOpener, UriOpener, show_blocking};

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use flowshot_actions::export::NotifySink;

/// What a [`Notifier`] dispatched (test/inspection vocabulary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationRecord {
    /// Post-save toast; payload is the saved file path (click opens it).
    Saved(PathBuf),
    /// Post-upload toast; payload is the URL (click opens it).
    Uploaded(String),
    /// Plain success toast (the `notify` action without a saved path).
    Captured,
    /// Capture aborted.
    Aborted,
    /// Error toast; payload is the message.
    Error(String),
    /// ONE-TIME nudge after the first successful portal shortcut
    /// registration (todo 34): recommends autostart + the settings
    /// surface. Emitted at most once per install (the restore-data file
    /// carries the notified flag).
    ShortcutsRegistered,
    /// Tray `About` entry (todo 33): the payload is the toast body
    /// ([`crate::strings::about_body`]). A full About surface lands with
    /// the todo-36 settings stack - recorded deviation.
    About(String),
}

/// The notification seam: every user-visible toast in the product goes
/// through this trait so dispatch is capturable without a bus.
pub trait Notifier: Send + Sync + std::fmt::Debug {
    /// Dispatch one notification.
    fn notify(&self, record: NotificationRecord);
}

/// Config gate: `[daemon].notifications = false` suppresses every toast
/// (the Amendment-#3 single bool replacing Flameshot's
/// `showDesktopNotification` + `showAbortNotification` pair).
#[derive(Debug)]
pub struct GatedNotifier {
    inner: Arc<dyn Notifier>,
    enabled: bool,
}

impl GatedNotifier {
    /// Wraps `inner` behind the `enabled` gate.
    #[must_use]
    pub fn new(inner: Arc<dyn Notifier>, enabled: bool) -> Self {
        Self { inner, enabled }
    }
}

impl Notifier for GatedNotifier {
    fn notify(&self, record: NotificationRecord) {
        if self.enabled {
            self.inner.notify(record);
        } else {
            tracing::debug!(record = ?record, "notification suppressed by [daemon].notifications");
        }
    }
}

/// Captures dispatches in memory (the plan's "notification dispatch
/// captured via mock" test seam).
#[derive(Debug, Clone, Default)]
pub struct RecordingNotifier {
    records: Arc<Mutex<Vec<NotificationRecord>>>,
}

impl RecordingNotifier {
    /// An empty recorder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every record dispatched so far, in order.
    #[must_use]
    pub fn records(&self) -> Vec<NotificationRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Notifier for RecordingNotifier {
    fn notify(&self, record: NotificationRecord) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

/// Bridges the todo-28/29 [`NotifySink`] seam (flowshot-actions) onto a
/// [`Notifier`]: the actions pipeline toasts through the daemon's
/// notification module (Metis #18 placement resolution).
#[derive(Debug)]
pub struct ActionNotifyBridge {
    notifier: Arc<dyn Notifier>,
}

impl ActionNotifyBridge {
    /// Bridges into `notifier` (already gated by the caller when the
    /// pipeline's own `notifications_enabled` gate is in play).
    #[must_use]
    pub fn new(notifier: Arc<dyn Notifier>) -> Self {
        Self { notifier }
    }
}

impl NotifySink for ActionNotifyBridge {
    fn on_saved(&self, path: &Path) {
        self.notifier
            .notify(NotificationRecord::Saved(path.to_path_buf()));
    }

    fn on_error(&self, message: &str) {
        self.notifier
            .notify(NotificationRecord::Error(message.to_owned()));
    }

    fn on_success(&self, saved_path: Option<&Path>) {
        match saved_path {
            Some(path) => self
                .notifier
                .notify(NotificationRecord::Saved(path.to_path_buf())),
            None => self.notifier.notify(NotificationRecord::Captured),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gated_notifier_passes_through_when_enabled() {
        let recorder = RecordingNotifier::new();
        let gated = GatedNotifier::new(Arc::new(recorder.clone()), true);
        gated.notify(NotificationRecord::Captured);
        assert_eq!(recorder.records(), vec![NotificationRecord::Captured]);
    }

    #[test]
    fn gated_notifier_suppresses_when_disabled() {
        let recorder = RecordingNotifier::new();
        let gated = GatedNotifier::new(Arc::new(recorder.clone()), false);
        gated.notify(NotificationRecord::Aborted);
        gated.notify(NotificationRecord::Error("boom".to_owned()));
        assert!(recorder.records().is_empty());
    }

    #[test]
    fn bridge_maps_every_notify_sink_call() {
        let recorder = RecordingNotifier::new();
        let bridge = ActionNotifyBridge::new(Arc::new(recorder.clone()));
        bridge.on_saved(Path::new("/tmp/shot.png"));
        bridge.on_error("encode failed");
        bridge.on_success(Some(Path::new("/tmp/shot2.png")));
        bridge.on_success(None);
        assert_eq!(
            recorder.records(),
            vec![
                NotificationRecord::Saved(PathBuf::from("/tmp/shot.png")),
                NotificationRecord::Error("encode failed".to_owned()),
                NotificationRecord::Saved(PathBuf::from("/tmp/shot2.png")),
                NotificationRecord::Captured,
            ]
        );
    }
}

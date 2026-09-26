//! Production notifier: `notify-rust` toasts + click-action -> `OpenURI`
//! portal (plan todo 32: click-action -> `OpenURI` on the saved path).
//!
//! Threading model: `notify-rust`'s `show()` blocks on its own zbus-5
//! stack and `wait_for_action` blocks until the notification is acted on
//! or closed, so every toast runs on a short-lived dedicated thread -
//! never on an async worker. The click handler opens the URI through a
//! [`UriOpener`]; the production opener builds a private current-thread
//! tokio runtime for the `ashpd` portal call (the todo-10 worker pattern:
//! ashpd's zbus-5-tokio tasks die with that runtime, closing the
//! connection deterministically).

use std::sync::Arc;
use std::time::Duration;

use notify_rust::{Notification, Urgency};

use crate::error::DaemonError;
use crate::notify::NotificationRecord;
use crate::strings;

use super::Notifier;

/// How long a toast stays up (also bounds the click-wait thread).
const NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens a URI through the user's default handler.
///
/// Synchronous by contract: implementations are called from the
/// notification click thread, never from an async context.
pub trait UriOpener: Send + Sync + std::fmt::Debug {
    /// Opens `uri` (file:// or https://).
    ///
    /// # Errors
    ///
    /// [`DaemonError::Portal`] when the portal call fails,
    /// [`DaemonError::Io`] when the runtime cannot be built.
    fn open_uri(&self, uri: &str) -> Result<(), DaemonError>;
}

/// Production opener: the XDG `OpenURI` portal via `ashpd`, driven on a
/// private current-thread tokio runtime (safe from plain threads; MUST
/// NOT be called from inside an async context - nested `block_on`
/// panics).
#[derive(Debug, Clone, Copy)]
pub struct PortalUriOpener;

impl UriOpener for PortalUriOpener {
    fn open_uri(&self, uri: &str) -> Result<(), DaemonError> {
        let url = url::Url::parse(uri)
            .map_err(|error| DaemonError::Portal(format!("invalid URI {uri}: {error}")))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime
            .block_on(async {
                ashpd::desktop::open_uri::OpenFileRequest::default()
                    .send_uri(&url)
                    .await
            })
            .map(|_request| ())
            .map_err(|error| DaemonError::Portal(error.to_string()))
    }
}

/// The `notify-rust` notifier with portal-backed click actions.
#[derive(Debug)]
pub struct DesktopNotifier {
    opener: Arc<dyn UriOpener>,
}

impl DesktopNotifier {
    /// A notifier opening click URIs through `opener`.
    #[must_use]
    pub fn new(opener: Arc<dyn UriOpener>) -> Self {
        Self { opener }
    }

    /// A notifier wired to the production [`PortalUriOpener`].
    #[must_use]
    pub fn portal() -> Self {
        Self::new(Arc::new(PortalUriOpener))
    }
}

impl Notifier for DesktopNotifier {
    fn notify(&self, record: NotificationRecord) {
        let opener = Arc::clone(&self.opener);
        let spawned = std::thread::Builder::new()
            .name("flowshot-notify".to_owned())
            .spawn(move || {
                if let Err(error) = show_blocking(&record, opener.as_ref()) {
                    tracing::warn!(%error, "desktop notification failed");
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not spawn the notification thread");
        }
    }
}

/// Sends one toast and (when it carries a click URI) blocks until the
/// notification is acted on or closed. Bounded by the notification
/// timeout on spec-compliant daemons. Callable from any plain thread;
/// this is what [`DesktopNotifier`] runs per toast and what the QA
/// example drives directly.
///
/// # Errors
///
/// [`DaemonError::Notify`] when the notification bus call fails.
pub fn show_blocking(
    record: &NotificationRecord,
    opener: &dyn UriOpener,
) -> Result<(), DaemonError> {
    let (notification, click_uri) = notification_for(record);
    let handle = notification
        .show()
        .map_err(|error| DaemonError::Notify(error.to_string()))?;
    if let Some(uri) = click_uri {
        handle.wait_for_action(move |key| click_action(key, &uri, opener));
    }
    Ok(())
}

/// The click dispatch: only the spec's `default` action (body click /
/// default button) opens the URI; `__closed` and any other key are
/// no-ops.
fn click_action(key: &str, uri: &str, opener: &dyn UriOpener) {
    if key != strings::ACTION_KEY_DEFAULT {
        return;
    }
    if let Err(error) = opener.open_uri(uri) {
        tracing::warn!(%error, uri = %uri, "click-action OpenURI failed");
    }
}

/// Pure record -> toast mapping (presentation lives here, semantics in
/// [`NotificationRecord`]). Returns the notification and the click URI
/// (absent when the record carries none or the path is not absolute).
fn notification_for(record: &NotificationRecord) -> (Notification, Option<String>) {
    let (summary, body, click_uri, urgency) = match record {
        NotificationRecord::Saved(path) => (
            strings::SUMMARY_SAVED,
            path.display().to_string(),
            flowshot_actions::clipboard::file_uri(path),
            Urgency::Normal,
        ),
        NotificationRecord::Uploaded(url) => (
            strings::SUMMARY_UPLOADED,
            url.clone(),
            Some(url.clone()),
            Urgency::Normal,
        ),
        NotificationRecord::Captured => (
            strings::SUMMARY_CAPTURED,
            String::new(),
            None,
            Urgency::Normal,
        ),
        NotificationRecord::Aborted => (
            strings::SUMMARY_ABORTED,
            String::new(),
            None,
            Urgency::Normal,
        ),
        NotificationRecord::Error(message) => (
            strings::SUMMARY_ERROR,
            message.clone(),
            None,
            Urgency::Critical,
        ),
        NotificationRecord::ShortcutsRegistered => (
            strings::SUMMARY_SHORTCUTS_REGISTERED,
            strings::BODY_SHORTCUTS_AUTOSTART_NUDGE.to_owned(),
            None,
            Urgency::Normal,
        ),
        NotificationRecord::About(body) => {
            (strings::SUMMARY_ABOUT, body.clone(), None, Urgency::Low)
        }
    };
    let mut notification = Notification::new();
    notification
        .appname(strings::APP_NAME)
        .summary(summary)
        .body(&body)
        .timeout(NOTIFICATION_TIMEOUT)
        .urgency(urgency);
    if click_uri.is_some() {
        notification.action(strings::ACTION_KEY_DEFAULT, strings::ACTION_OPEN_LABEL);
    }
    (notification, click_uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MockUriOpener {
        opened: Arc<Mutex<Vec<String>>>,
    }

    impl MockUriOpener {
        fn opened(&self) -> Vec<String> {
            self.opened
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    impl UriOpener for MockUriOpener {
        fn open_uri(&self, uri: &str) -> Result<(), DaemonError> {
            self.opened
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(uri.to_owned());
            Ok(())
        }
    }

    #[test]
    fn saved_record_maps_to_click_to_open_with_file_uri() {
        let path = Path::new("/tmp/flowshot-saved.png");
        let (notification, click_uri) =
            notification_for(&NotificationRecord::Saved(path.to_path_buf()));
        assert_eq!(notification.appname, strings::APP_NAME);
        assert_eq!(notification.summary, strings::SUMMARY_SAVED);
        assert_eq!(notification.body, "/tmp/flowshot-saved.png");
        assert_eq!(
            click_uri.as_deref(),
            Some("file:///tmp/flowshot-saved.png"),
            "click URI must be the file:// form of the saved path"
        );
        assert_eq!(
            notification.actions,
            vec![
                strings::ACTION_KEY_DEFAULT.to_owned(),
                strings::ACTION_OPEN_LABEL.to_owned()
            ]
        );
    }

    #[test]
    fn relative_saved_path_has_no_click_action() {
        let (notification, click_uri) = notification_for(&NotificationRecord::Saved(
            Path::new("shot.png").to_path_buf(),
        ));
        assert_eq!(click_uri, None);
        assert!(notification.actions.is_empty());
    }

    #[test]
    fn uploaded_record_clicks_through_to_the_url() {
        let (notification, click_uri) = notification_for(&NotificationRecord::Uploaded(
            "https://i.imgur.com/abc.png".to_owned(),
        ));
        assert_eq!(notification.summary, strings::SUMMARY_UPLOADED);
        assert_eq!(click_uri.as_deref(), Some("https://i.imgur.com/abc.png"));
        assert_eq!(notification.actions.len(), 2);
    }

    #[test]
    fn plain_records_carry_no_click_action() {
        for record in [
            NotificationRecord::Captured,
            NotificationRecord::Aborted,
            NotificationRecord::Error("encode failed".to_owned()),
        ] {
            let (notification, click_uri) = notification_for(&record);
            assert_eq!(click_uri, None, "{record:?} must not be clickable");
            assert!(notification.actions.is_empty());
        }
        let (aborted, _) = notification_for(&NotificationRecord::Aborted);
        assert_eq!(aborted.summary, strings::SUMMARY_ABORTED);
        let (error, _) = notification_for(&NotificationRecord::Error("x".to_owned()));
        assert_eq!(error.summary, strings::SUMMARY_ERROR);
        assert_eq!(error.body, "x");
        let (about, _) = notification_for(&NotificationRecord::About(strings::about_body("9.9.9")));
        assert_eq!(about.summary, strings::SUMMARY_ABOUT);
        assert_eq!(about.body, "Version 9.9.9");
    }

    #[test]
    fn click_action_opens_only_on_the_default_key() {
        let opener = MockUriOpener::default();
        click_action("__closed", "file:///tmp/a.png", &opener);
        click_action("some-button", "file:///tmp/a.png", &opener);
        assert!(opener.opened().is_empty());
        click_action(strings::ACTION_KEY_DEFAULT, "file:///tmp/a.png", &opener);
        assert_eq!(opener.opened(), vec!["file:///tmp/a.png".to_owned()]);
    }

    #[test]
    fn click_action_swallows_opener_errors() {
        #[derive(Debug)]
        struct FailingOpener;
        impl UriOpener for FailingOpener {
            fn open_uri(&self, _uri: &str) -> Result<(), DaemonError> {
                Err(DaemonError::Portal("no portal".to_owned()))
            }
        }
        click_action(
            strings::ACTION_KEY_DEFAULT,
            "file:///tmp/a.png",
            &FailingOpener,
        );
    }

    #[test]
    fn portal_opener_rejects_invalid_uris_typed() {
        let error = PortalUriOpener
            .open_uri("not a uri")
            .err()
            .unwrap_or_else(|| panic!("an invalid URI must be rejected"));
        assert!(matches!(error, DaemonError::Portal(_)));
    }
}

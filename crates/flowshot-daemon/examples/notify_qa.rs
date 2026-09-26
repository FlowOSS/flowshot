//! QA harness (todo 32 live acceptance): dispatches the post-save desktop
//! notification through the PRODUCTION notifier path (`show_blocking` =
//! exactly what [`DesktopNotifier`] runs on its notification thread) -
//! the test seam the task brief mandates instead of a real capture.
//! `busctl --user monitor org.freedesktop.Notifications` observes the
//! `Notify` call; the click action registers but is not exercised
//! (unattended QA never clicks).
//!
//! Usage: `notify_qa [saved-path]` - blocks until the notification closes
//! (bounded by its 5 s timeout on spec-compliant daemons).

#![forbid(unsafe_code)]

use std::path::PathBuf;

use flowshot_daemon::notify::{NotificationRecord, PortalUriOpener, show_blocking};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let path = std::env::args().nth(1).map_or_else(
        || PathBuf::from("/tmp/flowshot-notify-qa.png"),
        PathBuf::from,
    );
    let record = NotificationRecord::Saved(path);
    println!("dispatching post-save notification: {record:?}");
    show_blocking(&record, &PortalUriOpener)?;
    println!("notification closed");
    Ok(())
}

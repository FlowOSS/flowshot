//! User-facing message-key constants (Amendment #4 item 8: i18n-ready -
//! no inline literals in logic; English-only v1, catalog-ready).

/// Application name reported to the notification daemon.
pub const APP_NAME: &str = "FlowShot";

/// Summary for the post-save desktop notification (`showDesktopNotification`
/// in Flameshot vocabulary, folded into `[daemon].notifications`).
pub const SUMMARY_SAVED: &str = "Screenshot saved";

/// Summary for the post-upload desktop notification.
pub const SUMMARY_UPLOADED: &str = "Screenshot uploaded";

/// Summary for the plain success toast (the `notify` post-capture action
/// when nothing was saved to disk).
pub const SUMMARY_CAPTURED: &str = "Screenshot captured";

/// Summary for the abort notification (`showAbortNotification` in Flameshot
/// vocabulary).
pub const SUMMARY_ABORTED: &str = "Screenshot aborted";

/// Summary for error notifications.
pub const SUMMARY_ERROR: &str = "FlowShot error";

/// Label of the click action that opens the saved file / uploaded URL
/// through the `OpenURI` portal.
pub const ACTION_OPEN_LABEL: &str = "Open";

/// Identifier of the default (body-click) action per the freedesktop
/// notification spec.
pub const ACTION_KEY_DEFAULT: &str = "default";

/// `Name` field of the autostart `.desktop` entry.
pub const AUTOSTART_NAME: &str = "FlowShot";

/// `GenericName` field of the autostart `.desktop` entry.
pub const AUTOSTART_GENERIC_NAME: &str = "Screenshot Tool";

/// `Comment` field of the autostart `.desktop` entry.
pub const AUTOSTART_COMMENT: &str = "FlowShot screenshot daemon (session autostart)";

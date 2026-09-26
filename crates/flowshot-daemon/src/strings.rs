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

/// Summary for the ONE-TIME toast after the first successful portal
/// shortcut registration (plan todo 34, Oracle r4 F-3: portal hotkeys need
/// daemon residency across logins, so the nudge recommends autostart and
/// points at the settings surface - the clickable settings deep-link lands
/// with the todo-36 settings UI; recorded deviation).
pub const SUMMARY_SHORTCUTS_REGISTERED: &str = "Global shortcuts registered";

/// Body of the one-time shortcut-registration toast.
pub const BODY_SHORTCUTS_AUTOSTART_NUDGE: &str = "FlowShot global shortcuts are now registered. \
Enable \"Launch at startup\" in FlowShot settings (run `flowshot settings`) so they keep \
working across logins.";

/// `Name` field of the autostart `.desktop` entry.
pub const AUTOSTART_NAME: &str = "FlowShot";

/// `GenericName` field of the autostart `.desktop` entry.
pub const AUTOSTART_GENERIC_NAME: &str = "Screenshot Tool";

/// `Comment` field of the autostart `.desktop` entry.
pub const AUTOSTART_COMMENT: &str = "FlowShot screenshot daemon (session autostart)";

//! User-facing message-key constants (i18n-ready -
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
/// shortcut registration (Oracle r4 F-3: portal hotkeys need
/// daemon residency across logins, so the nudge recommends autostart and
/// points at the settings surface - the clickable settings deep-link lands
/// with the settings UI; recorded deviation).
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

/// `Id` property of the tray item (host-side identification token).
pub const TRAY_ID: &str = "flowshot";

/// Tray tooltip and `Title` property (tooltip `FlowShot`).
pub const TRAY_TITLE: &str = "FlowShot";

/// Tray menu: interactive region capture (F12 parity label).
pub const MENU_TAKE_SCREENSHOT: &str = "Take Screenshot";

/// Tray menu: full-desktop capture.
pub const MENU_CAPTURE_FULL: &str = "Capture Full Screen";

/// Tray menu: per-monitor submenu root (children = live-probed outputs).
pub const MENU_CAPTURE_SCREEN: &str = "Capture Screen";

/// Tray menu: per-monitor submenu placeholder while no output is known.
pub const MENU_NO_OUTPUTS: &str = "No outputs detected";

/// Tray menu: manual-coordinate launcher dialog.
pub const MENU_CAPTURE_LAUNCHER: &str = "Capture Launcher";

/// Tray menu: opens the configured `[save].path` (the platform pictures
/// directory when empty) in the user's file manager via the `OpenURI`
/// portal (F12 parity label).
pub const MENU_OPEN_SAVE_PATH: &str = "Open Save Path";

/// Tray menu: settings surface.
pub const MENU_CONFIGURE: &str = "Configure";

/// Tray menu: about entry (v1 = version toast; a full About surface lands
/// with the settings stack - recorded deviation).
pub const MENU_ABOUT: &str = "About";

/// Tray menu: clean daemon shutdown.
pub const MENU_QUIT: &str = "Quit";

/// Summary of the About notification (tray `About` entry).
pub const SUMMARY_ABOUT: &str = "About FlowShot";

/// Body of the About notification for `version`.
#[must_use]
pub fn about_body(version: &str) -> String {
    format!("Version {version}")
}

/// The exclusive stdout modes requested together (the executor rejects).
pub const STDOUT_MODES_CONFLICT: &str = "--raw and --print-geometry both own stdout; pick one";

/// The bus reply when the executor thread died (panic or spawn failure)
/// before reporting an outcome - surfaced instead of swallowed (the
/// silent-failure fix).
pub const EXECUTOR_THREAD_DIED: &str =
    "the FlowShot executor thread died before reporting an outcome";

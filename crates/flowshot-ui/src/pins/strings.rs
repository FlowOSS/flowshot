//! Pin user-facing strings (Amendment #4 item 8: message-key constants, no
//! inline literals in logic). English-only v1; the i18n catalog replaces the
//! values behind these keys on the roadmap. Labels mirror the Flameshot pin
//! context menu (F27 BORROW) so migrating users see familiar wording.

/// Context-menu item: copy the pin to the clipboard.
pub const MENU_COPY: &str = "Copy to clipboard";
/// Context-menu item: save the pin to a file.
pub const MENU_SAVE: &str = "Save to file";
/// Context-menu item: rotate 90 degrees clockwise.
pub const MENU_ROTATE_RIGHT: &str = "Rotate Right";
/// Context-menu item: rotate 90 degrees counter-clockwise.
pub const MENU_ROTATE_LEFT: &str = "Rotate Left";
/// Context-menu item: raise opacity by 0.1.
pub const MENU_INCREASE_OPACITY: &str = "Increase Opacity";
/// Context-menu item: lower opacity by 0.1.
pub const MENU_DECREASE_OPACITY: &str = "Decrease Opacity";
/// Context-menu item: close the pin.
pub const MENU_CLOSE: &str = "Close";

/// The pin window title (portable; the `flowshot-pin` `app_id` is applied by
/// the binary layer through the window customizer seam - purity gate).
pub const WINDOW_TITLE: &str = "FlowShot Pin";

//! The launcher dialog's user-facing strings (English-only v1; the
//! message-catalog-ready convention per Amendment #3 - every literal lives
//! here, never inline in the widget code).

/// Window title (also the tray item label that dispatches it, todo 33).
pub const WINDOW_TITLE: &str = "Capture Launcher";

/// Target dropdown label.
pub const TARGET_LABEL: &str = "Target";
/// Target dropdown entry: manual `WxH+X+Y` geometry.
pub const TARGET_MANUAL: &str = "Manual region";
/// Target dropdown entry prefix for a probed monitor (`Screen {n}: {name}`).
pub const TARGET_SCREEN_PREFIX: &str = "Screen";

/// Geometry field label.
pub const GEOMETRY_LABEL: &str = "Geometry";
/// Inline hint shown while the geometry field is empty.
pub const GEOMETRY_HINT: &str = "WxH+X+Y, e.g. 640x480+0+0";
/// Inline error shown for a malformed geometry entry.
pub const GEOMETRY_INVALID: &str = "Expected WxH+X+Y (e.g. 640x480+0+0)";

/// Delay spinner label.
pub const DELAY_LABEL: &str = "Delay";
/// Delay spinner unit suffix.
pub const DELAY_SUFFIX: &str = " ms";

/// Capture (dispatch) button.
pub const CAPTURE: &str = "Capture";
/// Cancel (close, no dispatch) button.
pub const CANCEL: &str = "Cancel";

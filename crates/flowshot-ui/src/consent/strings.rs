//! The consent dialog's user-facing strings (English-only v1; the
//! message-catalog-ready convention - every literal lives here, never
//! inline in the widget code).

/// Window title (the binary layer reuses it for the Wayland window title
/// through the customizer seam).
pub const WINDOW_TITLE: &str = "Help improve FlowShot?";

/// The honest one-paragraph pitch: what is sent, where, and the opt-in.
pub const BODY: &str = "Help improve FlowShot by sending crash reports and error diagnostics to our self-hosted Sentry. Nothing is sent unless you opt in.";

/// Checkbox 1: the master switch. Recommended ON as TEXT only - the box
/// itself starts unchecked (GDPR-honest opt-in, the module header rule).
pub const OPTION_SEND: &str =
    "Send telemetry (crash reports + basic environment info) — recommended";

/// Checkbox 2: the tier-2 technical payload. Recommended OFF through its
/// own honesty text (the GDPR note).
pub const OPTION_DETAILS: &str = "Include detailed technical information (GPU model, kernel version, monitor layout, install ID) — may be identifying under GDPR";

/// Records the checkbox answers (`asked_on_first_launch` flips true).
pub const SAVE_CHOICE: &str = "Save choice";
/// Defers permanently: both flags false, the question recorded - the
/// dialog never nags again.
pub const NOT_NOW: &str = "Not now";

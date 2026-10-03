//! The consent dialog's user-facing strings (English-only v1; the
//! message-catalog-ready convention - every literal lives here, never
//! inline in the widget code).

/// Window title (the binary layer reuses it for the Wayland window title
/// through the customizer seam) AND the in-dialog semibold headline - one
/// question, one string.
pub const WINDOW_TITLE: &str = "Help improve FlowShot?";

/// The honest pitch under the title: what is sent, where, and the
/// affirmative-save guarantee (nothing leaves the machine before it).
pub const BODY: &str = "Crash reports and error diagnostics go to our self-hosted Sentry and help us fix what actually breaks. Nothing is sent until you save your choice, and you can change it at any time in Settings.";

/// Checkbox 1 label: the master switch. Pre-checked by default (the user
/// directive, module header); the recommendation lives in the hint.
pub const OPTION_SEND_LABEL: &str = "Send telemetry";
/// Checkbox 1 hint: the tier-1 contents + the recommendation.
pub const OPTION_SEND_HINT: &str = "Crash reports and basic environment info — recommended.";

/// Checkbox 2 label: the tier-2 technical payload. Unchecked by default
/// (recommended off).
pub const OPTION_DETAILS_LABEL: &str = "Include detailed technical information";
/// Checkbox 2 hint: the tier-2 contents + the GDPR honesty text.
pub const OPTION_DETAILS_HINT: &str =
    "GPU model, kernel version, monitor layout, install ID — may be identifying under GDPR.";

/// Records the checkbox answers (`asked_on_first_launch` flips true).
pub const SAVE_CHOICE: &str = "Save choice";
/// Defers permanently: both flags false, the question recorded - the
/// dialog never nags again.
pub const NOT_NOW: &str = "Not now";

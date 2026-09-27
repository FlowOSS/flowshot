//! User-facing message-key constants (Amendment #4 item 8: i18n-ready - no
//! inline literals in logic; English-only v1, message-catalog-ready).

/// Rejection line for legacy Flameshot verbs (format arg 0: the verb).
pub const LEGACY_VERB_REJECTED: &str = "flowshot: `{0}` is not a FlowShot command; \
the Flameshot interface is deliberately not replicated (capability parity, not interface parity).";

/// Did-you-mean hint for the legacy `gui` verb.
pub const HINT_LEGACY_GUI: &str =
    "did you mean: flowshot capture   (interactive region capture with the editor)";

/// Did-you-mean hint for the legacy `launcher` verb.
pub const HINT_LEGACY_LAUNCHER: &str =
    "did you mean: flowshot capture --dialog   (manual-coordinate capture launcher)";

/// Did-you-mean hint for the legacy top-level `screen` verb.
pub const HINT_LEGACY_SCREEN: &str = "did you mean: flowshot capture screen [<n|connector>]   \
(no argument = the output under the cursor)";

/// `--upload` while `[upload].client_id` is empty (Amendment #3: the shared
/// default client-id was dropped; upload is disabled until configured).
pub const UPLOAD_UNCONFIGURED: &str =
    "--upload is not configured: set upload.client_id (flowshot settings)";

/// Rejection of a `--region` value outside the Oracle-r4 grammar.
pub const REGION_INVALID: &str = "invalid --region value: expected `WxH`, `WxH+X+Y` \
(signed offsets allowed, global logical pixels), or `at-cursor`";

/// A second positional was given without the `screen` target.
pub const SCREEN_SPEC_WITHOUT_TARGET: &str = "the <N|CONNECTOR> argument is only valid with the \
`screen` target (flowshot capture screen [<n|connector>])";

/// An empty screen selector.
pub const SCREEN_SPEC_EMPTY: &str =
    "the screen selector must be an index (from 0) or a connector name (e.g. DP-1)";

/// A screen index outside `u32`.
pub const SCREEN_SPEC_OUT_OF_RANGE: &str = "the screen index is out of range (expected a u32)";

/// The spawned helper daemon never acquired the bus name.
pub const DAEMON_SPAWN_TIMEOUT: &str = "the spawned FlowShot daemon did not acquire the bus name \
in time; if the session uses systemd, check `systemd-run --user` availability, otherwise run \
`flowshot daemon` manually for the error";

/// The exclusive stdout modes requested together.
pub const STDOUT_MODES_CONFLICT: &str = "--raw and --print-geometry both own stdout; pick one";

/// An argv element that is not valid Unicode (the Invoke wire is `as`).
pub const NON_UNICODE_ARG: &str = "argument is not valid Unicode";

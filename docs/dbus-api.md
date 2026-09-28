# D-Bus API reference

FlowShot's background daemon exposes a session-bus interface for scripting,
second-instance forwarding, and inter-process coordination. The CLI
(`flowshot`) forwards to the daemon over D-Bus when a running instance is
detected; one-shot modes (`--no-daemon`, `--raw`, `--print-geometry`) never
reach the bus.

## Service coordinates

| Constant | Value |
|---|---|
| Well-known bus name | `org.flowoss.FlowShot` |
| Object path | `/org/flowoss/FlowShot` |
| Interface name | `org.flowoss.FlowShot` |
| Bus type | Session bus |

The service name is in the `org.flowoss.*` namespace (User Amendment #2).
FlowShot must NEVER claim `org.flameshot.*` or `org.flowshot.*` — coexistence
with a parallel Flameshot install is a hard requirement.

## Methods

| Method | Signature | Purpose |
|---|---|---|
| `Capture` | `a{sv}` | Region capture with modifier options |
| `CaptureFull` | (none) | Full-desktop capture (all outputs) |
| `CaptureScreen` | `u` | Single output capture by probe index |
| `Launcher` | (none) | Open the manual-coordinate capture dialog |
| `Settings` | (none) | Open the settings surface |
| `Invoke` | `as` | Second-instance argv forwarding |

### `Capture(a{sv} options)`

Region capture with the modifier bag. The vardict keys are the
[`CAPTURE_OPTION_KEYS`](#capture-option-keys) vocabulary. Unknown keys are
ignored with a warning (forward compatibility: a newer CLI may talk to an
older daemon). A known key with the wrong type is a typed `InvalidArgs` error.

### `CaptureFull`

Full-desktop capture with no modifiers. Equivalent to `flowshot capture full`
with no flags.

### `CaptureScreen(u screen)`

Single-output capture by probe index. The index matches the probe order used
by the tray submenu and `flowshot capture screen <n>`.

### `Launcher`

Opens the manual-coordinate capture launcher dialog.

### `Settings`

Opens the settings UI.

### `Invoke(as argv)`

Second-instance argv forwarding. The daemon re-parses the argv with the CLI's
clap surface. The losing process exits 0 after this call succeeds, non-zero
when the daemon's execution failed early (the silent-failure fix).

## Capture option keys

The `Capture` method's vardict accepts these keys (defined in
`flowshot_daemon::request::CAPTURE_OPTION_KEYS`):

| Key | D-Bus type | CLI flag | Purpose |
|---|---|---|---|
| `delay_ms` | `u` (u32) | `-d/--delay <ms>` | Wait before capturing |
| `instant` | `b` (bool) | `--instant` | Accept-on-select, skip edit review |
| `no_edit` | `b` (bool) | `--no-edit` | Skip editor, go to post-capture actions |
| `copy` | `b` (bool) | `-c/--copy` | Add copy action for this invocation |
| `output` | `s` (string) | `-o/--output <file\|dir>` | Explicit destination |
| `pin` | `b` (bool) | `--pin` | Add pin action |
| `upload` | `b` (bool) | `--upload` | Add upload action |
| `raw` | `b` (bool) | `--raw` | PNG bytes to stdout (one-shot only) |
| `print_geometry` | `b` (bool) | `--print-geometry` | Print WxH+X+Y to stdout (one-shot only) |
| `hide_cursor` | `b` (bool) | `--hide-cursor` | Exclude cursor from capture |
| `region` | `s` (string) | `--region <WxH[+X+Y]\|at-cursor>` | Preselected region |
| `last_region` | `b` (bool) | `--last-region` | Repeat persisted last region |

Default-valued keys may be omitted from the vardict; the daemon defaults
missing keys (the round-trip is exact, property-tested in
`crates/flowshot-cli/tests/wire_roundtrip.rs`).

## CLI → D-Bus mapping

The CLI chooses which D-Bus method to call based on the subcommand and
modifiers:

| CLI form | Wire member | Notes |
|---|---|---|
| `capture` (interactive/preselected/last) | `Capture(a{sv})` | Modifier bag via `CAPTURE_OPTION_KEYS` |
| `capture full` (no modifiers) | `CaptureFull` | Typed member, no vardict |
| `capture screen <n>` (no modifiers) | `CaptureScreen(u)` | Typed member with probe index |
| `capture --dialog` | `Launcher` | Opens the manual-coordinate dialog |
| `settings` | `Settings` | Opens the settings UI |
| `capture full` WITH modifiers | `Invoke(as)` | Lossless argv channel |
| `capture screen` at-cursor or by connector | `Invoke(as)` | No typed member for these forms |
| `pin [FILE]` | `Invoke(as)` | Argv forwarding |
| `color` | `Invoke(as)` | Argv forwarding |

One-shot invocations (`--no-daemon`, `--raw`, `--print-geometry`) never reach
the bus: stdout is never routed over D-Bus. The CLI executes them in-process.

## Error replies

| Error | Condition |
|---|---|
| `org.freedesktop.DBus.Error.InvalidArgs` | Known key with wrong D-Bus type |
| `org.freedesktop.DBus.Error.Failed` | Execution failed early (message in detail) |

The `Failed` reply carries the executor's error message when the receipt
resolves to an error, or a fixed "executor thread died" message when the
receipt is dropped (the silent-failure fix).

## Lifecycle

The daemon stays alive only while something needs it (tray, registered
shortcuts, a held clipboard offer, open pins) and exits after an idle grace
(default 60s) otherwise. The `--idle-grace` flag overrides the default.

## Source locations

| Component | Path |
|---|---|
| Bus interface | `crates/flowshot-daemon/src/bus.rs` |
| Capture request parsing | `crates/flowshot-daemon/src/request.rs` |
| CLI → D-Bus mapping | `crates/flowshot-cli/src/wire.rs` |
| Wire round-trip tests | `crates/flowshot-cli/tests/wire_roundtrip.rs` |

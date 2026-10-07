# crates/flowshot-cli

**Generated:** 2026-10-06 | **Commit:** 36714a7 | **Branch:** main
Score 13 (distinct domain) — 24 files / 3.8k LOC: 12 in `src/`, 10 integration
tests, 1 example. The `flowshot` binary and the authoritative command surface.

## OVERVIEW

A lib + thin-binary split: the clap surface, validation, wire mapping, dispatch
and exit-code table live in `flowshot_cli` (so the daemon-side argv re-parse and
the tests reuse the same types); `src/main.rs` only initializes logging, calls
the lib, and maps failures to exit codes with top-level `anyhow`. Flameshot
INTERFACE parity is deliberately dropped — capability parity is kept.

## STRUCTURE

| File | Role |
|------|------|
| `args.rs` | the clap-derive surface — the authoritative interface spec |
| `invocation.rs` | `Cli` → validated `Invocation` exactly once (parse, don't validate) |
| `wire.rs` | the CLI→D-Bus mapping table: typed members when lossless, else `Invoke(argv)` |
| `dispatch.rs` | local surfaces, the in-process one-shot seam, the bus handshake |
| `spawn.rs` | auto-spawn of the helper daemon (the winner branch) |
| `exit.rs` | the exit-code table + `CliError` (the crate's only root re-export) |
| `completions.rs`, `config_man.rs`, `config.rs`, `strings.rs` | shell completions, the hand-authored `flowshot-config.5` roff, resilient config load, message keys |
| `tests/` | `parse_matrix.rs` (421 L), `wire_roundtrip.rs` (proptest), `capability_map.rs` + `cli_capability_map.toml`, `parity_matrix.rs`, `help_surface.rs`, `exit_mapping.rs`, `daemon_failure_surface.rs`, `session_dispatch.rs`, `man_pages.rs` |

## THE SURFACE

`flowshot` ≡ `flowshot capture`; `capture [region|full|screen [<n|connector>]|last]`
with `--region <WxH[+X+Y]|at-cursor>`, `--last-region`, `-d/--delay`, `--instant`,
`--no-edit`, `-c/--copy`, `-o/--output`, `--pin`, `--upload`, `--raw`,
`--print-geometry`, `--hide-cursor`, `--dialog`, `--no-daemon`; plus `pin [FILE]`,
`color`, `settings` (alias `config`), `daemon`, `completions <shell>`, and the
globals `--print-bind-help`, `--bus-address` (test/QA knob), `-V`, `-h`.
The legacy `gui`/`launcher`/`screen` verbs are rejected with did-you-mean hints.

Exit codes: `0` ok · `1` generic infrastructure · `2` usage · `3` user-cancelled ·
`4` capture-backend · `5` permission denied · `6` action/export. Code 5 comes
from downcasting the capture error chain for `IccError::PermissionDenied`, which
is why this crate depends on `flowshot-capture-wayland`.

## DISPATCH (the part that breaks silently)

1. `--raw`, `--print-geometry` and `--no-daemon` FORCE the in-process one-shot
   path — stdout is never routed over D-Bus. That path builds the fully typed
   request and calls `flowshot_daemon::execute` directly (`direct::run`,
   `overlay::run_interactive`, `launcher::run`) inside the CLI's own tokio
   runtime; the blocking overlay leg rides `spawn_blocking`.
2. Otherwise an ATOMIC bus-name probe decides: name held → forward to the
   running daemon; name free → this process won, so it releases the token,
   spawns the auto-spawned helper (wrapped in `systemd-run --user --scope` when
   systemd is present, because the portal `GlobalShortcuts` backend needs an
   `app-*` unit for the app id), waits for it to acquire the name, then
   forwards. Exactly one prober wins; a losing spawn degrades to a helper that
   exits `AlreadyRunning`.
3. Typed wire members carry only what is lossless; everything else goes as
   `Invoke(argv)`.

## ANTI-PATTERNS (THIS CRATE)

- **`src/lib.rs`'s claim that the daemon re-parses forwarded argv "with THIS
  crate's clap surface" is stale.** The reverse edge would be circular, so
  `flowshot-daemon/src/execute/invoke.rs` hand-parses a frozen subset and mirrors
  this grammar 1:1. Change `args.rs` → change `invoke.rs` → expect
  `tests/session_dispatch.rs` and the daemon's mirroring test to arbitrate.
- No config mutation through flags: config belongs to the settings UI + TOML.
- `clap_mangen` is a DEV-dependency — `examples/man_pages.rs` writes `flowshot.1`
  next to the hand-authored `flowshot-config.5`; man pages and completions are
  generated artifacts printed to stdout, never embedded in the binary.
- A missing, unreadable or corrupt config NEVER fails a command (`config.rs`);
  defaults win and the error is logged.

## COMMANDS

```bash
cargo test -p flowshot-cli                    # includes the parity-matrix validator
cargo run -p flowshot-cli -- --print-bind-help
just run capture full --no-daemon             # one-shot path, no daemon
```

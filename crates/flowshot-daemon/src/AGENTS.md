# crates/flowshot-daemon/src

Score 16 — 56 files / ~13k LOC, 9 module dirs. The crate root covers packaging,
features and dependency pins; this covers the module map and the three seams
every change has to respect.

## OVERVIEW

A bus-facing shell (`bus`/`instance`/`request`) that turns calls into a typed
`DaemonCommand`, one executor that runs it (`execute/`), and the residency
machinery that decides whether the process stays alive (`lifecycle` + `state`).

## STRUCTURE

| Group | Modules | Owns |
|-------|---------|------|
| Bus surface | `bus.rs`, `request.rs`, `command.rs`, `instance.rs` | `org.flowoss.FlowShot` methods, `a{sv}` → typed `CaptureRequest` (parse, don't validate), `DaemonCommand` + `CommandSink`, atomic name acquisition |
| Composition | `daemon.rs`, `main.rs`, `error.rs`, `paths.rs`, `systemd.rs` | startup order (object server registered BEFORE the name claim), `anyhow` only in `main.rs`, XDG resolution, optional `sd_notify` |
| Execution | `execute/` (see below) | capture → overlay/editor → post-capture actions |
| Residency | `lifecycle.rs`, `state.rs` | idle-grace verdicts + the four `PersistenceReasons` |
| Shell integration | `tray/`, `shortcut/`, `notify/`, `autostart.rs` | SNI + dbusmenu, GlobalShortcuts portal + bind fallback, toasts, XDG autostart entry |
| Telemetry | `telemetry/` | consent gate, tier-1 environment, tier-2 payload, `sanitize`/`scrub` |
| Shared | `strings.rs`, `logo_raster.rs`, `testsupport.rs` | i18n-ready message keys, build-time-shared rasterizer, p2p bus stubs |

`execute/`: `mod.rs` (the `execute` entry + `ExecCtx`), `backend.rs` (runs the
negotiation ladder), `direct.rs` (non-interactive full/output/delayed/stdout),
`overlay/` (interactive session; `wiring.rs::configure_core` is the SINGLE
production wiring shared with headless), `headless.rs` (`test-drive`),
`launcher/`, `settings.rs`, `pin.rs`, `consent.rs`, `invoke.rs` (argv
re-parse), `post/` (the action pipeline + sinks), `session/` (the
child-process contract + shared exit mapping), `sink.rs` (`ExecutingSink`,
`Heartbeat`).

## THE THREE SEAMS

1. **`CommandSink`** — every trigger (bus method, tray entry, shortcut
   activation) hands a typed `DaemonCommand` to the configured sink. Tests use
   `RecordingSink`; production uses `ExecutingSink`.
2. **`Notifier`** — every toast goes through the trait; `GatedNotifier` applies
   `[daemon].notifications`, `DesktopNotifier` is production (own thread,
   blocking `notify-rust`), `RecordingNotifier` is the test seam.
3. **`DaemonState` / `PersistenceReasons`** — tray enabled, shortcuts
   registered, pins alive, clipboard offer held. An AUTO-SPAWNED helper exits
   after `DEFAULT_IDLE_GRACE` (60 s) iff none holds; a supervised
   `flowshot daemon` always persists. Adding a fifth reason means touching
   `state.rs`, `lifecycle.rs` and both module docs.

## THE CHILD-PROCESS RULE

winit 0.30 permits exactly ONE event loop per process, so overlay, launcher,
settings and pin windows each run in a session **child** (`execute/session/`).
The parent registers state before spawning (a pin is in the registry before its
window exists), the child is never awaited by a command flow, and a `Heartbeat`
keeps an auto-spawned daemon alive while a child is open.

## CONVENTIONS

- Module-root style is mixed and stays mixed: `execute/mod.rs`,
  `telemetry/mod.rs` versus `notify.rs` + `notify/`, `shortcut.rs` +
  `shortcut/`, `tray.rs` + `tray/`. Match the neighbor.
- Lint suppressions are `#[expect(clippy::…, reason = "…")]` — 67 of them, zero
  `#[allow]` in this crate.
- Tests are inline `#[cfg(test)] mod tests` (32 files); sibling `tests.rs` files
  are a `flowshot-ui` convention, not this one.
- `paths.rs` confines environment reading to `xdg_config_home`; every consumer
  takes an injected base so tests never mutate process env (parallel-test race
  avoidance). Do the same for any new env read.
- Perf budget events use `target: "flowshot_perf"` with monotonic `elapsed_us`
  (`perf.command`, `perf.capture_ready`, `perf.frame_ready`, `perf.done`), so
  hotkey→frame budgets are assertable from logs alone.

## ANTI-PATTERNS (THIS DIRECTORY)

- Never run a blocking window loop on a zbus dispatch task: each command gets a
  dedicated thread with its own current-thread runtime (the recorded executor
  decision).
- `execute/invoke.rs` hand-parses a **frozen argv subset** because
  `flowshot-cli` depends on this crate — the reverse edge would be circular.
  Changing the CLI grammar means changing both parsers and their mirroring test.
- No inline user-facing literals: message keys live in `strings.rs`.

# crates/flowshot-daemon

75 files / ~15.2k LOC, 13 dirs: the largest crate and the only one
that may depend on `flowshot-ui`, `capture-wayland` and `actions` at once.

## OVERVIEW

The `org.flowoss.FlowShot` session service: D-Bus interface, single-instance
handshake, the executing pipeline, SNI tray, global shortcuts, notifications,
opt-in telemetry, autostart, the smart idle lifecycle. Ships a lib plus a
**helper binary `flowshot-daemon`** — the `flowshot` name is `flowshot-cli`'s.

## STRUCTURE

```
flowshot-daemon/
├── src/        # 56 files — module map in src/AGENTS.md
├── build.rs    # resvg at BUILD time: assets/logo.svg -> SNI ARGB32 pixmaps
├── examples/   # e2e_headless (needs test-drive), notify_qa, second_instance
└── tests/      # broker, consent_session, portal_shortcuts, telemetry_*, tray_bus,
                # upload_e2e + golden/bind-help-*.txt x5
```

## FEATURES

- `systemd` — the `sd_notify(3)` READY=1 handshake; otherwise INIT-AGNOSTIC,
  systemd is never a hard dependency.
- `test-drive` — forwards to `flowshot-ui/test-drive` so
  `examples/e2e_headless.rs` drives `OverlayCore` through the production funnel
  with synthetic input, offscreen. The default QA path (user-presence policy).

## BUILD SCRIPT

`build.rs` rasterizes `assets/logo.svg` into the SNI `IconPixmap` sizes
16/22/24/32/48 as raw ARGB32 wire bytes — the binary embeds ~18 KB of pixmaps
instead of the 410 KB PNG or a runtime SVG stack, so `resvg` stays build-only.
`src/logo_raster.rs` is compiled twice: by `build.rs` (via `#[path]`) and by the
crate, whose determinism test re-rasterizes and compares the bytes.

## PINNED DEPENDENCY REALITY (each has an audit comment in Cargo.toml)

- **zbus 5 picks its reactor PER CONNECTION at build time**: inside a tokio
  runtime → tokio reactor (the daemon's bus, the CLI handshake, portal probes);
  outside → the zbus-4 model (private async-executor + driver thread), which the
  p2p stubs pin via `Builder::async_io_unix_stream`. async-io has NO drop-time
  close, so `Daemon::run` and `acquire_or_forward` call `close()` explicitly.
- **ashpd 0.13 `GlobalShortcuts` has no `restore_token`/`persist_mode`** and
  exposes neither session nor handle tokens — "restore data" is therefore the
  re-registration set (`src/shortcut/persist.rs`). Its session-bus connection is
  a process-global `OnceLock`, which is why the stub-portal test drives it
  through `DBUS_SESSION_BUS_ADDRESS`.
- `sentry` is constructed ONLY when `[telemetry].enabled`; a disabled config
  builds no client, transport, thread or probe. The DSN is a build-time constant.

## ANTI-PATTERNS (THIS CRATE)

- **NEVER `org.flameshot.*` or `org.flowshot.*`** — the service is
  `org.flowoss.FlowShot` at `/org/flowoss/FlowShot` so Flameshot can coexist.
- Never await the telemetry-consent dialog child from a command flow
  (`src/execute/consent.rs`).
- Telemetry must never crash the daemon: no panic path in `src/telemetry/`, and
  `sanitize.rs` is the single `before_send` funnel.
- `tests/golden/bind-help-*.txt` are the user-facing contract for the
  compositor-bind snippets: regenerating them is a behavior change.

## COMMANDS

```bash
just daemon                                          # foreground daemon via the CLI
cargo test -p flowshot-daemon                        # 32 inline suites + 7 integration tests
cargo run -p flowshot-daemon --example e2e_headless --features test-drive
```

## NOTES

- `tests/portal_shortcuts.rs` (598 L) and the KWin stub class are load-sensitive
  with raised 30 s budgets; raise further rather than weaken. Both p2p bus sides
  must be built CONCURRENTLY (`src/testsupport.rs`, `tests/broker.rs`) or the
  server blocks forever on the client's SASL handshake.

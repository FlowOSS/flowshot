# ADR-004: Daemon lifecycle and persistence reasons

Status: accepted, implemented (flowshot-daemon, flowshot-cli dispatch).

## Context

A screenshot tool has a residency tension. Some features only work with a
resident process: a clipboard offer dies with the process that serves it
(Wayland data-control semantics), portal global shortcuts dispatch over
D-Bus to a running service, pins are windows someone must own, and the tray
is a persistent presence. But a permanent daemon for a tool you might invoke
twice a day is the bloat the project was founded against. Flameshot's answer
was a config flag (`autoCloseIdleDaemon`); FlowShot's answer is a lifecycle
policy that makes the flag unnecessary.

## Decision

A single-instance daemon owns the `org.flowoss.FlowShot` session-bus name
(object `/org/flowoss/FlowShot`, same-named interface) and stays alive only
while at least one of four **persistence reasons** holds:

1. **Tray**: the SNI item is registered (config `[daemon] tray` or a runtime
   registration; released if the SNI watcher vanishes).
2. **Shortcuts**: global shortcuts are registered with the portal (portal
   triggers need a resident daemon, so registration pins the process).
3. **Pins alive**: at least one pin window is open.
4. **Clipboard offer held**: the daemon served a capture copy whose offer
   dies with the process, so holding one pins it.

An **auto-spawned** helper (started on demand by the CLI) exits once it has
been idle for the grace period (default 60 s, `--idle-grace` overrides) with
no reason holding. A **supervised** daemon (`flowshot daemon`, what an init
system runs) persists unconditionally. The policy is a pure function over
(idle time, reasons); a `Notify` wake makes releases prompt and a floored
poll prevents hot loops.

Supporting mechanics:

- **Single instance** via atomic `RequestName(DoNotQueue)`: a second CLI
  invocation forwards its argv to the name owner over `Invoke` and exits.
  Auto-spawn is race-free: the winner probes, releases, spawns the helper,
  polls `NameHasOwner`, then forwards on a fresh connection.
- **Init-agnostic**: the daemon is a plain foreground process. `sd_notify`
  readiness sits behind a non-default `systemd` cargo feature (hand-rolled
  `NOTIFY_SOCKET` datagram); the systemd user unit is one supervised option,
  not an assumption (OpenRC/runit/s6/shepherd templates are a packaging
  phase).
- **Portal app-id**: when systemd is present, the CLI wraps the auto-spawned
  helper in `systemd-run --user --scope --unit app-org.flowoss.FlowShot-<nonce>`,
  which (with the installed desktop file) satisfies xdg-desktop-portal's
  app-id requirement for GlobalShortcuts. The wrap is a pure enhancement;
  its failure falls back to a direct spawn.
- **zbus 4** on its own per-connection driver thread, drivable from the
  daemon's tokio runtime, with explicit `Connection::close()` on every
  teardown path (zbus 4 with the async-io reactor has no drop-time close).

## Consequences

- Clipboard contents survive the capture window closing without paying for a
  permanent process when nothing needs one. This is the core promise and it
  is live-verified (idle exit at grace, persistence while reasons hold,
  clean SIGINT/SIGTERM shutdown with name release).
- A held clipboard offer is a conservative reason: wl-clipboard-rs exposes
  no "selection replaced" callback, so the flag clears only when the pipeline
  observes the release. The failure direction is safe (a daemon that lingers,
  never a dead offer).
- The tray reason is owned by registration, not config: with `tray = true`
  but no SNI host on the bus, an auto-spawned daemon still idle-exits.
- One honest caveat: daemon death while it holds the clipboard offer kills
  the offer. That is Wayland's model, documented rather than engineered
  around.

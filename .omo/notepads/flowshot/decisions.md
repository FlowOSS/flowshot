# Decisions — flowshot

Architectural choices and rationales discovered during work on this plan.

_Auto-scaffolded by /ulw-execute. Append new entries below - never overwrite._

---

## 2026-09-26 (todo 32): daemon executor, naming, and dependency decisions
- ZBUS EXECUTOR (task MUST-DO #1): daemon's own bus = workspace zbus 4.4 (async-io reactor) driven from a multi-thread TOKIO runtime; zbus 5 direct dep REJECTED (ashpd/notify-rust carry it internally; per-connection private executor + dedicated driver thread verified in zbus source - no conflict). Explicit Connection::close() on every teardown path (no drop-time close in zbus-4-async-io).
- BIN NAME: flowshot-daemon crate bin renamed `flowshot` -> `flowshot-daemon` (collision with flowshot-cli's `flowshot`; todo 35's `flowshot daemon` subcommand calls flowshot_daemon::{Daemon, DaemonOptions}).
- AUTOSTART: hand-rolled .desktop writer instead of the plan-cited auto-launch crate (absent from workspace table + lock; root manifests orchestrator-owned). Same file format, injectable dir, zero deps. Orchestrator may swap in auto-launch later if the root table grows it.
- SD_NOTIFY: hand-rolled NOTIFY_SOCKET datagram behind NON-default `systemd` cargo feature (sd-notify crate absent from lock). Init-agnostic per Amendment #3.
- IDLE GRACE: DEFAULT_IDLE_GRACE = 60 s (pub const; plan fixes no constant; --idle-grace flag overrides).
- CAPTURE WIRE FORMAT: Capture(options) = a{sv} vardict (portal/KWin idiom) parsed once into typed CaptureRequest; unknown keys warn+ignore (forward compat), wrong types = InvalidArgs reply. Wire keys = Amendment-#2 modifier names.
- DISPATCH SEAM: bus methods -> DaemonCommand -> CommandSink (LoggingSink default; ChannelSink for todo 35's executor consumer; RecordingSink for tests). Capture execution/overlay launch = todo 35/38 (task brief: "CLI wiring is todo 35").
- FOREIGN-NAME CLASSIFICATION: no-reply-until-timeout folds into ForeignNameHolder (a zbus peer without an object server silently drops calls - source-verified); OwnerVanished (NameHasNoOwner) stays separate (remedy = retry).

## 2026-09-26 (todo 34): global-shortcuts design decisions
- PORTAL CLIENT = ashpd 0.10.3 (the brief's dep) over a zbus-4 reimplementation: ashpd's wire behavior (path conventions, 's'-or-'o' session_handle quirk, Request/Response joining) is battle-tested against real portals, and the stub-portal e2e exercises the REAL production path (env-steered OnceLock) — so "never claim unexercised portal paths" holds without duplicating the protocol. Recorded pins: no restore tokens, pub(crate) session handle, internal assert (spawn-contained), process-global session connection.
- RESTORE DATA = re-registration set + notified-once flag (JSON, config dir): GlobalShortcuts v2 has no restore token, so "reuse across restarts" means identical re-binding without re-nagging — the honest reading of the plan acceptance, pinned in persist.rs docs.
- FALLBACK LADDER = plan-text-authoritative: portal -> paste-ready compositor snippets (hyprland/sway/gsettings-gnome) + documented guidance (KDE KGlobalAccel, generic). NO active KGlobalAccel D-Bus registration (KDE is portal-served per F14; the task brief defers to the plan text for the ladder).
- ACTIVE_SCREEN (u32::MAX) named sentinel for "active monitor" CaptureScreen: the todo-32 vocabulary has no under-cursor screen command; a documented sentinel defers resolution to the todo-12/18/35 executor instead of hardcoding screen 0 or widening the command enum mid-wave.
- ONE-TIME NUDGE: toast body names `flowshot settings` (textual deep-link); a clickable settings deep-link needs the todo-36 settings surface — recorded deviation in strings.rs docs. Gated by [daemon].notifications like every toast.
- DaemonOptions.shortcuts defaults DISABLED; the flowshot-daemon binary opts in via ShortcutOptions::production() — existing daemon tests/callers keep exact todo-32 behavior (broker tests manually set the persistence flag; untouched).
- Registration blocks Daemon::start up to a 30s budget AFTER sd_notify readiness + bus-name acquisition (a GNOME confirmation dialog must not delay service availability); timeout aborts the task and falls back.
- DesktopEnv REUSED from flowshot-capture (new daemon dep edge; zero new external crates) instead of a shortcut-local enum — single desktop vocabulary per F14; detection logic mirrored from capture-wayland (pure fn + env glue, Wayland gate kept: v1 is Wayland-only).

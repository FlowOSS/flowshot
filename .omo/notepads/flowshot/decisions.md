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

## 2026-09-26 (todo 33): SNI tray design decisions
- HAND-ROLLED SNI + com.canonical.dbusmenu on the daemon's existing zbus-4 connection: no SNI crate (ksni/tray-icon) in the workspace table OR the lock, root manifest orchestrator-owned; task brief sanctions hand-rolling, ksni's wire shapes used as the reference (Unlicense). Deviation from the plan's "ksni" wording recorded here + learnings.
- TRAY ICON = procedural token glyph (four corner brackets, the capture-region symbol) rendered straight to ARGB32-BE pixmaps at 5 sizes: todo-19's vendored set is editor-tool SVGs with NO app logo, and its atlas lives in flowshot-ui's OUT_DIR (consuming it = GPU-stack dep + PNG decoder in the daemon). IconName stays EMPTY (no installed FlowShot themed icon until packaging todo 41/42; an unresolvable name renders broken on some hosts) - packaging can flip to the themed name with a one-line property change. Attention color = red-600 constant (core palette has no danger token yet - todo 36 action).
- TRAY PERSISTENCE REASON is owned by REGISTRATION, not the config seed: DaemonState now seeds tray=false; set_tray(true) only after a successful RegisterStatusNotifierItem, released on watcher vanish (NameOwnerChanged loop re-registers on late arrival). An auto-spawned daemon with [daemon].tray=true but no host on the bus still idle-exits (task contract). Todo-32's `set_tray(config.daemon.tray)` seed line in Daemon::start removed.
- QUIT = tokio Notify seam in Daemon::run's select (ShutdownReason::Quit) - the org.flowoss.FlowShot bus vocabulary stays frozen at the todo-32 five methods + Invoke (no bus-level Quit; menu-only).
- ABOUT = version toast (NotificationRecord::About) until the todo-36 settings stack provides a real surface - recorded deviation from the plan's About dialog parity.
- HOST COMPAT: ItemIsMenu=true, Activate -> UnknownMethod (ksni-verified GNOME-appindicator/Plasma<6.4 menu fallback), SecondaryActivate -> quick region Capture, ContextMenu -> no-op (hosts render the D-Bus menu). Status idles at "Active" (Passive items get hidden); set_status seam + NewStatus emission exist but nothing drives them in v1.
- PER-MONITOR SUBMENU: stable ids SCREEN_BASE(100)+probe_index; refresh = AboutToShow(submenu) -> deduped background task (spawn_blocking CaptureThread per refresh, no resident wayland connection) -> revision++ -> LayoutUpdated signal; initial probe kicked at tray start; probe failure = disabled placeholder entry (headless-safe, infallible trait contract).
- DaemonOptions.tray seeds from config ([daemon].tray + [ui].accent_color) in the constructors - unlike todo-34's shortcuts (binary opt-in), the tray gate IS the config key; default config = off = pre-todo-33 behavior byte-identical.

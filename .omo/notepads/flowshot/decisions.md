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

#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! The `FlowShot` session daemon (plan todo 32).
//!
//! Owns the `org.flowoss.FlowShot` session-bus service at
//! `/org/flowoss/FlowShot` (org namespace = `FlowOSS`, product = `FlowShot`,
//! User Amendment #2 + Metis #10 - NEVER `org.flameshot.*` or
//! `org.flowshot.*`, so a parallel `Flameshot` install coexists).
//!
//! # Pieces
//!
//! - [`bus`] - the `D-Bus` interface: `Capture(options)`, `CaptureFull`,
//!   `CaptureScreen(n)`, `Launcher`, `Settings`, plus `Invoke(argv)`;
//! - [`instance`] - single-instance handshake: bus-name acquisition is
//!   ATOMIC on the bus (exactly one winner, race-free by construction);
//!   losers forward their command line through `Invoke` and exit 0;
//! - [`lifecycle`] + [`state`] - the Amendment-#3 smart lifecycle: an
//!   auto-spawned helper exits after the idle grace iff NO persistence
//!   reason holds (tray / shortcuts / pins / clipboard offer);
//!   `flowshot daemon` (supervised) always persists and is init-agnostic
//!   (`sd_notify` lives behind the optional `systemd` cargo feature);
//! - [`notify`] - desktop notifications (`notify-rust`): post-save/upload
//!   toasts with click-action -> `OpenURI` portal, abort + error toasts,
//!   the `[daemon].notifications` gate, and the `NotifySink` bridge the
//!   todo-28/29 actions pipeline toasts through;
//! - [`shortcut`] - global shortcuts (todo 34): the `GlobalShortcuts`
//!   portal primary path (`ashpd`) with the `Activated` -> [`CommandSink`]
//!   dispatch, restore-data persistence, the one-time autostart nudge, the
//!   `shortcuts` persistence reason, and the compositor-bind fallback
//!   ladder (paste-ready Hyprland/Sway/GNOME snippets, `KGlobalAccel`
//!   guidance) behind `flowshot --print-bind-help` (todo 35);
//! - [`autostart`] - `[daemon].startup_launch` -> XDG `.desktop` autostart
//!   entry;
//! - [`daemon`] - the composition root ([`Daemon::start`] /
//!   [`Daemon::run`]).
//!
//! # Executor decision (todo 32, recorded per the task brief)
//!
//! The daemon's OWN bus stays on workspace **zbus 4** (default features =
//! async-io reactor), driven from a **tokio** runtime. Source-verified
//! mechanics (zbus 4.4 `abstractions/executor.rs` + `connection/builder.rs`):
//! each connection owns a PRIVATE `async-executor` instance plus a dedicated
//! driver thread (`zbus::Connection executor`) that ticks until the
//! connection's tasks end - so zbus-4 futures are drivable from ANY
//! executor (the todo-11-proven pattern) with no interference with tokio.
//! Portal-facing async coexists on the same runtime: `ashpd` (zbus 5 +
//! tokio reactor) runs inside private current-thread runtimes at its call
//! sites (the todo-10 worker pattern), and `notify-rust` (zbus 5, blocking
//! `show()`) only ever runs on dedicated notification threads. A direct
//! zbus-5 dependency was REJECTED: it would deepen the existing duplicate
//! zbus family for zero functional gain. Teardown discipline: zbus-4
//! async-io has NO drop-time close, so [`daemon::Daemon::run`] and
//! [`instance::acquire_or_forward`] close connections explicitly (todo-11
//! lesson).

pub mod autostart;
pub mod bus;
pub mod command;
pub mod daemon;
pub mod error;
pub mod instance;
pub mod lifecycle;
pub mod notify;
pub mod paths;
pub mod request;
pub mod shortcut;
pub mod state;
pub mod strings;

#[cfg(feature = "systemd")]
pub mod systemd;

#[cfg(test)]
mod testsupport;

pub use bus::{IFACE, OBJECT_PATH, SERVICE};
pub use command::{ChannelSink, CommandSink, DaemonCommand, LoggingSink, RecordingSink};
pub use daemon::{Daemon, DaemonOptions, ShutdownReason, Startup};
pub use error::DaemonError;
pub use instance::{Acquisition, Ownership, acquire_or_forward};
pub use lifecycle::{Clock, DaemonMode, LifecyclePolicy, TokioClock, Verdict};
pub use notify::{
    ActionNotifyBridge, DesktopNotifier, GatedNotifier, NotificationRecord, Notifier,
    RecordingNotifier,
};
pub use request::CaptureRequest;
pub use shortcut::{
    ACTIVE_SCREEN, CompositorFlavor, Registration, RestoreData, ShortcutOptions, ShortcutSpec,
    ShortcutWiring, bind_help, default_shortcuts,
};
pub use state::{DaemonState, PersistenceReasons};

use std::time::Duration;

/// Default idle grace for the auto-spawned helper mode (the plan fixes no
/// constant; 60 s matches the "lean on-demand daemon" intent - the binary
/// exposes `--idle-grace` and todo 39's packaging may revisit).
pub const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(60);

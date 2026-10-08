#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! The `FlowShot` session daemon.
//!
//! Owns the `org.flowoss.FlowShot` session-bus service at
//! `/org/flowoss/FlowShot` (org namespace = `FlowOSS`, product = `FlowShot`,
//! Metis #10 - NEVER `org.flameshot.*` or
//! `org.flowshot.*`, so a parallel `Flameshot` install coexists).
//!
//! # Pieces
//!
//! - [`bus`] - the `D-Bus` interface: `Capture(options)`, `CaptureFull`,
//!   `CaptureScreen(n)`, `Launcher`, `Settings`, plus `Invoke(argv)`;
//! - [`instance`] - single-instance handshake: bus-name acquisition is
//!   ATOMIC on the bus (exactly one winner, race-free by construction);
//!   losers forward their command line through `Invoke` and exit 0;
//! - [`lifecycle`] + [`state`] - the smart lifecycle: an
//!   auto-spawned helper exits after the idle grace iff NO persistence
//!   reason holds (tray / shortcuts / pins / clipboard offer);
//!   `flowshot daemon` (supervised) always persists and is init-agnostic
//!   (`sd_notify` lives behind the optional `systemd` cargo feature);
//! - [`notify`] - desktop notifications (`notify-rust`): post-save/upload
//!   toasts with click-action -> `OpenURI` portal, abort + error toasts,
//!   the `[daemon].notifications` gate, and the `NotifySink` bridge the
//!   actions pipeline toasts through;
//! - [`shortcut`] - global shortcuts: the `GlobalShortcuts`
//!   portal primary path (`ashpd`) with the `Activated` -> [`CommandSink`]
//!   dispatch, restore-data persistence, the one-time autostart nudge, the
//!   `shortcuts` persistence reason, and the compositor-bind fallback
//!   ladder (paste-ready Hyprland/Sway/GNOME snippets, `KGlobalAccel`
//!   guidance) behind `flowshot --print-bind-help`;
//! - [`stale`] - the stale-binary self-check: a dispatch-time
//!   `/proc/self/exe` probe; a superseded image exits cleanly UNSERVED
//!   (`ShutdownReason::Superseded`), and the broker's `NoReply` answer
//!   to the pending call rides the CLI's single dispatch retry into a
//!   fresh-daemon respawn;
//! - [`tray`] - the SNI tray: `org.kde.StatusNotifierItem` +
//!   `com.canonical.dbusmenu` hand-rolled on the same zbus connection,
//!   the F12-parity menu dispatching into [`CommandSink`], the
//!   `[daemon].tray` gate, the absent-watcher degrade (warn, never
//!   crash), and the `tray` lifecycle persistence reason;
//! - [`autostart`] - `[daemon].startup_launch` -> XDG `.desktop` autostart
//!   entry;
//! - [`telemetry`] - the opt-in error-telemetry engine (`[telemetry]`
//!   consent gates EVERYTHING: disabled = no client, no transport, no
//!   thread, no probes; enabled = the sanitized two-tier payload to the
//!   self-hosted Sentry, DSN a build-time constant);
//! - [`daemon`] - the composition root ([`Daemon::start`] /
//!   [`Daemon::run`]).
//!
//! # Executor decision (recorded)
//!
//! The daemon's OWN bus rides workspace **zbus 5** from a **tokio**
//! runtime. Source-verified mechanics (zbus 5.19 `abstractions/mod.rs`
//! `use_tokio` + `connection/builder.rs`): BOTH reactors compile in
//! (default async-io + tokio unified via ashpd) and the reactor is chosen
//! PER CONNECTION at build time - built inside a tokio runtime, the
//! connection's internal tasks ride that runtime and die with it (the
//! daemon's bus, the CLI handshake, the portal-worker probes); built
//! outside, the connection gets the old zbus-4 model (PRIVATE
//! `async-executor` + dedicated driver thread, futures drivable from ANY
//! executor). The p2p test stubs pin that async-io path via
//! `Builder::async_io_unix_stream`. Portal-facing async coexists on the
//! same runtime: `ashpd` (zbus 5 + tokio reactor) runs inside private
//! current-thread runtimes at its call sites (the worker pattern), and
//! `notify-rust` (zbus 5, blocking `show()`) only ever runs on dedicated
//! notification threads. The upgrade also UNIFIES the family: the direct
//! edge and ashpd/notify-rust now share one zbus 5. Teardown discipline
//! (todo 11, unchanged): the async-io reactor has NO drop-time close, so
//! [`daemon::Daemon::run`] and [`instance::acquire_or_forward`] close
//! connections explicitly; on the tokio reactor `close()` remains the
//! deterministic release and runtime teardown is the backstop.

pub mod autostart;
pub mod bus;
pub mod command;
pub mod daemon;
pub mod error;
pub mod execute;
pub mod instance;
pub mod lifecycle;
pub mod notify;
pub mod paths;
pub mod request;
pub mod shortcut;
pub mod stale;
pub mod state;
pub mod strings;
pub mod telemetry;
pub mod tray;

#[cfg(feature = "systemd")]
pub mod systemd;

#[cfg(test)]
mod logo_raster;
#[cfg(test)]
mod testsupport;

pub use bus::{BusWiring, IFACE, OBJECT_PATH, SERVICE};
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
pub use stale::{ExeIdentity, SupersessionGate};
pub use state::{DaemonState, PersistenceReasons};
pub use tray::{TrayHandle, TrayOptions, TrayWiring};

use std::time::Duration;

/// Default idle grace for the auto-spawned helper mode (no spec constant
/// fixes this; 60 s matches the "lean on-demand daemon" intent - the binary
/// exposes `--idle-grace` and packaging may revisit).
pub const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(60);

#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! The `FlowShot` command-line interface.
//!
//! The Flameshot INTERFACE is deliberately not replicated (capability parity
//! only): bare `flowshot` is `flowshot capture`, the `gui`/`launcher`/
//! `screen` verbs are rejected with did-you-mean hints, and config mutation
//! belongs to the settings UI + TOML, never to CLI flags.
//!
//! # Surface
//!
//! - `flowshot` / `flowshot capture [region|full|screen [<n|connector>]|last]
//!   [flags]` - capture with the modifier set (`--region
//!   <WxH[+X+Y]|at-cursor>`, `--last-region`, `-d/--delay`, `--instant`,
//!   `--no-edit`, `-c/--copy`, `-o/--output`, `--pin`, `--upload`, `--raw`,
//!   `--print-geometry`, `--hide-cursor`, `--dialog`, `--no-daemon`);
//! - `flowshot pin [FILE]`, `flowshot color`, `flowshot settings` (alias
//!   `config`), `flowshot daemon`, `flowshot completions <shell>`;
//! - global: `--print-bind-help` (compositor-bind snippets),
//!   `--bus-address` (test/QA knob, mirrors `flowshot-daemon`), `-V`, `-h`.
//!
//! # Dispatch (Oracle r4)
//!
//! 1. STDOUT-producing flags (`--raw`, `--print-geometry`) and `--no-daemon`
//!    FORCE the in-process one-shot path - stdout is never routed over
//!    `D-Bus` (Oracle r4 F-5.iii). EXECUTION SEAM: the one-shot path
//!    produces the fully typed request; the capture pipeline (backend ->
//!    overlay/editor -> export actions) plugs in at this seam.
//! 2. Otherwise the daemon path: an ATOMIC bus-name probe decides - name
//!    held -> forward to the running daemon; name free -> this process won,
//!    so it releases the token, spawns the auto-spawned helper daemon
//!    (wrapped in a `systemd-run --user --scope` unit when systemd is
//!    present; finding: the portal `GlobalShortcuts` backend needs
//!    an `app-*` unit for the app id; init-agnostic everywhere else), waits
//!    for the helper to acquire the name, then forwards. Race-free: exactly
//!    one prober wins, and a losing spawn degrades to a helper that exits
//!    `AlreadyRunning`.
//! 3. Forwarding uses the typed wire members when they are LOSSLESS
//!    ([`wire`]: `Capture(a{sv})` over the [`CAPTURE_OPTION_KEYS`]
//!    vocabulary, `CaptureFull`, `CaptureScreen(u)`, `Launcher`, `Settings`)
//!    and `Invoke(argv)` otherwise (targets/modifiers the typed members
//!    cannot carry: `screen` at-cursor/connector, modifiers on
//!    full/screen, `pin`, `color`). The daemon re-parses forwarded argv
//!    with THIS crate's clap surface.
//!
//! # Exit codes ([`exit`])
//!
//! 0 ok / 1 generic-infrastructure / 2 usage / 3 user-cancelled /
//! 4 capture-backend / 5 permission denied / 6 action-export.
//!
//! [`CAPTURE_OPTION_KEYS`]: flowshot_daemon::request::CAPTURE_OPTION_KEYS

pub mod args;
pub mod completions;
pub mod config;
pub mod config_man;
pub mod dispatch;
pub mod exit;
pub mod invocation;
pub mod spawn;
pub mod strings;
pub mod wire;

pub use exit::CliError;

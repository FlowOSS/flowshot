//! Command dispatch: the local surfaces (completions, bind help), the
//! in-process one-shot seam, and the `D-Bus` handshake with the daemon
//! (Oracle r4).
//!
//! # Handshake
//!
//! The well-known name is the single-instance token. This process probes it
//! with an ATOMIC `RequestName(DoNotQueue)`:
//!
//! - held by another process -> forward the call to the running daemon;
//! - acquired here -> this process won the race, so it RELEASES the token,
//!   spawns the auto-spawned helper daemon ([`crate::spawn`], wrapped in a
//!   `systemd-run --user --scope` unit when systemd is present per the
//!   portal app-id finding), waits for the helper to acquire the
//!   name, then forwards. A concurrent second winner degrades gracefully:
//!   its helper exits `AlreadyRunning` and both CLIs forward to whoever
//!   holds the name.
//!
//! # Execution
//!
//! The one-shot path runs the executor IN-PROCESS
//! (`flowshot_daemon::execute`); daemon-forwarded invocations execute
//! inside the daemon through the [`ExecutingSink`] this module installs
//! into `DaemonOptions` (the same executor library, daemon-resident).

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use flowshot_daemon::autostart::exec_value;
use flowshot_daemon::daemon::{Daemon, DaemonOptions, Startup};
use flowshot_daemon::execute::direct::{ScreenTarget, Target};
use flowshot_daemon::execute::{self, ExecCtx, ExecutingSink};
use flowshot_daemon::instance::{self, Ownership};
use flowshot_daemon::shortcut::{CompositorFlavor, bind_help, default_shortcuts, detect_desktop};
use flowshot_daemon::{SERVICE, ShortcutOptions};
use zbus::Connection;

use crate::completions;
use crate::exit::{self, CliError};
use crate::invocation::{CaptureInvocation, CaptureSelection, DaemonRun, Invocation, ScreenSpec};
use crate::spawn;
use crate::wire::{self, WireCall};

/// Budget for the spawned helper to acquire the bus name.
const SPAWN_WAIT: Duration = Duration::from_secs(5);
/// Poll interval while waiting for the helper.
const SPAWN_POLL: Duration = Duration::from_millis(50);
/// The `D-Bus` error name meaning "the owner vanished mid-handshake".
const NAME_HAS_NO_OWNER: &str = "org.freedesktop.DBus.Error.NameHasNoOwner";

/// Runs one resolved invocation to completion.
///
/// # Errors
///
/// [`CliError`] per the [`crate::exit`] table: usage-class rejections,
/// bus/transport failures, helper-spawn failures.
pub async fn dispatch(
    invocation: &Invocation,
    bus_address: Option<&str>,
    argv_tail: &[String],
) -> Result<ExitCode, CliError> {
    match invocation {
        Invocation::Completions(shell) => {
            completions::generate_to_stdout(*shell)?;
            Ok(ExitCode::SUCCESS)
        }
        Invocation::PrintBindHelp => {
            print_bind_help();
            Ok(ExitCode::SUCCESS)
        }
        Invocation::Daemon(run) => run_daemon(run, bus_address).await,
        Invocation::Capture(capture) => {
            if capture.one_shot {
                one_shot_capture(capture).await
            } else {
                let call = wire::capture_call(capture, argv_tail);
                dispatch_bus(&call, bus_address).await
            }
        }
        Invocation::Launcher { one_shot } => {
            if *one_shot {
                one_shot_launcher().await
            } else {
                dispatch_bus(&WireCall::Launcher, bus_address).await
            }
        }
        Invocation::Pin(_) | Invocation::Color => {
            dispatch_bus(&WireCall::Invoke(argv_tail.to_vec()), bus_address).await
        }
        Invocation::Settings => dispatch_bus(&WireCall::Settings, bus_address).await,
    }
}

/// The stdout-producing local surface: `--print-bind-help` (the
/// compositor-bind fallback snippets; the same text feeds the settings tab
/// and the docs).
#[expect(
    clippy::print_stdout,
    reason = "the bind-help surface writes its artifact to stdout by contract"
)]
fn print_bind_help() {
    let flavor = CompositorFlavor::for_desktop(detect_desktop());
    print!("{}", bind_help(flavor, &default_shortcuts()));
}

/// The in-process one-shot capture: the same executor library
/// the daemon runs, on this process (documented trade-off: a one-shot
/// clipboard offer dies with this process - the `--no-daemon` help text).
async fn one_shot_capture(capture: &CaptureInvocation) -> Result<ExitCode, CliError> {
    tracing::info!(
        selection = ?capture.selection,
        request = ?capture.request,
        "one-shot capture executing in-process"
    );
    let ctx = ExecCtx::one_shot(None);
    let started = Instant::now();
    let request = capture.request.clone();
    let result = match &capture.selection {
        CaptureSelection::Interactive => {
            execute::overlay::run_interactive(request, &ctx, started, false).await
        }
        CaptureSelection::Full => execute::direct::run(Target::Full, request, &ctx, started).await,
        CaptureSelection::Screen(spec) => {
            let target = Target::Screen(match spec {
                ScreenSpec::Cursor => ScreenTarget::Cursor,
                ScreenSpec::Index(index) => ScreenTarget::Index(*index),
                ScreenSpec::Connector(name) => ScreenTarget::Connector(name.clone()),
            });
            execute::direct::run(target, request, &ctx, started).await
        }
    };
    Ok(exit::exec_exit_code(&result))
}

/// The one-shot launcher dialog (`capture --dialog --no-daemon`); Cancel
/// maps onto the exit-3 user-cancelled class.
async fn one_shot_launcher() -> Result<ExitCode, CliError> {
    let ctx = ExecCtx::one_shot(None);
    let result = execute::launcher::run(&ctx, Instant::now()).await;
    Ok(exit::exec_exit_code(&result))
}

/// Forwards one wire call to the daemon, with a single full retry when the
/// owner vanished between the probe and the call (the remedy the daemon's
/// `OwnerVanished` classification names).
async fn dispatch_bus(call: &WireCall, bus_address: Option<&str>) -> Result<ExitCode, CliError> {
    match dispatch_bus_once(call, bus_address).await {
        Err(CliError::Dbus(zbus::Error::MethodError(name, _, _))) if name == NAME_HAS_NO_OWNER => {
            dispatch_bus_once(call, bus_address).await
        }
        other => other,
    }
}

async fn dispatch_bus_once(
    call: &WireCall,
    bus_address: Option<&str>,
) -> Result<ExitCode, CliError> {
    let connection = instance::connect(bus_address).await?;
    let outcome = handshake(&connection, call, bus_address).await;
    close(connection).await;
    outcome
}

async fn handshake(
    connection: &Connection,
    call: &WireCall,
    bus_address: Option<&str>,
) -> Result<ExitCode, CliError> {
    match instance::request_ownership(connection, SERVICE).await? {
        Ownership::HeldByOther => {
            tracing::debug!(member = call.member(), "forwarding to the running daemon");
            call.send(connection).await?;
            Ok(ExitCode::SUCCESS)
        }
        Ownership::Acquired => {
            // Winner: release the token so the spawned helper can acquire
            // it - the helper serves the command and outlives this process
            // (clipboard offers/pins persist in the daemon).
            connection.release_name(SERVICE).await?;
            spawn::helper(bus_address)?;
            let daemon = wait_for_owner(bus_address).await?;
            let forwarded = call.send(&daemon).await;
            close(daemon).await;
            forwarded?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Polls the bus until the spawned helper owns the name (or the budget
/// expires).
async fn wait_for_owner(bus_address: Option<&str>) -> Result<Connection, CliError> {
    let connection = instance::connect(bus_address).await?;
    let deadline = tokio::time::Instant::now() + SPAWN_WAIT;
    loop {
        if name_has_owner(&connection).await {
            return Ok(connection);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(CliError::DaemonUnavailable {
                detail: format!("waited {}s", SPAWN_WAIT.as_secs()),
            });
        }
        tokio::time::sleep(SPAWN_POLL).await;
    }
}

/// `org.freedesktop.DBus.NameHasOwner` for the service name; transport
/// errors count as "not owned yet" (the poll budget bounds the wait).
async fn name_has_owner(connection: &Connection) -> bool {
    const DBUS: &str = "org.freedesktop.DBus";
    const DBUS_PATH: &str = "/org/freedesktop/DBus";
    connection
        .call_method(Some(DBUS), DBUS_PATH, Some(DBUS), "NameHasOwner", &SERVICE)
        .await
        .and_then(|reply| reply.body().deserialize::<bool>())
        .unwrap_or(false)
}

/// `flowshot daemon`: the same library the `flowshot-daemon` helper binary
/// runs (foreground; the systemd user unit and other init supervisors
/// start it - init-agnostic).
async fn run_daemon(run: &DaemonRun, bus_address: Option<&str>) -> Result<ExitCode, CliError> {
    let config = crate::config::load(run.config.as_deref());
    let mut options = if run.auto_spawned {
        DaemonOptions::auto_spawned(config)
    } else {
        DaemonOptions::supervised(config)
    };
    options.idle_grace = Duration::from_secs(run.idle_grace_secs);
    options.bus_address = bus_address.map(ToOwned::to_owned);
    options.autostart_exec = std::env::current_exe().ok().map(|exe| exec_value(&exe));
    options.shortcuts = ShortcutOptions::production();
    // The executing sink replaces the LoggingSink default
    // (bus/tray/shortcut commands run the real capture pipeline).
    options.command_sink = Some(Arc::new(ExecutingSink::new(ExecCtx {
        config_path: run.config.clone(),
        state: Some(Arc::clone(&options.state)),
        notifier: None,
        upload_base_url: None,
    })));
    match Daemon::start(options).await? {
        Startup::Running(daemon) => {
            let reason = daemon.run().await?;
            tracing::info!(reason = ?reason, "daemon exited");
        }
        Startup::AlreadyRunning => {
            tracing::info!("another FlowShot daemon is already running; exiting");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Explicit connection close (zbus's async-io reactor has NO drop-time
/// close; `close()` is the deterministic release on both reactors);
/// close errors are teardown noise.
async fn close(connection: Connection) {
    if let Err(error) = connection.close().await {
        tracing::debug!(%error, "the bus connection close reported an error");
    }
}

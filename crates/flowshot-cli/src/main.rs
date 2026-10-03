//! `flowshot` - the `FlowShot` CLI binary (the authoritative command
//! surface). Top-level `anyhow`; all logic lives in the
//! [`flowshot_cli`] library, and every failure exits through the
//! exit-code table.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use flowshot_cli::args::Cli;
use flowshot_cli::exit::{self, CliError};
use flowshot_cli::invocation::{self, Invocation, Resolved};
use flowshot_daemon::telemetry::Surface;

fn main() -> ExitCode {
    init_tracing();
    let argv: Vec<OsString> = std::env::args_os().collect();
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(error) => return exit::clap_exit(&error),
    };
    let bus_address = cli.bus_address.clone();
    let resolved = match invocation::resolve(cli) {
        Ok(resolved) => resolved,
        Err(error) => return report(&error.into()),
    };
    // Telemetry: the absolute first thing after the config load, BEFORE
    // any runtime build or session dispatch (the session child inits its
    // own inside run_child from the spec). The guard must live until the
    // process exits - dropping it flushes the transport, so report()'s
    // capture still reaches a live client.
    let _telemetry = match &resolved {
        Resolved::Session(_) => None,
        Resolved::Command(invocation) => {
            let config = flowshot_cli::config::load(daemon_config_path(invocation));
            flowshot_daemon::telemetry::init(&config.telemetry, surface_of(invocation))
        }
    };
    match execute(resolved, bus_address.as_deref(), &argv) {
        Ok(code) => code,
        Err(error) => report(&error),
    }
}

/// The telemetry surface of one invocation (the daemon subcommand runs
/// the resident daemon in-process; everything else is the CLI surface -
/// the window sessions are child processes with their own surfaces).
fn surface_of(invocation: &Invocation) -> Surface {
    match invocation {
        Invocation::Daemon(_) => Surface::Daemon,
        Invocation::Capture(_)
        | Invocation::Launcher { .. }
        | Invocation::Pin(_)
        | Invocation::Color
        | Invocation::Settings
        | Invocation::Completions(_)
        | Invocation::PrintBindHelp => Surface::Cli,
    }
}

/// The explicit config path of the invocations that carry one (only the
/// daemon subcommand has `--config`; the rest read the default location).
fn daemon_config_path(invocation: &Invocation) -> Option<&std::path::Path> {
    match invocation {
        Invocation::Daemon(run) => run.config.as_deref(),
        Invocation::Capture(_)
        | Invocation::Launcher { .. }
        | Invocation::Pin(_)
        | Invocation::Color
        | Invocation::Settings
        | Invocation::Completions(_)
        | Invocation::PrintBindHelp => None,
    }
}

fn execute(
    resolved: Resolved,
    bus_address: Option<&str>,
    argv: &[OsString],
) -> anyhow::Result<ExitCode> {
    match resolved {
        // The session child runs on the MAIN thread OUTSIDE any tokio
        // runtime (the winit one-event-loop contract; its child legs build
        // their own runtimes - a nested block_on panics). Mirrors the
        // flowshot-daemon binary's early dispatch.
        Resolved::Session(spec) => Ok(ExitCode::from(
            flowshot_daemon::execute::session::run_child(&spec),
        )),
        Resolved::Command(invocation) => run_command(&invocation, bus_address, argv),
    }
}

fn run_command(
    invocation: &Invocation,
    bus_address: Option<&str>,
    argv: &[OsString],
) -> anyhow::Result<ExitCode> {
    let argv_tail = invocation::argv_tail(argv)?;
    if let Invocation::Capture(capture) = invocation
        && capture.request.upload
    {
        // The only CLI-side config gate: unconfigured
        // upload is a usage-class rejection with a settings hint.
        invocation::validate_upload(&capture.request, &flowshot_cli::config::load(None))?;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not build the tokio runtime")?;
    Ok(runtime.block_on(flowshot_cli::dispatch::dispatch(
        invocation,
        bus_address,
        &argv_tail,
    ))?)
}

/// The single error boundary: typed errors map onto the exit-code table,
/// anything else is the generic infrastructure code. Every non-zero exit
/// reports the typed chain to telemetry first (no-op while disabled).
#[expect(
    clippy::print_stderr,
    reason = "the binary's error boundary reports to stderr by contract"
)]
fn report(error: &anyhow::Error) -> ExitCode {
    flowshot_daemon::telemetry::capture_error(&**error);
    let code = error
        .downcast_ref::<CliError>()
        .map_or(exit::GENERIC, exit::exit_code);
    eprintln!("flowshot: {error:#}");
    ExitCode::from(code)
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"))
        .add_directive("zbus::proxy=error".parse().unwrap())
        .add_directive("wgpu_hal::vulkan::conv=error".parse().unwrap())
        .add_directive("wgpu_hal::vulkan::instance=error".parse().unwrap());

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

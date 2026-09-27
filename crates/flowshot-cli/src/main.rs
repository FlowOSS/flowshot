//! `flowshot` - the `FlowShot` CLI binary (plan todo 35, User Amendment #2
//! surface). Top-level `anyhow` per Amendment #4; all logic lives in the
//! [`flowshot_cli`] library, and every failure exits through the todo-35
//! exit-code table.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use flowshot_cli::args::Cli;
use flowshot_cli::exit::{self, CliError};
use flowshot_cli::invocation::{self, Invocation, Resolved};

fn main() -> ExitCode {
    init_tracing();
    let argv: Vec<OsString> = std::env::args_os().collect();
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(error) => return exit::clap_exit(&error),
    };
    match execute(cli, &argv) {
        Ok(code) => code,
        Err(error) => report(&error),
    }
}

fn execute(cli: Cli, argv: &[OsString]) -> anyhow::Result<ExitCode> {
    let bus_address = cli.bus_address.clone();
    match invocation::resolve(cli)? {
        // The session child runs on the MAIN thread OUTSIDE any tokio
        // runtime (the winit one-event-loop contract; its child legs build
        // their own runtimes - a nested block_on panics). Mirrors the
        // flowshot-daemon binary's early dispatch.
        Resolved::Session(spec) => Ok(ExitCode::from(
            flowshot_daemon::execute::session::run_child(&spec),
        )),
        Resolved::Command(invocation) => run_command(&invocation, bus_address.as_deref(), argv),
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
        // The only CLI-side config gate (Amendment #3): unconfigured
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
/// anything else is the generic infrastructure code.
#[expect(
    clippy::print_stderr,
    reason = "the binary's error boundary reports to stderr by contract"
)]
fn report(error: &anyhow::Error) -> ExitCode {
    let code = error
        .downcast_ref::<CliError>()
        .map_or(exit::GENERIC, exit::exit_code);
    eprintln!("flowshot: {error:#}");
    ExitCode::from(code)
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
}

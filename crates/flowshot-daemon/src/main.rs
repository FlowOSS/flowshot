//! `flowshot-daemon` - supervised foreground daemon binary (plan todo 32;
//! the `flowshot daemon` CLI subcommand of todo 35 calls the same
//! library). Top-level `anyhow` per Amendment #4; all logic lives in the
//! library.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use flowshot_core::Config;
use flowshot_daemon::autostart::exec_value;
use flowshot_daemon::daemon::{Daemon, DaemonOptions, Startup};
use flowshot_daemon::paths;

/// `FlowShot` session daemon (`org.flowoss.FlowShot`).
#[derive(Debug, Parser)]
#[command(name = "flowshot-daemon", version, about)]
struct Args {
    /// Run as the auto-spawned helper: exit after the idle grace when no
    /// persistence reason holds (default: supervised, always persists).
    #[arg(long)]
    auto_spawned: bool,

    /// Idle grace in seconds for --auto-spawned mode.
    #[arg(long, value_name = "SECONDS", default_value_t = 60)]
    idle_grace: u64,

    /// Config file to load (default: <xdg-config-home>/flowshot/flowshot.toml).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Bus address to serve on (default: the session bus). Test/QA knob:
    /// a private dbus-daemon address isolates the service completely.
    #[arg(long, value_name = "ADDRESS")]
    bus_address: Option<String>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let config = load_config(args.config.as_deref())?;

    let mut options = if args.auto_spawned {
        DaemonOptions::auto_spawned(config)
    } else {
        DaemonOptions::supervised(config)
    };
    options.idle_grace = Duration::from_secs(args.idle_grace);
    options.bus_address = args.bus_address;
    options.autostart_exec = std::env::current_exe().ok().map(|exe| exec_value(&exe));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not build the tokio runtime")?;
    runtime.block_on(async {
        match Daemon::start(options)
            .await
            .context("daemon startup failed")?
        {
            Startup::Running(daemon) => {
                let reason = daemon.run().await.context("daemon run failed")?;
                tracing::info!(reason = ?reason, "daemon exited");
            }
            Startup::AlreadyRunning => {
                tracing::info!("another FlowShot daemon is already running; exiting");
            }
        }
        Ok(())
    })
}

/// Loads the config, falling back to defaults when the file is absent
/// (first run) or unreadable/corrupt (todo-2 resilience rule: never fail
/// the daemon over config).
fn load_config(explicit: Option<&std::path::Path>) -> anyhow::Result<Config> {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => paths::default_config_path()?,
    };
    match Config::load(&path) {
        Ok(config) => Ok(config),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "config load failed; using defaults");
            Ok(Config::default())
        }
    }
}

//! `flowshot-daemon` - supervised foreground daemon binary (the
//! `flowshot daemon` CLI subcommand calls the same library). Top-level
//! `anyhow`; all logic lives in the library.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use flowshot_core::Config;
use flowshot_daemon::autostart::{exec_value, launch_target};
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

    #[command(subcommand)]
    command: Option<Sub>,
}

/// Internal verbs (hidden from the user surface).
#[derive(Debug, clap::Subcommand)]
enum Sub {
    /// Run one window session from a spec file (the daemon's
    /// child-process contract - winit allows one event loop per
    /// process, so every window session is a dedicated child).
    #[command(hide = true)]
    Session {
        /// The session spec JSON path.
        #[arg(long, value_name = "PATH")]
        spec: PathBuf,
    },
}

/// Default tracing filter when `RUST_LOG` is unset (an explicit `RUST_LOG`
/// overrides it completely via `try_from_default_env`): an `info` base plus
/// ERROR-only for three known-benign upstream spam targets - the ashpd
/// portal session/request teardown race (`zbus::proxy` `GetAll` warnings) and
/// wgpu's NVIDIA present-mode/layer warnings (`wgpu_hal::vulkan`).
const DEFAULT_LOG_FILTER: &str =
    "info,zbus::proxy=error,wgpu_hal::vulkan::conv=error,wgpu_hal::vulkan::instance=error";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER)),
        )
        .init();

    let args = Args::parse();
    if let Some(Sub::Session { spec }) = &args.command {
        let code = flowshot_daemon::execute::session::run_child(spec);
        std::process::exit(i32::from(code));
    }
    let config = load_config(args.config.as_deref())?;
    // Telemetry: the absolute first thing after the config load, before
    // the runtime build (the session child above inits its own inside
    // run_child). The guard must live until the process exits - dropping
    // it flushes the transport; disabled config = None = zero activity.
    let _telemetry = flowshot_daemon::telemetry::init(
        &config.telemetry,
        flowshot_daemon::telemetry::Surface::Daemon,
    );

    let mut options = if args.auto_spawned {
        DaemonOptions::auto_spawned(config)
    } else {
        DaemonOptions::supervised(config)
    };
    options.idle_grace = Duration::from_secs(args.idle_grace);
    options.bus_address = args.bus_address;
    // The launch target survives the process ($APPIMAGE inside an
    // AppImage, else this binary); a bare invocation of this helper IS
    // the daemon, so no subcommand is appended.
    options.autostart_exec = launch_target().map(|exe| exec_value(&exe));
    options.shortcuts = flowshot_daemon::shortcut::ShortcutOptions::production();
    // The first-launch consent prompt (the daemon is the single prompt
    // owner; the child loads/writes the same config this binary loaded).
    options.consent =
        flowshot_daemon::execute::consent::ConsentPrompt::daemon_startup(args.config.clone());
    // The executing sink (bus/tray/shortcut commands run the real
    // capture pipeline; the CLI's `flowshot daemon` installs the same).
    options.command_sink = Some(std::sync::Arc::new(
        flowshot_daemon::execute::ExecutingSink::new(flowshot_daemon::execute::ExecCtx {
            config_path: args.config.clone(),
            state: Some(std::sync::Arc::clone(&options.state)),
            notifier: None,
            upload_base_url: None,
        }),
    ));

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
/// (first run) or unreadable/corrupt (resilience rule: never fail
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

#[cfg(test)]
mod tests {
    use super::DEFAULT_LOG_FILTER;

    #[test]
    fn default_filter_suppresses_the_three_upstream_spam_targets() {
        assert!(DEFAULT_LOG_FILTER.contains("zbus::proxy=error"));
        assert!(DEFAULT_LOG_FILTER.contains("wgpu_hal::vulkan::conv=error"));
        assert!(DEFAULT_LOG_FILTER.contains("wgpu_hal::vulkan::instance=error"));
    }

    #[test]
    fn default_filter_keeps_the_info_base_and_touches_no_flowshot_crate() {
        assert!(DEFAULT_LOG_FILTER.starts_with("info,"));
        assert!(!DEFAULT_LOG_FILTER.contains("flowshot"));
    }
}

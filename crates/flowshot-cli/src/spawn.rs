//! Auto-spawn of the helper daemon (the winner branch of the
//! dispatch handshake).
//!
//! Finding (portal app id): `xdp` `GlobalShortcuts` requires the
//! daemon to run under an `app-*` systemd unit, so when a systemd USER
//! session is present the spawn is wrapped in
//! `systemd-run --user --scope --unit app-org.flowoss.FlowShot-<nonce>`.
//! The wrap is a pure enhancement (init-agnostic): without
//! systemd - or when `systemd-run` fails to spawn - the helper starts
//! directly and everything except portal shortcuts keeps working.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use flowshot_daemon::autostart::launch_target;

use crate::exit::CliError;

/// The systemd-presence facts the wrap decision needs (injected in tests;
/// read from the system in production).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Systemd {
    /// `/run/systemd/system` exists (system manager is systemd).
    pub system_running: bool,
    /// `XDG_RUNTIME_DIR` is set and non-empty (a user session exists).
    pub user_runtime: bool,
    /// Resolved `systemd-run` executable.
    pub systemd_run: Option<PathBuf>,
}

impl Systemd {
    /// Reads the facts from the running system.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            system_running: Path::new("/run/systemd/system").is_dir(),
            user_runtime: std::env::var_os("XDG_RUNTIME_DIR")
                .is_some_and(|value| !value.is_empty()),
            systemd_run: which("systemd-run"),
        }
    }

    /// Whether the scope wrap applies.
    #[must_use]
    pub const fn wrap_applies(&self) -> bool {
        self.system_running && self.user_runtime && self.systemd_run.is_some()
    }
}

/// The pure spawn decision (unit-tested without spawning anything).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperPlan {
    /// Start the helper directly.
    Direct {
        /// The `flowshot` executable.
        exe: PathBuf,
        /// `daemon --auto-spawned [--bus-address ADDR]`.
        args: Vec<OsString>,
    },
    /// Start the helper inside a transient `app-*` scope.
    Scope {
        /// The resolved `systemd-run`.
        systemd_run: PathBuf,
        /// The transient unit name (`app-org.flowoss.FlowShot-<nonce>`).
        unit: String,
        /// The `flowshot` executable.
        exe: PathBuf,
        /// `daemon --auto-spawned [--bus-address ADDR]`.
        args: Vec<OsString>,
    },
}

/// Builds the helper argv: `daemon --auto-spawned [--bus-address ADDR]`
/// (the spawned helper inherits the environment, so `RUST_LOG` and the
/// `FLOWSHOT_SHORTCUTS_PORTAL` QA override carry over).
#[must_use]
pub fn helper_args(bus_address: Option<&str>) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["daemon", "--auto-spawned"]
        .iter()
        .map(|token| OsString::from(*token))
        .collect();
    if let Some(address) = bus_address {
        args.push(OsString::from("--bus-address"));
        args.push(OsString::from(address));
    }
    args
}

/// The scope unit name (the `app-` prefix is what the portal's
/// `GlobalShortcuts` backend accepts as an app id). The nonce is DECIMAL:
/// xdp derives the app id by splitting the unit at the LAST dash, and a
/// live probe confirmed the digit-random form registers
/// (an extra dash segment corrupts the derived app id -> `NotAllowed`).
#[must_use]
pub fn scope_unit(nonce: u128) -> String {
    format!("app-org.flowoss.FlowShot-{nonce}")
}

/// Decides between the direct and scope-wrapped spawn.
#[must_use]
pub fn helper_plan(
    exe: &Path,
    systemd: &Systemd,
    nonce: u128,
    bus_address: Option<&str>,
) -> HelperPlan {
    let args = helper_args(bus_address);
    match &systemd.systemd_run {
        Some(systemd_run) if systemd.wrap_applies() => HelperPlan::Scope {
            systemd_run: systemd_run.clone(),
            unit: scope_unit(nonce),
            exe: exe.to_path_buf(),
            args,
        },
        _ => HelperPlan::Direct {
            exe: exe.to_path_buf(),
            args,
        },
    }
}

/// Translates a plan into the detached child command (null stdio: the
/// helper must never write into the CLI's stdout - `--raw` contract).
fn plan_command(plan: &HelperPlan) -> Command {
    let mut command = match plan {
        HelperPlan::Direct { exe, args } => {
            let mut command = Command::new(exe);
            command.args(args);
            command
        }
        HelperPlan::Scope {
            systemd_run,
            unit,
            exe,
            args,
        } => {
            let mut command = Command::new(systemd_run);
            command
                .arg("--user")
                .arg("--scope")
                .arg(format!("--unit={unit}"))
                .arg(exe)
                .args(args);
            command
        }
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Spawns the auto-spawned helper daemon, detached, and returns once the
/// child is started (name acquisition is awaited by the caller).
///
/// # Errors
///
/// [`CliError::Spawn`] when the current exe cannot be resolved or the
/// direct spawn fails (a failed scope wrap falls back to the direct spawn).
pub fn helper(bus_address: Option<&str>) -> Result<(), CliError> {
    // The helper is spawned DETACHED and this process then exits: inside an
    // AppImage, current_exe is the ephemeral FUSE mount that dies at
    // unmount, so the spawn must reference the stable $APPIMAGE path - the
    // spawned daemon then performs its own mount and owns its lifecycle.
    let exe = launch_target().ok_or_else(|| {
        CliError::Spawn(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not resolve the daemon launch target (neither $APPIMAGE nor the current exe)",
        ))
    })?;
    let systemd = Systemd::detect();
    let plan = helper_plan(&exe, &systemd, unique_nonce(), bus_address);
    match plan_command(&plan).spawn() {
        Ok(_child) => {
            tracing::debug!(kind = plan_kind(&plan), "auto-spawned the helper daemon");
            Ok(())
        }
        Err(error) => match plan {
            // The wrap is a pure enhancement: fall back to the direct spawn.
            HelperPlan::Scope { .. } => {
                tracing::warn!(%error, "the systemd-run scope wrap failed to spawn; falling back to a direct spawn");
                let direct = HelperPlan::Direct {
                    exe,
                    args: helper_args(bus_address),
                };
                plan_command(&direct).spawn().map_err(CliError::Spawn)?;
                Ok(())
            }
            HelperPlan::Direct { .. } => Err(CliError::Spawn(error)),
        },
    }
}

fn plan_kind(plan: &HelperPlan) -> &'static str {
    match plan {
        HelperPlan::Direct { .. } => "direct",
        HelperPlan::Scope { .. } => "systemd-scope",
    }
}

/// A per-spawn unique nonce (nanos mixed with the pid; a unit-name
/// collision merely fails the wrap, which falls back to the direct spawn).
fn unique_nonce() -> u128 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u128::from(elapsed.subsec_nanos()) + u128::from(elapsed.as_secs()) * 1_000_000_000
        });
    nanos ^ (u128::from(std::process::id()) << 64)
}

/// Resolves an executable on `PATH`.
fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn systemd(run: Option<&str>) -> Systemd {
        Systemd {
            system_running: true,
            user_runtime: true,
            systemd_run: run.map(PathBuf::from),
        }
    }

    #[test]
    fn scope_wrap_applies_only_with_all_three_facts() {
        let exe = Path::new("/usr/bin/flowshot");
        let plan = helper_plan(exe, &systemd(Some("/usr/bin/systemd-run")), 0xab, None);
        assert_eq!(
            plan,
            HelperPlan::Scope {
                systemd_run: PathBuf::from("/usr/bin/systemd-run"),
                unit: "app-org.flowoss.FlowShot-171".to_owned(),
                exe: exe.to_path_buf(),
                args: helper_args(None),
            }
        );
        for absent in [
            Systemd {
                system_running: false,
                ..systemd(Some("/usr/bin/systemd-run"))
            },
            Systemd {
                user_runtime: false,
                ..systemd(Some("/usr/bin/systemd-run"))
            },
            systemd(None),
        ] {
            assert!(
                matches!(
                    helper_plan(exe, &absent, 1, None),
                    HelperPlan::Direct { .. }
                ),
                "wrap must degrade to direct for {absent:?}"
            );
        }
    }

    #[test]
    fn bus_address_rides_along_in_both_plans() {
        let exe = Path::new("/usr/bin/flowshot");
        let plan = helper_plan(exe, &Systemd::default(), 1, Some("unix:path=/tmp/bus"));
        match plan {
            HelperPlan::Direct { args, .. } => assert_eq!(
                args,
                vec![
                    OsString::from("daemon"),
                    OsString::from("--auto-spawned"),
                    OsString::from("--bus-address"),
                    OsString::from("unix:path=/tmp/bus"),
                ]
            ),
            HelperPlan::Scope { .. } => panic!("no systemd facts were given"),
        }
    }

    #[test]
    fn unit_names_carry_the_app_prefix_the_portal_requires() {
        assert!(scope_unit(0x1234).starts_with("app-org.flowoss.FlowShot-"));
        assert_ne!(scope_unit(1), scope_unit(2));
    }
}

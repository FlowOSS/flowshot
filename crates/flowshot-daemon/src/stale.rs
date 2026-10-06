//! Stale-binary self-check (Linux's `/proc/self/exe` contract).
//!
//! After a rebuild, a resident daemon runs a superseded image: its
//! `/proc/self/exe` resolves with a `" (deleted)"` suffix and its
//! session-child spawns (which re-resolve `current_exe`) fail with the
//! confusing "io failed: No such file or directory" (observed live
//! 2026-10-04: a daemon auto-spawned before a rebuild kept the bus name
//! through it). The daemon probes its own binary identity on every
//! incoming `D-Bus` dispatch ([`crate::bus`]) and, when the image was
//! superseded, exits cleanly WITHOUT serving the command
//! ([`ShutdownReason::Superseded`]). The pending call is answered by the
//! broker with `NoReply` once the connection closes (empirically verified
//! against `dbus-daemon` by `tests/supersession.rs`) - one of the
//! owner-vanished errors the CLI's single dispatch retry remedies by
//! re-handshaking, which then acquires the free name and spawns a fresh
//! daemon from the new binary. The client's call never hangs and never
//! runs on the stale image.
//!
//! Platform honesty: the probe reads `/proc/self/exe` - Linux's contract.
//! Where it is unavailable, [`capture_exe_identity`] yields `None` and the
//! gate is inert (never stale); a failing probe is ALWAYS conservative
//! (never stale), so a `/proc` hiccup cannot kill a healthy daemon. The
//! codebase is Linux-only today; this degrades instead of pretending.
//!
//! [`ShutdownReason::Superseded`]: crate::daemon::ShutdownReason::Superseded

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::Notify;

/// The running binary's magic link (Linux).
const PROC_SELF_EXE: &str = "/proc/self/exe";

/// The suffix the kernel appends to `/proc/self/exe` when the running
/// inode has been unlinked (replaced or removed on disk).
const DELETED_SUFFIX: &str = "(deleted)";

/// The running binary's inode identity, snapshotted at daemon startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExeIdentity {
    /// The containing device (`st_dev`).
    pub device: u64,
    /// The inode (`st_ino`).
    pub inode: u64,
    /// The last modification time (`st_mtime`).
    pub modified: SystemTime,
}

/// One staleness probe's observations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExeProbe {
    /// `readlink /proc/self/exe` (`std::env::current_exe`), when readable.
    pub link: Option<PathBuf>,
    /// `stat /proc/self/exe` (the RUNNING inode - it follows the magic
    /// link and succeeds even for a deleted target), when readable.
    pub identity: Option<ExeIdentity>,
}

/// The pure staleness decision: `baseline` is the startup snapshot,
/// `probe` the current observation.
///
/// Stale when either:
///
/// 1. the `/proc/self/exe` link carries the kernel's `" (deleted)"`
///    suffix - the running inode was unlinked (cargo's rebuild-and-rename
///    supersession). The identity comparison CANNOT see this case: the
///    stat follows to the same deleted inode and still matches;
/// 2. the running inode's identity moved off the baseline - a different
///    file, or the same file modified in place (mtime moved).
///
/// A probe that read nothing is conservative: NOT stale.
#[must_use]
pub fn exe_is_stale(baseline: &ExeIdentity, probe: &ExeProbe) -> bool {
    if probe.link.as_deref().is_some_and(is_deleted_link) {
        return true;
    }
    probe.identity.is_some_and(|current| current != *baseline)
}

/// Whether a resolved `/proc/self/exe` link carries the deleted-inode
/// suffix (the lossy conversion preserves the ASCII suffix regardless of
/// the path's own encoding).
fn is_deleted_link(link: &Path) -> bool {
    link.as_os_str().to_string_lossy().ends_with(DELETED_SUFFIX)
}

/// Snapshots the running binary's identity (`None` without a readable
/// `/proc/self/exe` - the gate then stays inert).
#[must_use]
pub fn capture_exe_identity() -> Option<ExeIdentity> {
    identity_of_self_exe().ok()
}

/// Runs one live probe (readlink + stat of `/proc/self/exe`; two cheap
/// syscalls per user-initiated dispatch - no throttle needed).
#[must_use]
pub fn probe_exe() -> ExeProbe {
    ExeProbe {
        link: std::env::current_exe().ok(),
        identity: identity_of_self_exe().ok(),
    }
}

#[cfg(unix)]
fn identity_of_self_exe() -> std::io::Result<ExeIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(PROC_SELF_EXE)?;
    Ok(ExeIdentity {
        device: meta.dev(),
        inode: meta.ino(),
        modified: meta.modified()?,
    })
}

#[cfg(not(unix))]
fn identity_of_self_exe() -> std::io::Result<ExeIdentity> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no /proc/self/exe identity off unix",
    ))
}

/// The dispatch-time gate: the startup identity baseline plus the
/// daemon's clean-exit trigger.
///
/// A gate without a baseline is inert: [`Self::is_stale`] is always
/// `false` (test stubs, platforms without `/proc/self/exe`).
#[derive(Debug, Clone)]
pub struct SupersessionGate {
    baseline: Option<ExeIdentity>,
    exit: Arc<Notify>,
}

impl SupersessionGate {
    /// The production gate: snapshots the running binary now; `exit`
    /// fires the daemon's clean shutdown when a dispatch finds the image
    /// superseded (the composition root's shutdown select watches it).
    #[must_use]
    pub fn new(exit: Arc<Notify>) -> Self {
        Self::with_baseline(capture_exe_identity(), exit)
    }

    /// A gate against an injected baseline (the `bus_address` QA-knob
    /// precedent: tests force the superseded path with a foreign
    /// identity; `None` builds an inert gate).
    #[must_use]
    pub const fn with_baseline(baseline: Option<ExeIdentity>, exit: Arc<Notify>) -> Self {
        Self { baseline, exit }
    }

    /// Whether the running binary was superseded on disk (a failing
    /// probe is conservative: never stale).
    #[must_use]
    pub fn is_stale(&self) -> bool {
        let Some(baseline) = self.baseline else {
            return false;
        };
        exe_is_stale(&baseline, &probe_exe())
    }

    /// Fires the clean daemon exit.
    pub fn trigger_exit(&self) {
        self.exit.notify_one();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::time::Duration;

    fn identity(inode: u64, modified: SystemTime) -> ExeIdentity {
        ExeIdentity {
            device: 1,
            inode,
            modified,
        }
    }

    fn probe(link: &str, identity: ExeIdentity) -> ExeProbe {
        ExeProbe {
            link: Some(PathBuf::from(link)),
            identity: Some(identity),
        }
    }

    #[test]
    fn deleted_link_suffix_is_stale_even_with_matching_identity() {
        // The cargo-rename supersession: stat still follows to the deleted
        // inode and matches the baseline - only the suffix reveals it.
        let baseline = identity(7, SystemTime::UNIX_EPOCH);
        assert!(exe_is_stale(
            &baseline,
            &probe("/usr/bin/flowshot (deleted)", baseline)
        ));
    }

    #[test]
    fn intact_link_with_matching_identity_is_fresh() {
        let baseline = identity(7, SystemTime::UNIX_EPOCH);
        assert!(!exe_is_stale(
            &baseline,
            &probe("/usr/bin/flowshot", baseline)
        ));
    }

    #[test]
    fn inode_replacement_is_stale() {
        let baseline = identity(7, SystemTime::UNIX_EPOCH);
        let other = identity(8, SystemTime::UNIX_EPOCH);
        assert!(exe_is_stale(&baseline, &probe("/usr/bin/flowshot", other)));
    }

    #[test]
    fn in_place_modification_is_stale() {
        let baseline = identity(7, SystemTime::UNIX_EPOCH);
        let touched = identity(7, SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        assert!(exe_is_stale(
            &baseline,
            &probe("/usr/bin/flowshot", touched)
        ));
    }

    #[test]
    fn unreadable_probe_is_conservatively_fresh() {
        let baseline = identity(7, SystemTime::UNIX_EPOCH);
        assert!(!exe_is_stale(&baseline, &ExeProbe::default()));
        assert!(!exe_is_stale(
            &baseline,
            &ExeProbe {
                link: Some(PathBuf::from("/usr/bin/flowshot")),
                identity: None,
            }
        ));
    }

    #[test]
    fn gate_without_baseline_is_inert() {
        let gate = SupersessionGate::with_baseline(None, Arc::new(Notify::new()));
        assert!(!gate.is_stale());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_gate_reads_fresh_for_the_test_binary_and_stale_for_a_foreign_baseline() {
        let live = capture_exe_identity().expect("a linux test process has /proc/self/exe");
        let fresh = SupersessionGate::with_baseline(Some(live), Arc::new(Notify::new()));
        assert!(!fresh.is_stale());
        let foreign = ExeIdentity {
            inode: live.inode + 1,
            ..live
        };
        let stale = SupersessionGate::with_baseline(Some(foreign), Arc::new(Notify::new()));
        assert!(stale.is_stale());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_probe_of_an_intact_binary_carries_both_observations() {
        let probe = probe_exe();
        let link = probe.link.expect("current_exe resolves on linux");
        assert!(!is_deleted_link(&link), "the test binary is intact");
        assert!(probe.identity.is_some());
    }

    #[tokio::test]
    async fn trigger_exit_wakes_the_shutdown_watcher() {
        let exit = Arc::new(Notify::new());
        let gate = SupersessionGate::with_baseline(None, Arc::clone(&exit));
        let waiter = tokio::spawn(async move { exit.notified().await });
        tokio::task::yield_now().await;
        gate.trigger_exit();
        assert!(
            tokio::time::timeout(Duration::from_secs(5), waiter)
                .await
                .is_ok()
        );
    }
}

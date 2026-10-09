//! Private-broker integration tests: REAL atomic name contention on a
//! throwaway `dbus-daemon` (the p2p stubs cannot emulate broker name
//! ownership - p2p `request_name` is local self-identification).
//!
//! Environment gate: when `dbus-daemon` is absent the tests announce a
//! SKIP loudly and pass - the hermetic p2p coverage of the forwarding
//! mechanics lives in `src/instance.rs` / `src/testsupport.rs`, and the
//! live session-bus proof is in the single-instance QA evidence.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Load-sensitive budget (the AGENTS.md rule: raise budgets, never weaken
/// assertions). The former 15 s acquisition/exit waits timed out on loaded
/// 4-vCPU CI containers while passing on 16-core dev machines; green-path
/// speed is unchanged because the deadline only bounds polling. Same 45 s
/// budget as `testsupport::CALL_TIMEOUT`.
const TEST_BUDGET: Duration = Duration::from_secs(45);

use flowshot_core::Config;
use flowshot_daemon::bus::SERVICE;
use flowshot_daemon::command::{DaemonCommand, RecordingSink};
use flowshot_daemon::daemon::{Daemon, DaemonOptions, ShutdownReason, Startup};
use flowshot_daemon::instance::{Acquisition, acquire_or_forward};
use flowshot_daemon::state::DaemonState;

static BUS_COUNTER: AtomicU64 = AtomicU64::new(0);

const CONFIG_TEMPLATE: &str = r#"<busconfig>
  <type>session</type>
  <listen>unix:path=__SOCK__</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/>
    <allow own="*"/>
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
  </policy>
</busconfig>
"#;

/// A throwaway private bus: unique socket path per spawn (process- and
/// counter-namespaced, so concurrent suites cannot collide), child killed
/// and directory removed on drop.
struct PrivateBus {
    child: Child,
    address: String,
    dir: PathBuf,
}

impl PrivateBus {
    /// Spawns the broker, or returns `None` when `dbus-daemon` is absent.
    fn spawn(tag: &str) -> Option<Self> {
        let probe = Command::new("dbus-daemon").arg("--version").output();
        if probe.is_err() {
            return None;
        }
        let unique = BUS_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = PathBuf::from(format!(
            "/tmp/fsd-bus-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("bus");
        let config_path = dir.join("bus.xml");
        std::fs::write(
            &config_path,
            CONFIG_TEMPLATE.replace("__SOCK__", sock.to_str().unwrap()),
        )
        .unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg("--nofork")
            .arg("--print-address=1")
            .arg(format!("--config-file={}", config_path.display()))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take().unwrap();
        let address = BufReader::new(stdout).lines().next().unwrap().unwrap();
        Some(Self {
            child,
            address,
            dir,
        })
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// Polls the broker until `SERVICE` has an owner (bounded).
async fn wait_for_name(address: &str) {
    let deadline = Instant::now() + TEST_BUDGET;
    loop {
        if let Ok(connection) = flowshot_daemon::instance::connect(Some(address)).await {
            let owned = match zbus::fdo::DBusProxy::new(&connection).await {
                Ok(proxy) => proxy
                    .get_name_owner(SERVICE.try_into().unwrap())
                    .await
                    .is_ok(),
                Err(_) => false,
            };
            let _ = connection.close().await;
            if owned {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the daemon never acquired the name"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Polls the recorder until `expected` appears (bounded).
async fn wait_for_command(recorder: &RecordingSink, expected: &DaemonCommand) {
    let deadline = Instant::now() + TEST_BUDGET;
    loop {
        if recorder.commands().contains(expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "command {expected} never arrived"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn daemon_options(
    address: &str,
    state: Arc<DaemonState>,
    sink: Arc<RecordingSink>,
) -> DaemonOptions {
    let mut options = DaemonOptions::auto_spawned(Config::default());
    options.idle_grace = Duration::from_millis(200);
    options.bus_address = Some(address.to_owned());
    options.state = state;
    options.command_sink = Some(sink);
    options
}

#[tokio::test]
async fn name_contention_forwards_argv_to_the_winner() {
    let Some(bus) = PrivateBus::spawn("contention") else {
        eprintln!("SKIP: dbus-daemon not installed - private-broker contention test not run");
        return;
    };

    // Given a running auto-spawned daemon pinned by the shortcuts reason
    // (so the contention phase cannot race the idle exit).
    let recorder = RecordingSink::new();
    let state = Arc::new(DaemonState::new(false, Instant::now()));
    state.set_shortcuts_registered(true);
    let options = daemon_options(&bus.address, Arc::clone(&state), Arc::new(recorder.clone()));
    let winner = tokio::spawn(async move {
        match Daemon::start(options).await.unwrap() {
            Startup::Running(daemon) => daemon.run().await.unwrap(),
            Startup::AlreadyRunning => panic!("a fresh private bus must be name-free"),
        }
    });
    wait_for_name(&bus.address).await;

    // When a second instance runs the client handshake with its argv.
    let argv = vec!["capture".to_owned(), "screen".to_owned(), "1".to_owned()];
    let outcome = acquire_or_forward(Some(&bus.address), SERVICE, &argv)
        .await
        .unwrap();

    // Then it is the forwarding loser, and the winner observed the Invoke.
    assert!(
        matches!(outcome, Acquisition::Forwarded),
        "the second instance must not win the name"
    );
    wait_for_command(&recorder, &DaemonCommand::Invoke(argv)).await;

    // And releasing the last persistence reason lets the idle grace exit
    // the winner (real clock, short grace - bounded).
    state.set_shortcuts_registered(false);
    let reason = tokio::time::timeout(TEST_BUDGET, winner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason, ShutdownReason::IdleExit);
}

#[tokio::test]
async fn second_daemon_start_reports_already_running() {
    let Some(bus) = PrivateBus::spawn("already") else {
        eprintln!("SKIP: dbus-daemon not installed - private-broker contention test not run");
        return;
    };

    // Given a running daemon pinned by the tray reason.
    let recorder = RecordingSink::new();
    let state = Arc::new(DaemonState::new(true, Instant::now()));
    let options = daemon_options(&bus.address, Arc::clone(&state), Arc::new(recorder.clone()));
    let first = tokio::spawn(async move {
        match Daemon::start(options).await.unwrap() {
            Startup::Running(daemon) => daemon.run().await.unwrap(),
            Startup::AlreadyRunning => panic!("a fresh private bus must be name-free"),
        }
    });
    wait_for_name(&bus.address).await;

    // When a second daemon process starts against the same bus.
    let second_options = daemon_options(
        &bus.address,
        Arc::new(DaemonState::new(false, Instant::now())),
        Arc::new(recorder.clone()),
    );
    let second = Daemon::start(second_options).await.unwrap();

    // Then it defers without touching the object server.
    assert!(
        matches!(second, Startup::AlreadyRunning),
        "the second daemon must defer to the name owner"
    );

    // Cleanup: releasing the tray lets the first daemon idle-exit.
    state.set_tray(false);
    let reason = tokio::time::timeout(TEST_BUDGET, first)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason, ShutdownReason::IdleExit);
}

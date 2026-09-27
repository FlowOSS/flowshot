//! The daemon silent-failure regression (the live-found defect's second
//! half): a forwarded `flowshot capture` whose session child dies used to
//! leave the CLI exiting 0 with no output - the failure existed only in
//! the daemon's log. The bus reply must carry the typed failure so the
//! forwarding CLI exits non-zero with the message on stderr.
//!
//! Harness: a REAL daemon (in-process, private `dbus-daemon` bus - the
//! flowshot-daemon `broker.rs` recipe) with the production
//! [`ExecutingSink`], and the REAL `flowshot` binary forwarding to it. The
//! daemon's session child is `current_exe` = THIS test binary, and libtest
//! rejects the `session --spec` arguments instantly: a deterministic,
//! fast, windowless failing child - exactly the defect's failure class.
//!
//! Environment gate: when `dbus-daemon` is absent the test announces a
//! SKIP loudly and passes (the broker.rs precedent).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use flowshot_core::Config;
use flowshot_daemon::bus::SERVICE;
use flowshot_daemon::daemon::{Daemon, DaemonOptions, ShutdownReason, Startup};
use flowshot_daemon::execute::{ExecCtx, ExecutingSink};
use flowshot_daemon::state::DaemonState;

const FLOWSHOT: &str = env!("CARGO_BIN_EXE_flowshot");

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

/// A throwaway private bus (the broker.rs harness): unique socket path per
/// spawn, child killed and directory removed on drop.
struct PrivateBus {
    child: Child,
    address: String,
    dir: PathBuf,
}

impl PrivateBus {
    fn spawn(tag: &str) -> Option<Self> {
        let probe = Command::new("dbus-daemon").arg("--version").output();
        if probe.is_err() {
            return None;
        }
        let unique = BUS_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = PathBuf::from(format!(
            "/tmp/fsd-cli-bus-{}-{tag}-{unique}",
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
    let deadline = Instant::now() + Duration::from_secs(15);
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

/// Runs the CLI to completion under a budget, killing it on expiry (the
/// daemon replies within its 5 s startup window; the budget only guards
/// against hangs).
fn run_cli_bounded(address: &str) -> std::process::Output {
    let mut child = Command::new(FLOWSHOT)
        .arg("--bus-address")
        .arg(address)
        .arg("capture")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("the CLI did not finish within its budget");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test]
async fn failing_session_child_surfaces_nonzero_exit_and_message_on_the_cli() {
    let Some(bus) = PrivateBus::spawn("surface") else {
        eprintln!("SKIP: dbus-daemon not installed - daemon failure-surface test not run");
        return;
    };

    // Given: a real daemon with the production executing sink on a private
    // bus, pinned alive by the shortcuts reason.
    let state = Arc::new(DaemonState::new(false, Instant::now()));
    state.set_shortcuts_registered(true);
    let mut options = DaemonOptions::auto_spawned(Config::default());
    options.idle_grace = Duration::from_millis(200);
    options.bus_address = Some(bus.address.clone());
    options.state = Arc::clone(&state);
    options.command_sink = Some(Arc::new(ExecutingSink::new(ExecCtx {
        config_path: None,
        state: Some(Arc::clone(&state)),
        notifier: None,
        upload_base_url: None,
    })));
    let daemon = tokio::spawn(async move {
        match Daemon::start(options).await.unwrap() {
            Startup::Running(daemon) => daemon.run().await.unwrap(),
            Startup::AlreadyRunning => panic!("a fresh private bus must be name-free"),
        }
    });
    wait_for_name(&bus.address).await;

    // When: the real CLI forwards an interactive capture, whose session
    // child (this test binary + `session --spec` args) dies instantly.
    let address = bus.address.clone();
    let output = tokio::task::spawn_blocking(move || run_cli_bounded(&address))
        .await
        .unwrap();

    // Then: the CLI exits non-zero with the child failure on stderr
    // (pre-fix: a silent exit 0).
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(
        output.status.code(),
        Some(0),
        "the daemon swallowed the child failure; stderr: {stderr}"
    );
    assert!(
        stderr.contains("session child failed"),
        "stderr must carry the typed child failure: {stderr}"
    );

    // Cleanup: releasing the pin lets the daemon idle-exit (bounded).
    state.set_shortcuts_registered(false);
    let reason = tokio::time::timeout(Duration::from_secs(15), daemon)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason, ShutdownReason::IdleExit);
}

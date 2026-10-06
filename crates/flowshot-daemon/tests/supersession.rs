//! Stale-binary supersession (rider B): a daemon whose on-disk image was
//! replaced exits UNSERVED on the next dispatch, and the broker answers
//! the pending call with `NoReply` - one of the two owner-vanished errors
//! the CLI's single dispatch retry remedies by respawning a fresh daemon
//! (`flowshot-cli`'s `dispatch_bus`). This test is the empirical oracle
//! for that broker behavior (dbus-daemon answers a call pending on a
//! disconnected connection with `NoReply`, NOT `NameHasNoOwner`).
//! Private-broker integration test (the `broker.rs` precedent): the p2p
//! stubs cannot emulate the broker's answer-on-disconnect behavior this
//! contract depends on.
//!
//! Environment gate: when `dbus-daemon` is absent the test announces a
//! SKIP loudly and passes (the pure staleness decision is unit-tested in
//! `src/stale.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use flowshot_core::Config;
use flowshot_daemon::bus::{IFACE, OBJECT_PATH, SERVICE};
use flowshot_daemon::command::RecordingSink;
use flowshot_daemon::daemon::{Daemon, DaemonOptions, ShutdownReason, Startup};
use flowshot_daemon::stale::{ExeIdentity, capture_exe_identity};
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

/// A throwaway private bus (the `broker.rs` harness pattern: unique
/// socket path per spawn, child killed and directory removed on drop).
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
            "/tmp/fsd-supersession-{}-{tag}-{unique}",
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

#[tokio::test]
async fn superseded_daemon_exits_unserved_and_the_broker_answers_no_reply() {
    let Some(bus) = PrivateBus::spawn("stale") else {
        eprintln!("SKIP: dbus-daemon not installed - supersession broker test not run");
        return;
    };

    // Given: a running daemon whose binary identity no longer matches
    // disk (the injected foreign baseline is the post-rebuild state:
    // every dispatch finds the image superseded). The tray reason pins
    // it, so the idle exit cannot race the supersession exit.
    let recorder = RecordingSink::new();
    let state = Arc::new(DaemonState::new(false, Instant::now()));
    state.set_tray(true);
    let live = capture_exe_identity().expect("the test process has a readable /proc/self/exe");
    let foreign = ExeIdentity {
        inode: live.inode + 1,
        ..live
    };
    let mut options = DaemonOptions::auto_spawned(Config::default());
    options.idle_grace = Duration::from_secs(3600);
    options.bus_address = Some(bus.address.clone());
    options.state = Arc::clone(&state);
    options.command_sink = Some(Arc::new(recorder.clone()));
    options.supersession_baseline = Some(foreign);
    let daemon = tokio::spawn(async move {
        match Daemon::start(options).await.unwrap() {
            Startup::Running(daemon) => daemon.run().await.unwrap(),
            Startup::AlreadyRunning => panic!("a fresh private bus must be name-free"),
        }
    });
    wait_for_name(&bus.address).await;

    // When: a client calls the superseded daemon.
    let client = flowshot_daemon::instance::connect(Some(&bus.address))
        .await
        .unwrap();
    let call = client.call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "CaptureFull", &());
    let error = tokio::time::timeout(Duration::from_secs(30), call)
        .await
        .expect("the parked call must be answered by the broker, never hang")
        .expect_err("a superseded daemon must not serve the call");

    // Then: the broker answers the parked call with NoReply once the
    // daemon's shutdown closed its connection - an owner-vanished signal
    // the CLI's single dispatch retry turns into a fresh-daemon respawn.
    match error {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(name.as_str(), "org.freedesktop.DBus.Error.NoReply");
        }
        other => panic!("unexpected client error: {other}"),
    }

    // And: the daemon exited as Superseded without serving the command.
    let reason = tokio::time::timeout(Duration::from_secs(15), daemon)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason, ShutdownReason::Superseded);
    assert_eq!(
        recorder.commands(),
        [] as [flowshot_daemon::DaemonCommand; 0]
    );
    let _ = client.close().await;
}

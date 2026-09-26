//! Tray integration on a private broker (the todo-32 dbus-daemon recipe):
//! a stub `org.kde.StatusNotifierWatcher` proves the registration
//! handshake, the menu wire answers `GetLayout`, and `Event(clicked)`
//! activations land on the `CommandSink` dispatch table. The watcher-absent
//! degrade and the `[daemon].tray` config gate are the negative cases.
//!
//! Environment gate: without `dbus-daemon` the tests announce a SKIP
//! loudly and pass (the pure dispatch table is unit-tested in
//! `src/tray/menu.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use flowshot_core::geometry::{
    Logical, LogicalRect, OutputInfo, PhysicalPx, PhysicalSize, Transform,
};
use flowshot_core::{Config, config::DaemonConfig};
use flowshot_daemon::bus::{IFACE, OBJECT_PATH, SERVICE};
use flowshot_daemon::command::{DaemonCommand, RecordingSink};
use flowshot_daemon::daemon::{Daemon, DaemonOptions, ShutdownReason, Startup};
use flowshot_daemon::notify::{NotificationRecord, RecordingNotifier};
use flowshot_daemon::request::CaptureRequest;
use flowshot_daemon::tray::menu::{
    ABOUT_ID, CAPTURE_FULL_ID, CONFIGURE_ID, LAUNCHER_ID, QUIT_ID, SCREEN_BASE, SCREEN_SUBMENU_ID,
    TAKE_SCREENSHOT_ID,
};
use flowshot_daemon::tray::outputs::OutputProbe;
use flowshot_daemon::tray::spec::{MENU_INTERFACE, MENU_PATH, WATCHER_PATH, WATCHER_SERVICE};
use zbus::zvariant::{OwnedValue, Value};

static BUS_COUNTER: AtomicU64 = AtomicU64::new(0);
const CALL_TIMEOUT: Duration = Duration::from_secs(15);

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
            "/tmp/fsd-tray-{}-{tag}-{unique}",
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

#[derive(Debug)]
struct StubWatcher {
    recorded: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl StubWatcher {
    fn register_status_notifier_item(&self, service: String) {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(service);
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[zbus(property)]
    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    #[expect(
        clippy::unused_self,
        reason = "spec-constant property; the zbus interface macro requires a receiver"
    )]
    fn protocol_version(&self) -> i32 {
        0
    }
}

/// Owns the watcher name on the bus; `recorded` survives the handle.
async fn spawn_stub_watcher(address: &str) -> (zbus::Connection, Arc<Mutex<Vec<String>>>) {
    let connection = flowshot_daemon::instance::connect(Some(address))
        .await
        .unwrap();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let stub = StubWatcher {
        recorded: Arc::clone(&recorded),
    };
    connection
        .object_server()
        .at(WATCHER_PATH, stub)
        .await
        .unwrap();
    connection.request_name(WATCHER_SERVICE).await.unwrap();
    (connection, recorded)
}

fn recorded_services(recorded: &Mutex<Vec<String>>) -> Vec<String> {
    recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[derive(Debug)]
struct StubProbe(Vec<OutputInfo>);

impl OutputProbe for StubProbe {
    fn probe(&self) -> Vec<OutputInfo> {
        self.0.clone()
    }
}

fn stub_outputs() -> Vec<OutputInfo> {
    let output = |connector: &str, name: &str, x: f64| {
        OutputInfo::new(
            connector,
            name,
            LogicalRect::new(Logical(x), Logical(0.0), Logical(1920.0), Logical(1080.0)),
            PhysicalSize::new(PhysicalPx(1920), PhysicalPx(1080)),
            1.0,
            Transform::Normal,
        )
        .unwrap()
    };
    vec![
        output("DP-1", "Monitor One", 0.0),
        output("HDMI-A-1", "", 1920.0),
    ]
}

fn tray_options(address: &str, tray_enabled: bool, sink: Arc<RecordingSink>) -> DaemonOptions {
    let config = Config {
        daemon: DaemonConfig {
            tray: tray_enabled,
            ..DaemonConfig::default()
        },
        ..Config::default()
    };
    let mut options = DaemonOptions::auto_spawned(config);
    options.idle_grace = Duration::from_secs(3600);
    options.bus_address = Some(address.to_owned());
    options.command_sink = Some(sink);
    options.tray.probe = Some(Arc::new(StubProbe(stub_outputs())));
    options
}

fn spawn_daemon(options: DaemonOptions) -> tokio::task::JoinHandle<ShutdownReason> {
    tokio::spawn(async move {
        match Daemon::start(options).await.unwrap() {
            Startup::Running(daemon) => daemon.run().await.unwrap(),
            Startup::AlreadyRunning => panic!("a fresh private bus must be name-free"),
        }
    })
}

async fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + CALL_TIMEOUT;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

type WireProps = HashMap<String, OwnedValue>;
type WireNode = (i32, WireProps, Vec<OwnedValue>);

fn to_value(value: &OwnedValue) -> Value<'static> {
    Value::from(value.try_clone().unwrap())
}

fn parse_node(value: &OwnedValue) -> WireNode {
    let Value::Structure(structure) = to_value(value) else {
        panic!("a menu child must be a (ia{{sv}}av) structure");
    };
    WireNode::try_from(structure).unwrap()
}

fn label_of(props: &WireProps) -> Option<String> {
    props
        .get("label")
        .map(|value| String::try_from(to_value(value)).unwrap())
}

async fn get_layout(client: &zbus::Connection, service: &str) -> zbus::Result<(u32, WireNode)> {
    let reply = client
        .call_method(
            Some(service),
            MENU_PATH,
            Some(MENU_INTERFACE),
            "GetLayout",
            &(0i32, -1i32, Vec::<String>::new()),
        )
        .await?;
    reply.body().deserialize::<(u32, WireNode)>()
}

async fn click(client: &zbus::Connection, service: &str, id: i32) -> zbus::Result<()> {
    client
        .call_method(
            Some(service),
            MENU_PATH,
            Some(MENU_INTERFACE),
            "Event",
            &(id, "clicked", Value::Bool(false), 0u32),
        )
        .await?;
    Ok(())
}

async fn layout_with_submenu_children(client: &zbus::Connection, service: &str) -> (u32, WireNode) {
    let deadline = Instant::now() + CALL_TIMEOUT;
    loop {
        if let Ok((revision, root)) = get_layout(client, service).await {
            let submenu_filled = root
                .2
                .iter()
                .map(parse_node)
                .any(|(id, _, children)| id == SCREEN_SUBMENU_ID && !children.is_empty());
            if submenu_filled {
                return (revision, root);
            }
        }
        assert!(
            Instant::now() < deadline,
            "the per-monitor submenu never gained children"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_command(sink: &RecordingSink, expected: &DaemonCommand) {
    wait_for(&format!("command {expected}"), || {
        sink.commands().into_iter().find(|c| c == expected)
    })
    .await;
}

#[tokio::test]
async fn registered_tray_serves_the_menu_and_dispatches_every_action() {
    let Some(bus) = PrivateBus::spawn("full") else {
        eprintln!("SKIP: dbus-daemon not installed - tray broker test not run");
        return;
    };
    // Given a watcher host and a tray-enabled daemon with a stub probe.
    let (_watcher_conn, recorded) = spawn_stub_watcher(&bus.address).await;
    let sink = RecordingSink::new();
    let notifier = RecordingNotifier::new();
    let mut options = tray_options(&bus.address, true, Arc::new(sink.clone()));
    options.notifier = Some(Arc::new(notifier.clone()));
    let state = Arc::clone(&options.state);
    let daemon = spawn_daemon(options);

    // When the registration task runs, the watcher sees the daemon's
    // unique name and the tray persistence reason is held.
    let service = wait_for("tray registration", || recorded_services(&recorded).pop()).await;
    assert!(service.starts_with(':'), "unique name expected: {service}");
    wait_for("tray persistence reason", || {
        state.reasons().tray.then_some(())
    })
    .await;

    // Then the menu answers GetLayout with the parity entries and the
    // live-probed per-monitor children.
    let client = flowshot_daemon::instance::connect(Some(&bus.address))
        .await
        .unwrap();
    let (_revision, root) = layout_with_submenu_children(&client, &service).await;
    assert_eq!(root.0, 0, "the dbusmenu root id is 0");
    let children: Vec<WireNode> = root.2.iter().map(parse_node).collect();
    let ids: Vec<i32> = children.iter().map(|node| node.0).collect();
    assert_eq!(
        ids,
        vec![
            TAKE_SCREENSHOT_ID,
            CAPTURE_FULL_ID,
            3, // the per-monitor submenu root
            LAUNCHER_ID,
            5, // separator
            CONFIGURE_ID,
            ABOUT_ID,
            8, // separator
            QUIT_ID,
        ]
    );
    assert_eq!(label_of(&children[0].1).as_deref(), Some("Take Screenshot"));
    assert!(
        children[4].1.contains_key("type"),
        "separators carry a type"
    );
    let submenu = &children[2];
    assert!(matches!(
        submenu.1.get("children-display").map(|v| String::try_from(to_value(v)).unwrap()),
        Some(ref marker) if marker == "submenu"
    ));
    let screens: Vec<WireNode> = submenu.2.iter().map(parse_node).collect();
    assert_eq!(screens.len(), 2);
    assert_eq!(screens[0].0, SCREEN_BASE);
    assert_eq!(screens[1].0, SCREEN_BASE + 1);
    assert_eq!(
        label_of(&screens[0].1).as_deref(),
        Some("Screen 0: Monitor One")
    );
    assert_eq!(
        label_of(&screens[1].1).as_deref(),
        Some("Screen 1: HDMI-A-1")
    );

    // When each entry is activated through the wire Event callback, the
    // dispatch table lands the typed command on the sink.
    click(&client, &service, TAKE_SCREENSHOT_ID).await.unwrap();
    wait_for_command(&sink, &DaemonCommand::Capture(CaptureRequest::default())).await;
    click(&client, &service, CAPTURE_FULL_ID).await.unwrap();
    wait_for_command(&sink, &DaemonCommand::CaptureFull).await;
    click(&client, &service, LAUNCHER_ID).await.unwrap();
    wait_for_command(&sink, &DaemonCommand::Launcher).await;
    click(&client, &service, CONFIGURE_ID).await.unwrap();
    wait_for_command(&sink, &DaemonCommand::Settings).await;
    click(&client, &service, SCREEN_BASE).await.unwrap();
    wait_for_command(&sink, &DaemonCommand::CaptureScreen(0)).await;
    click(&client, &service, SCREEN_BASE + 1).await.unwrap();
    wait_for_command(&sink, &DaemonCommand::CaptureScreen(1)).await;

    // About is a toast, not a command.
    click(&client, &service, ABOUT_ID).await.unwrap();
    wait_for("the about toast", || {
        notifier
            .records()
            .into_iter()
            .find(|r| matches!(r, NotificationRecord::About(_)))
    })
    .await;

    // Unknown ids are rejected, not dispatched.
    let unknown = click(&client, &service, 12_345).await;
    assert!(
        matches!(&unknown, Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.InvalidArgs"),
        "unexpected reply for an unknown id: {unknown:?}"
    );

    // Quit exits the daemon cleanly and releases the tray reason. The
    // reply can race the shutdown (the daemon may close before the Event
    // answer is delivered) - the exit reason is the contract, not the
    // reply.
    let _ = click(&client, &service, QUIT_ID).await;
    let reason = tokio::time::timeout(CALL_TIMEOUT, daemon)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason, ShutdownReason::Quit);
    assert!(!state.reasons().tray, "shutdown releases the tray reason");
}

#[tokio::test]
async fn absent_watcher_disables_the_tray_but_the_daemon_keeps_serving() {
    let Some(bus) = PrivateBus::spawn("no-watcher") else {
        eprintln!("SKIP: dbus-daemon not installed - tray broker test not run");
        return;
    };
    // Given a tray-enabled daemon on a bus with NO watcher.
    let sink = RecordingSink::new();
    let options = tray_options(&bus.address, true, Arc::new(sink.clone()));
    let state = Arc::clone(&options.state);
    let daemon = spawn_daemon(options);
    let client = flowshot_daemon::instance::connect(Some(&bus.address))
        .await
        .unwrap();

    // When the tray host starts, the bus objects exist (the late-arrival
    // retry needs them) but the persistence reason is NEVER held.
    let deadline = Instant::now() + CALL_TIMEOUT;
    loop {
        if get_layout(&client, SERVICE).await.is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "the tray objects never appeared");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for _ in 0..10 {
        assert!(
            !state.reasons().tray,
            "an absent watcher must not hold the tray reason"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Then the D-Bus service still answers (zero daemon impact).
    client
        .call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "Launcher", &())
        .await
        .unwrap();
    wait_for_command(&sink, &DaemonCommand::Launcher).await;

    // And the menu's Quit still exits cleanly (the reply can race the
    // shutdown - the exit reason is the contract, not the reply).
    let _ = click(&client, SERVICE, QUIT_ID).await;
    let reason = tokio::time::timeout(CALL_TIMEOUT, daemon)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason, ShutdownReason::Quit);
}

#[tokio::test]
async fn config_gate_off_never_registers_and_idle_exits() {
    let Some(bus) = PrivateBus::spawn("gate-off") else {
        eprintln!("SKIP: dbus-daemon not installed - tray broker test not run");
        return;
    };
    // Given a watcher host but [daemon].tray = false and a short grace.
    let (_watcher_conn, recorded) = spawn_stub_watcher(&bus.address).await;
    let sink = RecordingSink::new();
    let mut options = tray_options(&bus.address, false, Arc::new(sink.clone()));
    options.idle_grace = Duration::from_millis(200);
    let state = Arc::clone(&options.state);
    let daemon = spawn_daemon(options);

    // When the daemon runs, no tray registration ever reaches the watcher
    // and the (never held) tray reason lets the idle grace exit it.
    let reason = tokio::time::timeout(CALL_TIMEOUT, daemon)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason, ShutdownReason::IdleExit);
    assert!(
        recorded_services(&recorded).is_empty(),
        "the config gate must keep the tray off the bus"
    );
    assert!(!state.reasons().tray);
}

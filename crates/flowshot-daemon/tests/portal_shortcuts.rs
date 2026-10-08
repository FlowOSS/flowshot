//! Stub-portal e2e for the shortcut-fallback ladder (the "stub
//! portal (private bus) for the portal path" requirement).
//!
//! The REAL production code path runs against a REAL `dbus-daemon` broker
//! hosting a stub `org.freedesktop.portal.Desktop`: `ashpd`'s portal
//! connection is a process-global `OnceLock` bound to
//! `DBUS_SESSION_BUS_ADDRESS` (vendored `proxy.rs` - the pin recorded in
//! `src/shortcut/portal.rs`), so the test steers it by setting that
//! variable ONCE, before any `ashpd` use, under a file-level lock that
//! serializes every test in this binary (no concurrent `getenv` readers;
//! lib-unit tests run in a separate binary and never touch `ashpd`).
//!
//! The stub reproduces the wire contract `ashpd` 0.13.13 codes against
//! (source-verified): request paths
//! `/org/freedesktop/portal/desktop/request/{SENDER}/{TOKEN}`, session
//! paths `.../session/{SENDER}/{TOKEN}`, `Response(u, a{sv})` signals on
//! `org.freedesktop.portal.Request`, and the real xdp quirk that
//! `session_handle` arrives as a STRING ('s'), not an object path (see
//! ashpd `session.rs` `CreateSessionResponse` comment).
//!
//! Ladder phases in ONE runtime (the `OnceLock` permits exactly one portal
//! bus per process): A) portal absent -> Hyprland fallback, nothing
//! persisted, no nudge, no persistence reason; B) stub portal appears ->
//! registration binds the three defaults, restore data persists, the
//! one-time nudge fires, the `shortcuts` reason holds; C) `Activated`
//! signals dispatch mapped commands and foreign ids are ignored; D)
//! shutdown closes the session and releases the reason; E) restart reuses
//! the restore data WITHOUT a second nudge.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use flowshot_capture::DesktopEnv;
use flowshot_daemon::command::{DaemonCommand, RecordingSink};
use flowshot_daemon::notify::{NotificationRecord, RecordingNotifier};
use flowshot_daemon::shortcut::persist::RestoreData;
use flowshot_daemon::shortcut::{
    ACTIVE_SCREEN, CompositorFlavor, Registration, ShortcutOptions, ShortcutWiring,
};
use flowshot_daemon::state::DaemonState;
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedValue, SerializeDict, Type, Value};
use zbus::{Connection, fdo, object_server::ObjectServer};

// The portal's reply payloads, typed exactly like ashpd's own wire structs
// (SerializeDict = a{sv}).
#[derive(SerializeDict, Type)]
#[zvariant(signature = "dict")]
struct CreateSessionResults {
    session_handle: String,
}

#[derive(SerializeDict, Type)]
#[zvariant(signature = "dict")]
struct BindShortcutsResults {
    shortcuts: Vec<BoundShortcut>,
}

#[derive(serde::Serialize, Type)]
struct BoundShortcut(String, BoundShortcutInfo);

#[derive(SerializeDict, Type)]
#[zvariant(signature = "dict")]
struct BoundShortcutInfo {
    description: String,
    trigger_description: String,
}

/// Serializes every test in this binary: the session-bus env variable and
/// `ashpd`'s process-global connection singleton are shared state.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Session-bus env steering (panic-safe)
// ---------------------------------------------------------------------------

struct SessionBusEnv;

impl SessionBusEnv {
    fn point_at(address: &str) -> Self {
        // SAFETY: ENV_LOCK serializes this binary; no other thread reads
        // the environment between this set and the Drop-restore below
        // (every test acquires the lock first, and the scenario runs on
        // this thread's private runtime).
        unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address) };
        Self
    }
}

impl Drop for SessionBusEnv {
    fn drop(&mut self) {
        // SAFETY: same ENV_LOCK serialization as the setter; Drop runs
        // even when the scenario panics, so the variable never leaks a
        // dead broker address into a later test.
        unsafe { std::env::remove_var("DBUS_SESSION_BUS_ADDRESS") };
    }
}

// ---------------------------------------------------------------------------
// Private broker (the tests/broker.rs recipe, self-contained per
// the throwaway-QA-harness convention)
// ---------------------------------------------------------------------------

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
        let dir = PathBuf::from(format!("/tmp/fsd-sc-{}-{tag}", std::process::id()));
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

// ---------------------------------------------------------------------------
// The stub portal
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct StubHandles {
    binds: Arc<Mutex<Vec<RecordedBind>>>,
    sessions: Arc<Mutex<Vec<String>>>,
    closes: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedBind {
    session: String,
    parent_window: String,
    ids: Vec<String>,
    triggers: Vec<String>,
}

#[derive(Debug, Default)]
struct StubPortal {
    handles: StubHandles,
}

const REQUEST_PREFIX: &str = "/org/freedesktop/portal/desktop/request";
const SESSION_PREFIX: &str = "/org/freedesktop/portal/desktop/session";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";
const SHORTCUTS_IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

/// The portal's `{SENDER}` path element: the caller's unique name without
/// the leading colon, dots underscored (ashpd `Proxy::unique_name`).
fn sender_element(header: &Header<'_>) -> fdo::Result<String> {
    let sender = header
        .sender()
        .ok_or_else(|| fdo::Error::Failed("method call without sender".to_owned()))?;
    Ok(sender.as_str().trim_start_matches(':').replace('.', "_"))
}

fn string_entry(dict: &HashMap<String, OwnedValue>, key: &str) -> fdo::Result<String> {
    let value = dict
        .get(key)
        .ok_or_else(|| fdo::Error::InvalidArgs(format!("options missing `{key}`")))?;
    match &**value {
        Value::Str(text) => Ok(text.to_string()),
        other => Err(fdo::Error::InvalidArgs(format!(
            "`{key}` is {}, expected a string",
            other.value_signature()
        ))),
    }
}

async fn emit_response<R>(conn: &Connection, request_path: &str, results: &R) -> fdo::Result<()>
where
    R: serde::Serialize + Type,
{
    conn.emit_signal(
        None::<&str>,
        request_path,
        REQUEST_IFACE,
        "Response",
        &(0u32, results),
    )
    .await
    .map_err(|error| fdo::Error::Failed(error.to_string()))
}

#[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
impl StubPortal {
    // The live xdp frontend reports version 1 for this interface
    // (busctl-introspected on the QA machine); portal property names are
    // lowercase, hence the explicit member name.
    #[expect(
        clippy::unused_self,
        reason = "the zbus property macro requires a &self getter signature"
    )]
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn create_session(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        #[zbus(object_server)] server: &ObjectServer,
        options: HashMap<String, OwnedValue>,
    ) -> fdo::Result<ObjectPath<'static>> {
        let sender = sender_element(&header)?;
        let handle_token = string_entry(&options, "handle_token")?;
        let session_token = string_entry(&options, "session_handle_token")?;
        let request_path = format!("{REQUEST_PREFIX}/{sender}/{handle_token}");
        let session_path = format!("{SESSION_PREFIX}/{sender}/{session_token}");

        self.handles
            .sessions
            .lock()
            .unwrap()
            .push(session_path.clone());
        let session_object_path = ObjectPath::try_from(session_path.clone())
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        server
            .at(
                session_object_path,
                StubSession {
                    path: session_path.clone(),
                    handles: self.handles.clone(),
                },
            )
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;

        // Real xdp returns session_handle as a STRING (the ashpd
        // CreateSessionResponse 's'-or-'o' quirk); the stub is faithful.
        let results = CreateSessionResults {
            session_handle: session_path.clone(),
        };
        emit_response(conn, &request_path, &results).await?;
        ObjectPath::try_from(request_path).map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    async fn bind_shortcuts(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        session_handle: ObjectPath<'_>,
        shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
        parent_window: String,
        options: HashMap<String, OwnedValue>,
    ) -> fdo::Result<ObjectPath<'static>> {
        let sender = sender_element(&header)?;
        let handle_token = string_entry(&options, "handle_token")?;
        let known = self
            .handles
            .sessions
            .lock()
            .unwrap()
            .contains(&session_handle.to_string());
        if !known {
            return Err(fdo::Error::InvalidArgs("unknown session handle".to_owned()));
        }

        let mut bound: Vec<BoundShortcut> = Vec::new();
        let mut record = RecordedBind {
            session: session_handle.to_string(),
            parent_window,
            ids: Vec::new(),
            triggers: Vec::new(),
        };
        for (id, info) in &shortcuts {
            let description = string_entry(info, "description")?;
            // XDPH assigns the preferred trigger verbatim when free.
            let trigger = string_entry(info, "preferred_trigger")?;
            bound.push(BoundShortcut(
                id.clone(),
                BoundShortcutInfo {
                    description,
                    trigger_description: trigger.clone(),
                },
            ));
            record.ids.push(id.clone());
            record.triggers.push(trigger);
        }
        self.handles.binds.lock().unwrap().push(record);

        let request_path = format!("{REQUEST_PREFIX}/{sender}/{handle_token}");
        let results = BindShortcutsResults { shortcuts: bound };
        emit_response(conn, &request_path, &results).await?;
        ObjectPath::try_from(request_path).map_err(|error| fdo::Error::Failed(error.to_string()))
    }
}

#[derive(Debug, Clone)]
struct StubSession {
    path: String,
    handles: StubHandles,
}

#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl StubSession {
    fn close(&self) {
        self.handles.closes.lock().unwrap().push(self.path.clone());
    }
}

/// Emits the portal's `Activated` broadcast (body `(o,s,t,a{sv})`) as if
/// the compositor had fired the shortcut.
async fn emit_activated(conn: &Connection, session_path: &str, shortcut_id: &str) {
    let body = (
        ObjectPath::try_from(session_path.to_owned()).unwrap(),
        shortcut_id.to_owned(),
        1_700_000_000_000u64,
        HashMap::<String, OwnedValue>::new(),
    );
    conn.emit_signal(
        None::<&str>,
        PORTAL_PATH,
        SHORTCUTS_IFACE,
        "Activated",
        &body,
    )
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Scenario helpers
// ---------------------------------------------------------------------------

fn temp_dir(tag: &str) -> PathBuf {
    let unique = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("flowshot-sc-{}-{tag}-{unique}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn restore_path(config_dir: &Path) -> PathBuf {
    config_dir.join(flowshot_daemon::paths::SHORTCUTS_RESTORE_FILE_NAME)
}

fn wiring() -> (
    ShortcutWiring,
    RecordingSink,
    RecordingNotifier,
    Arc<DaemonState>,
) {
    let sink = RecordingSink::new();
    let notifier = RecordingNotifier::new();
    let state = Arc::new(DaemonState::new(false, Instant::now()));
    (
        ShortcutWiring {
            sink: Arc::new(sink.clone()),
            state: Arc::clone(&state),
            notifier: Arc::new(notifier.clone()),
        },
        sink,
        notifier,
        state,
    )
}

fn options(config_dir: &Path) -> ShortcutOptions {
    ShortcutOptions {
        enabled: true,
        desktop: Some(DesktopEnv::Hyprland),
        config_dir: Some(config_dir.to_path_buf()),
        registration_timeout: Duration::from_secs(30),
        ..ShortcutOptions::default()
    }
}

async fn wait_for(condition: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---------------------------------------------------------------------------
// The ladder e2e
// ---------------------------------------------------------------------------

#[test]
fn portal_ladder_end_to_end() {
    let _guard = lock();
    let Some(bus) = PrivateBus::spawn("ladder") else {
        eprintln!("SKIP: dbus-daemon not installed - stub-portal e2e not run");
        return;
    };
    let _env = SessionBusEnv::point_at(&bus.address);

    // A private current-thread runtime mirrors the daemon's ownership
    // model (ashpd's zbus-5-tokio tasks live and die with it).
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(scenario(&bus.address));
    drop(runtime);
}

#[expect(
    clippy::too_many_lines,
    reason = "a linear five-phase e2e scenario; splitting would thread the shared stub/wiring context through private helpers without isolating any decision"
)]
async fn scenario(broker_address: &str) {
    // ---- phase A: portal absent -> Hyprland fallback -------------------
    let dir_a = temp_dir("absent");
    let (wiring_a, _sink_a, notifier_a, state_a) = wiring();
    let registration_a = flowshot_daemon::shortcut::start(&options(&dir_a), wiring_a).await;
    let Registration::Fallback(info) = &registration_a else {
        panic!("an absent portal must fall back to compositor binds");
    };
    assert_eq!(info.desktop, DesktopEnv::Hyprland);
    assert_eq!(info.flavor, CompositorFlavor::Hyprland);
    assert!(
        info.help.contains("bind = ,Print,exec,flowshot capture"),
        "fallback help must carry the plan's verbatim hyprland snippet"
    );
    assert!(!state_a.reasons().shortcuts, "fallback needs no residency");
    assert!(!restore_path(&dir_a).exists(), "fallback persists nothing");
    assert!(notifier_a.records().is_empty(), "fallback never nudges");
    drop(registration_a);

    // ---- the stub portal appears on the broker --------------------------
    // serve_at (NOT post-build object_server().at()): zbus 5 dispatches
    // method calls through a lazily-subscribed task; the builder awaits its
    // registration before the socket reader spawns, and the name is only
    // requested afterwards, so no early call can race the subscription.
    let portal = StubPortal::default();
    let handles = portal.handles.clone();
    let stub_conn = zbus::connection::Builder::address(broker_address)
        .unwrap()
        .serve_at(PORTAL_PATH, portal)
        .unwrap()
        .build()
        .await
        .unwrap();
    stub_conn.request_name(PORTAL_NAME).await.unwrap();

    // ---- phase B: registration binds, persists, nudges, pins -----------
    let dir_b = temp_dir("portal");
    let (wiring_b, sink_b, notifier_b, state_b) = wiring();
    let registration_b = flowshot_daemon::shortcut::start(&options(&dir_b), wiring_b).await;
    assert!(
        registration_b.is_portal(),
        "the stub portal must satisfy the registration rung"
    );
    let binds = handles.binds.lock().unwrap().clone();
    assert_eq!(binds.len(), 1, "exactly one BindShortcuts call");
    assert!(
        binds[0].parent_window.is_empty(),
        "the headless daemon passes no parent-window identifier"
    );
    assert_eq!(
        binds[0].ids,
        vec!["capture-region", "capture-full", "capture-active-monitor"]
    );
    assert_eq!(
        binds[0].triggers,
        vec!["Print", "Shift+Print", "Ctrl+Print"]
    );
    assert!(
        state_b.reasons().shortcuts,
        "portal registration is a persistence reason"
    );
    let data = RestoreData::load(&restore_path(&dir_b)).expect("restore data must persist");
    assert!(data.first_registration_notified);
    assert_eq!(data.shortcuts.len(), 3);
    assert_eq!(data.shortcuts[0].id, "capture-region");
    assert_eq!(data.shortcuts[0].trigger_description, "Print");
    assert!(
        notifier_b
            .records()
            .contains(&NotificationRecord::ShortcutsRegistered),
        "the FIRST successful registration nudges autostart"
    );

    // ---- phase C: Activated signals dispatch mapped commands ------------
    let session_path = binds[0].session.clone();
    emit_activated(&stub_conn, &session_path, "capture-full").await;
    wait_for(
        || sink_b.commands().contains(&DaemonCommand::CaptureFull),
        "CaptureFull dispatch",
    )
    .await;
    emit_activated(&stub_conn, &session_path, "capture-active-monitor").await;
    wait_for(
        || {
            sink_b
                .commands()
                .contains(&DaemonCommand::CaptureScreen(ACTIVE_SCREEN))
        },
        "active-monitor dispatch",
    )
    .await;
    let dispatched = sink_b.commands().len();
    emit_activated(&stub_conn, &session_path, "another-apps-shortcut").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        sink_b.commands().len(),
        dispatched,
        "foreign shortcut ids must be ignored"
    );

    // ---- phase D: shutdown closes the session and releases the reason ---
    registration_b.shutdown().await;
    assert!(!state_b.reasons().shortcuts, "shutdown releases the reason");
    assert_eq!(
        handles.closes.lock().unwrap().clone(),
        vec![session_path.clone()],
        "shutdown must Close the portal session"
    );

    // ---- phase E: restart reuses the restore data, nudge stays once -----
    let (wiring_e, _sink_e, notifier_e, state_e) = wiring();
    let registration_e = flowshot_daemon::shortcut::start(&options(&dir_b), wiring_e).await;
    assert!(registration_e.is_portal(), "restart re-registers");
    assert!(
        !notifier_e
            .records()
            .contains(&NotificationRecord::ShortcutsRegistered),
        "the autostart nudge is ONE-TIME across restarts"
    );
    assert!(state_e.reasons().shortcuts);
    assert_eq!(handles.binds.lock().unwrap().len(), 2);
    let reused = RestoreData::load(&restore_path(&dir_b)).expect("restore data survives rewrite");
    assert!(reused.first_registration_notified);
    registration_e.shutdown().await;
    assert!(!state_e.reasons().shortcuts);

    // ---- teardown (zbus explicit-close discipline) ----------------------
    std::fs::remove_dir_all(&dir_a).ok();
    std::fs::remove_dir_all(&dir_b).ok();
    stub_conn.close().await.unwrap();
}

//! Private-bus `org.kde.KWin.ScreenShot2` stub for hermetic unit tests.
//!
//! CONTRACT FIDELITY - pinned to the fetched upstream sources (never
//! memory), matching the citation in [`crate::kwin`]:
//!
//! - `KDE/kwin` `src/plugins/screenshot/screenshotdbusinterface2.cpp` at
//!   <https://github.com/KDE/kwin/blob/4788c6c176bbc4a6e98c46055026ae5734252ea2/src/plugins/screenshot/screenshotdbusinterface2.cpp>
//!   (commit `4788c6c176bbc4a6e98c46055026ae5734252ea2`, master, fetched
//!   2026-09-25, identical to master HEAD at fetch time);
//! - `src/plugins/screenshot/org.kde.KWin.ScreenShot2.xml` at the same
//!   commit (signatures: `CaptureArea` = `iiuua{sv}h -> a{sv}`,
//!   `CaptureActiveScreen`/`CaptureActiveWindow` = `a{sv}h -> a{sv}`,
//!   `Version` is a read-only `u` PROPERTY, value 5);
//! - `QImage::Format` enum values from `qt/qtbase`
//!   `src/gui/image/qimage.h` at
//!   <https://github.com/qt/qtbase/blob/d793ab5031c9555c514ee2ab6b4ae3c06b9e90c5/src/gui/image/qimage.h>
//!   (commit `d793ab5031c9555c514ee2ab6b4ae3c06b9e90c5`, fetched
//!   2026-09-25).
//!
//! Reproduced `KWin` behavior (`ScreenShotSinkPipe2::flush` +
//! `ScreenShotWriter2::run`): the metadata vardict reply
//! `{type:"raw", format, width, height, stride, scale}` plus additive keys
//! (`screen` for screen captures, `windowId` for window captures) is sent
//! FIRST; the RAW `QImage` bits (`constBits()`/`sizeInBytes()`, never an
//! encoded image) are then written into the client-supplied pipe fd from a
//! separate thread (`QThreadPool` equivalent) that blocks under
//! backpressure, and closing the fd signals end-of-file. Write failures
//! (the client went away) are ignored, mirroring `KWin`'s "pipe is broken"
//! path. Failure injection: `StubFixture::truncate` withholds tail bytes
//! (writer-abort), `StubFixture::format` can carry an unmappable
//! `QImage::Format`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use zbus::zvariant::{OwnedFd, OwnedValue, Str, Value};

use super::{PATH, SERVICE};

/// What the stub serves for one capture call.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StubFixture {
    /// The raw `QImage` bytes the writer thread delivers.
    pub payload: Vec<u8>,
    /// The `QImage::Format` enum value announced in the metadata.
    pub format: u32,
    /// Announced width in pixels.
    pub width: u32,
    /// Announced height in pixels.
    pub height: u32,
    /// Announced bytes per row.
    pub stride: u32,
    /// Announced device pixel ratio.
    pub scale: f64,
    /// Bytes withheld from the tail of `payload` (writer-abort injection).
    pub truncate: usize,
}

impl StubFixture {
    /// A `Format_ARGB32` (5) fixture with a distinct byte pattern per
    /// pixel, tight stride.
    pub(crate) fn argb32(width: u32, height: u32) -> Self {
        let mut payload = Vec::new();
        for index in 0..width.saturating_mul(height) {
            let byte = u8::try_from(index % 251).unwrap_or(250);
            payload.extend_from_slice(&[byte, byte.wrapping_add(1), byte.wrapping_add(2), 255]);
        }
        Self {
            payload,
            format: 5,
            width,
            height,
            stride: width.saturating_mul(4),
            scale: 1.0,
            truncate: 0,
        }
    }
}

/// One method call the stub served.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ServedCall {
    /// The `D-Bus` member name.
    pub member: &'static str,
    /// The `include-cursor` option value, when present.
    pub include_cursor: Option<bool>,
    /// The `native-resolution` option value, when present.
    pub native_resolution: Option<bool>,
    /// The `CaptureArea` rectangle, for area calls.
    pub area: Option<(i32, i32, u32, u32)>,
}

/// Shared stub state: the fixture, served-call log, and writer threads.
#[derive(Debug)]
pub(crate) struct StubState {
    /// The fixture this stub serves.
    pub fixture: StubFixture,
    /// Whether `GetNameOwner` claims the service name.
    pub serving_name: bool,
    served: Mutex<Vec<ServedCall>>,
    writers: Mutex<Vec<JoinHandle<()>>>,
}

impl StubState {
    fn new(fixture: StubFixture, serving_name: bool) -> Self {
        Self {
            fixture,
            serving_name,
            served: Mutex::new(Vec::new()),
            writers: Mutex::new(Vec::new()),
        }
    }

    /// The calls served so far.
    pub(crate) fn served(&self) -> Vec<ServedCall> {
        lock(&self.served).clone()
    }

    /// Joins every writer thread (each closes its pipe fd on completion):
    /// call before fd-leak assertions.
    pub(crate) fn join_writers(&self) {
        for handle in lock(&self.writers).drain(..) {
            let _joined = handle.join();
        }
    }

    fn record(&self, call: ServedCall) {
        lock(&self.served).push(call);
    }

    fn spawn_writer(self: &Arc<Self>, pipe: OwnedFd) {
        let fixture = self.fixture.clone();
        let handle = std::thread::spawn(move || {
            let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(pipe));
            let end = fixture.payload.len().saturating_sub(fixture.truncate);
            file.write_all(&fixture.payload[..end]).ok();
        });
        lock(&self.writers).push(handle);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The `org.kde.KWin.ScreenShot2` interface stub.
pub(crate) struct StubScreenShot2 {
    state: Arc<StubState>,
}

#[zbus::interface(name = "org.kde.KWin.ScreenShot2")]
impl StubScreenShot2 {
    #[zbus(property)]
    #[expect(
        clippy::unused_self,
        reason = "the zbus property getter signature requires &self"
    )]
    fn version(&self) -> u32 {
        5
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    fn capture_active_screen(
        &self,
        options: HashMap<String, OwnedValue>,
        pipe: OwnedFd,
    ) -> HashMap<String, Value<'static>> {
        self.serve("CaptureActiveScreen", &options, None, pipe)
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    fn capture_active_window(
        &self,
        options: HashMap<String, OwnedValue>,
        pipe: OwnedFd,
    ) -> HashMap<String, Value<'static>> {
        self.serve("CaptureActiveWindow", &options, None, pipe)
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    fn capture_area(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        options: HashMap<String, OwnedValue>,
        pipe: OwnedFd,
    ) -> HashMap<String, Value<'static>> {
        self.serve("CaptureArea", &options, Some((x, y, width, height)), pipe)
    }
}

impl StubScreenShot2 {
    fn new(state: Arc<StubState>) -> Self {
        Self { state }
    }

    fn serve(
        &self,
        member: &'static str,
        options: &HashMap<String, OwnedValue>,
        area: Option<(i32, i32, u32, u32)>,
        pipe: OwnedFd,
    ) -> HashMap<String, Value<'static>> {
        self.state.record(ServedCall {
            member,
            include_cursor: bool_option(options, "include-cursor"),
            native_resolution: bool_option(options, "native-resolution"),
            area,
        });
        let results = self.results(member);
        self.state.spawn_writer(pipe);
        results
    }

    fn results(&self, member: &str) -> HashMap<String, Value<'static>> {
        let fixture = &self.state.fixture;
        let mut results = HashMap::new();
        results.insert("type".to_owned(), Value::Str(Str::from("raw".to_owned())));
        results.insert("format".to_owned(), Value::U32(fixture.format));
        results.insert("width".to_owned(), Value::U32(fixture.width));
        results.insert("height".to_owned(), Value::U32(fixture.height));
        results.insert("stride".to_owned(), Value::U32(fixture.stride));
        results.insert("scale".to_owned(), Value::F64(fixture.scale));
        for (key, value) in additive_keys(member) {
            results.insert(key.to_owned(), value);
        }
        results
    }
}

/// The additive metadata keys the real interface MAY carry (`screen` from
/// screen captures, `windowId` from window captures) plus one key from "the
/// future" - the parser must ignore all of them.
fn additive_keys(member: &str) -> Vec<(&'static str, Value<'static>)> {
    let mut keys = vec![("future-key", Value::U64(42))];
    match member {
        "CaptureActiveScreen" => keys.push(("screen", Value::Str(Str::from("STUB-1".to_owned())))),
        "CaptureActiveWindow" => keys.push((
            "windowId",
            Value::Str(Str::from("11111111-2222-3333-4444-555555555555".to_owned())),
        )),
        _ => {}
    }
    keys
}

fn bool_option(options: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    match &**options.get(key)? {
        Value::Bool(value) => Some(*value),
        _ => None,
    }
}

/// The `org.freedesktop.DBus` bus-driver stub (only `GetNameOwner`, the
/// probe's name-owner check).
pub(crate) struct StubBusDriver {
    state: Arc<StubState>,
}

#[zbus::interface(name = "org.freedesktop.DBus")]
impl StubBusDriver {
    fn get_name_owner(&self, name: String) -> Result<String, zbus::fdo::Error> {
        if self.state.serving_name && name == SERVICE {
            Ok(name)
        } else {
            Err(zbus::fdo::Error::NameHasNoOwner(name))
        }
    }
}

/// A live private-bus stub peer: hold it for the test's lifetime.
pub(crate) struct StubBus {
    client: Option<zbus::Connection>,
    /// The shared stub state (served calls, writer joins).
    pub state: Arc<StubState>,
    _server: zbus::Connection,
}

impl StubBus {
    /// Takes the built client connection (once) to hand to the code under
    /// test. Building it here (concurrently with the server side) is what
    /// makes the p2p SASL handshake complete - a server built alone would
    /// wait forever for a client AUTH.
    pub(crate) fn client_conn(&mut self) -> zbus::Connection {
        self.client.take().unwrap()
    }
}

/// Spawns the stub peer over a `UnixStream` pair: both p2p connections are
/// built concurrently (the handshake needs both sides), then the server
/// registers both well-known names (p2p name registration is local
/// self-identification) and serves the `ScreenShot2` object unless
/// `register_screenshot` is false (introspection-failure injection). The
/// server's dispatch tasks live on the `zbus` executor pool, so merely
/// holding the connection keeps the stub alive.
pub(crate) fn spawn_stub(
    fixture: StubFixture,
    serving_name: bool,
    register_screenshot: bool,
) -> StubBus {
    let (server_socket, client_socket) = UnixStream::pair().unwrap();
    let state = Arc::new(StubState::new(fixture, serving_name));
    let (server, client) = futures::executor::block_on(async {
        futures::future::try_join(
            build_server(server_socket, &state, register_screenshot),
            zbus::ConnectionBuilder::unix_stream(client_socket)
                .p2p()
                .build(),
        )
        .await
        .unwrap()
    });
    StubBus {
        client: Some(client),
        state,
        _server: server,
    }
}

async fn build_server(
    socket: UnixStream,
    state: &Arc<StubState>,
    register_screenshot: bool,
) -> zbus::Result<zbus::Connection> {
    let guid = zbus::Guid::generate();
    let connection = zbus::ConnectionBuilder::unix_stream(socket)
        .server(guid)?
        .p2p()
        .build()
        .await?;
    connection.request_name(SERVICE).await?;
    connection.request_name("org.freedesktop.DBus").await?;
    connection
        .object_server()
        .at(
            "/org/freedesktop/DBus",
            StubBusDriver {
                state: Arc::clone(state),
            },
        )
        .await?;
    if register_screenshot {
        connection
            .object_server()
            .at(PATH, StubScreenShot2::new(Arc::clone(state)))
            .await?;
    }
    Ok(connection)
}

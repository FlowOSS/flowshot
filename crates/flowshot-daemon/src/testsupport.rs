//! Private-bus stub harnesses (todo-11 recipe): both p2p sides are built
//! CONCURRENTLY - a server built alone blocks forever waiting for the
//! client's SASL handshake. Teardown closes both connections explicitly:
//! zbus-4-async-io has NO drop-time close (todo-11 regression lesson).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use zbus::Connection;

use crate::bus::{FlowShotInterface, OBJECT_PATH, SERVICE};
use crate::command::CommandSink;
use crate::lifecycle::TokioClock;
use crate::state::DaemonState;

/// Serializes all stub-based tests (todo-11 `STUB_LOCK` precedent): their
/// p2p sockets, dedicated zbus driver threads, and the process-global
/// async-io reactor are shared process-wide resources, and one observed
/// 60 s stall under heavy external CPU contention motivated bounding +
/// serialization instead of relying on scheduling luck.
pub(crate) static STUB_LOCK: Mutex<()> = Mutex::new(());

/// Acquires the stub serialization guard (poison-tolerant).
pub(crate) fn stub_guard() -> MutexGuard<'static, ()> {
    STUB_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Every test-side wire await is bounded: a stall becomes a loud named
/// failure instead of hanging the suite.
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(15);

/// Awaits `future` under [`CALL_TIMEOUT`], panicking with `what` on expiry.
pub(crate) async fn bounded<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(CALL_TIMEOUT, future)
        .await
        .unwrap_or_else(|_| panic!("{what} did not complete within {CALL_TIMEOUT:?}"))
}

/// A live p2p stub pair serving the REAL [`FlowShotInterface`]: the server
/// side holds the name (local self-identification) and dispatches into the
/// injected sink; the client side is what code under test calls with.
pub(crate) struct ServiceStub {
    client: Connection,
    server: Connection,
    state: Arc<DaemonState>,
}

/// Spawns the service stub over a `UnixStream` pair.
pub(crate) async fn spawn_service_stub(sink: Arc<dyn CommandSink>) -> ServiceStub {
    let (server_socket, client_socket) = UnixStream::pair().unwrap();
    let state = Arc::new(DaemonState::new(false, Instant::now()));
    let (server, client) = tokio::try_join!(
        build_server(server_socket, sink, Arc::clone(&state)),
        build_client(client_socket),
    )
    .unwrap();
    ServiceStub {
        client,
        server,
        state,
    }
}

async fn build_server(
    socket: UnixStream,
    sink: Arc<dyn CommandSink>,
    state: Arc<DaemonState>,
) -> zbus::Result<Connection> {
    let guid = zbus::Guid::generate();
    let connection = zbus::ConnectionBuilder::unix_stream(socket)
        .server(guid)?
        .p2p()
        .build()
        .await?;
    connection.request_name(SERVICE).await?;
    let interface = FlowShotInterface::new(sink, state, Arc::new(TokioClock));
    connection
        .object_server()
        .at(OBJECT_PATH, interface)
        .await?;
    Ok(connection)
}

async fn build_client(socket: UnixStream) -> zbus::Result<Connection> {
    zbus::ConnectionBuilder::unix_stream(socket)
        .p2p()
        .build()
        .await
}

impl ServiceStub {
    /// The client end (code under test talks through this).
    pub(crate) const fn client(&self) -> &Connection {
        &self.client
    }

    /// The state the served interface touches.
    pub(crate) const fn state(&self) -> &Arc<DaemonState> {
        &self.state
    }

    /// Deterministic teardown: explicit close on BOTH ends (todo-11:
    /// dropping a zbus-4-async-io connection leaks the socket fd with the
    /// reader task parked on the global pool).
    pub(crate) async fn shutdown(self) {
        self.client.close().await.unwrap();
        self.server.close().await.unwrap();
    }
}

/// A stub that holds the well-known name but serves NO interface: the
/// "foreign holder" of the plan's failure QA scenario.
pub(crate) struct ForeignStub {
    client: Connection,
    server: Connection,
}

/// Spawns the foreign-holder stub.
pub(crate) async fn spawn_foreign_stub() -> ForeignStub {
    let (server_socket, client_socket) = UnixStream::pair().unwrap();
    let (server, client) = tokio::try_join!(
        async {
            let guid = zbus::Guid::generate();
            let connection = zbus::ConnectionBuilder::unix_stream(server_socket)
                .server(guid)?
                .p2p()
                .build()
                .await?;
            connection.request_name(SERVICE).await?;
            Ok::<Connection, zbus::Error>(connection)
        },
        build_client(client_socket),
    )
    .unwrap();
    ForeignStub { client, server }
}

impl ForeignStub {
    /// The client end.
    pub(crate) const fn client(&self) -> &Connection {
        &self.client
    }

    /// Deterministic teardown (both ends closed explicitly).
    pub(crate) async fn shutdown(self) {
        self.client.close().await.unwrap();
        self.server.close().await.unwrap();
    }
}

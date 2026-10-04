//! The `org.flowoss.FlowShot` session-bus interface.
//!
//! Naming (Metis #10): org namespace = `FlowOSS`,
//! product = `FlowShot`. This service must NEVER claim `org.flameshot.*`
//! or `org.flowshot.*` - coexistence with a parallel `Flameshot` install
//! is a hard requirement.
//!
//! Wire contract (own API; the CLI<->`D-Bus` mapping table is recorded in
//! the docs):
//!
//! | Member          | Signature | Purpose                              |
//! |-----------------|-----------|--------------------------------------|
//! | `Capture`       | `a{sv}`   | region capture with modifiers        |
//! | `CaptureFull`   | (none)    | full-desktop capture                 |
//! | `CaptureScreen` | `u`       | single output by index               |
//! | `Launcher`      | (none)    | manual-coordinate dialog             |
//! | `Settings`      | (none)    | settings surface                     |
//! | `Invoke`        | `as`      | second-instance argv forwarding      |

use std::collections::HashMap;
use std::sync::Arc;

use zbus::fdo;
use zbus::zvariant::OwnedValue;

use crate::command::{CommandSink, DaemonCommand};
use crate::lifecycle::Clock;
use crate::request::CaptureRequest;
use crate::state::DaemonState;

/// The well-known bus name (single-instance token: acquisition is atomic
/// on the bus, so exactly one process ever holds it).
pub const SERVICE: &str = "org.flowoss.FlowShot";

/// The single object path every method lives at.
pub const OBJECT_PATH: &str = "/org/flowoss/FlowShot";

/// The interface name (methods are exposed on the same name).
pub const IFACE: &str = "org.flowoss.FlowShot";

/// The object-server implementation: parses at the boundary, touches the
/// activity stamp, and hands typed commands to the sink.
#[derive(Debug)]
pub struct FlowShotInterface {
    sink: Arc<dyn CommandSink>,
    state: Arc<DaemonState>,
    clock: Arc<dyn Clock>,
}

impl FlowShotInterface {
    /// An interface dispatching into `sink`, keeping `state`'s activity
    /// stamp fresh from `clock`.
    #[must_use]
    pub fn new(sink: Arc<dyn CommandSink>, state: Arc<DaemonState>, clock: Arc<dyn Clock>) -> Self {
        Self { sink, state, clock }
    }

    /// The bus reply contract (the silent-failure fix): the reply waits
    /// for the sink's execution receipt - an early failure becomes a typed
    /// `Failed` error reply (the forwarding CLI exits non-zero with the
    /// message), while a receipt resolving `Ok` (success, or the startup
    /// reply window elapsed with a window session still running) replies
    /// acceptance. Untracked sinks reply immediately.
    async fn accept(&self, command: DaemonCommand) -> fdo::Result<()> {
        self.state.touch(self.clock.now());
        let Some(receipt) = self.sink.dispatch_tracked(command) else {
            return Ok(());
        };
        match receipt.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(fdo::Error::Failed(message)),
            Err(_dropped) => Err(fdo::Error::Failed(
                crate::strings::EXECUTOR_THREAD_DIED.to_owned(),
            )),
        }
    }
}

#[zbus::interface(name = "org.flowoss.FlowShot")]
impl FlowShotInterface {
    /// Region capture with the CLI modifier bag (`a{sv}`; keys
    /// per [`crate::request::CAPTURE_OPTION_KEYS`]).
    async fn capture(&self, options: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        let request = CaptureRequest::from_vardict(&options)
            .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))?;
        self.accept(DaemonCommand::Capture(request)).await
    }

    /// Full-desktop capture.
    async fn capture_full(&self) -> fdo::Result<()> {
        self.accept(DaemonCommand::CaptureFull).await
    }

    /// Single-output capture by index.
    async fn capture_screen(&self, screen: u32) -> fdo::Result<()> {
        self.accept(DaemonCommand::CaptureScreen(screen)).await
    }

    /// Open the capture launcher dialog.
    async fn launcher(&self) -> fdo::Result<()> {
        self.accept(DaemonCommand::Launcher).await
    }

    /// Open the settings surface.
    async fn settings(&self) -> fdo::Result<()> {
        self.accept(DaemonCommand::Settings).await
    }

    /// Second-instance argv forwarding (single-instance parity UX: the
    /// losing process exits 0 after this call, non-zero when the daemon's
    /// execution failed early - the silent-failure fix).
    async fn invoke(&self, argv: Vec<String>) -> fdo::Result<()> {
        tracing::info!(argv = ?argv, "invoke received");
        self.accept(DaemonCommand::Invoke(argv)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{ExecutionOutcome, ExecutionReceipt, RecordingSink};
    use crate::testsupport::{ServiceStub, bounded, spawn_service_stub, stub_guard};
    use std::time::Instant;
    use zbus::zvariant::{Str, Value};

    /// A sink whose tracked receipt resolves to a canned outcome (the
    /// bus-side half of the silent-failure fix: receipt -> reply mapping).
    #[derive(Debug)]
    struct ReceiptSink(ReceiptMode);

    #[derive(Debug)]
    enum ReceiptMode {
        Accepted,
        Failed(&'static str),
        ThreadDied,
    }

    impl CommandSink for ReceiptSink {
        fn dispatch(&self, _command: DaemonCommand) {}

        fn dispatch_tracked(&self, _command: DaemonCommand) -> Option<ExecutionReceipt> {
            let (reply, receipt) = tokio::sync::oneshot::channel::<ExecutionOutcome>();
            match &self.0 {
                ReceiptMode::Accepted => {
                    let _ = reply.send(Ok(()));
                }
                ReceiptMode::Failed(message) => {
                    let _ = reply.send(Err((*message).to_owned()));
                }
                ReceiptMode::ThreadDied => {}
            }
            Some(receipt)
        }
    }

    /// The `Failed` reply assertion shared by the receipt-error tests.
    async fn assert_failed_reply(client: &zbus::Connection, expected_detail: &str) {
        let error = bounded(
            "CaptureFull (failing receipt)",
            client.call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "CaptureFull", &()),
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("a failing receipt must produce an error reply"));
        match &error {
            zbus::Error::MethodError(name, detail, _) => {
                assert_eq!(name.as_str(), "org.freedesktop.DBus.Error.Failed");
                let detail = detail.as_deref().unwrap_or_default();
                assert!(
                    detail.contains(expected_detail),
                    "reply detail {detail:?} must carry {expected_detail:?}"
                );
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn early_execution_failure_replies_typed_failed() {
        // Given: a sink whose execution fails inside the reply window.
        let _guard = stub_guard();
        let sink = ReceiptSink(ReceiptMode::Failed("session child failed (exit 1): boom"));
        let stub = spawn_service_stub(Arc::new(sink)).await;
        // When/Then: the bus call answers with the typed Failed error
        // carrying the executor's message (the CLI maps it onto a non-zero
        // exit + stderr text).
        assert_failed_reply(stub.client(), "session child failed").await;
        stub.shutdown().await;
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn dropped_receipt_replies_executor_thread_died() {
        // Given: a sink whose executor thread died before reporting.
        let _guard = stub_guard();
        let stub = spawn_service_stub(Arc::new(ReceiptSink(ReceiptMode::ThreadDied))).await;
        // When/Then: the dead receipt surfaces as a typed failure, not Ok.
        assert_failed_reply(stub.client(), crate::strings::EXECUTOR_THREAD_DIED).await;
        stub.shutdown().await;
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn resolved_ok_receipt_replies_acceptance() {
        // Given: a sink whose execution reported success.
        let _guard = stub_guard();
        let stub = spawn_service_stub(Arc::new(ReceiptSink(ReceiptMode::Accepted))).await;
        // When/Then: the call replies Ok.
        call_ok(stub.client(), "CaptureFull", &()).await;
        stub.shutdown().await;
    }

    async fn recording_stub() -> (ServiceStub, RecordingSink) {
        let sink = RecordingSink::new();
        let stub = spawn_service_stub(Arc::new(sink.clone())).await;
        (stub, sink)
    }

    /// One bounded wire call that must succeed.
    async fn call_ok<B>(client: &zbus::Connection, method: &str, body: &B)
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::Type,
    {
        bounded(
            method,
            client.call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), method, body),
        )
        .await
        .unwrap_or_else(|error| panic!("{method}: {error}"));
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn every_method_lands_on_the_sink_with_typed_payload() {
        let _guard = stub_guard();
        let (stub, sink) = recording_stub().await;
        let client = stub.client();
        let options: HashMap<String, Value<'_>> = HashMap::from([
            ("copy".to_owned(), Value::Bool(true)),
            ("delay_ms".to_owned(), Value::U32(120)),
            ("region".to_owned(), Value::Str(Str::from("at-cursor"))),
        ]);
        call_ok(client, "Capture", &options).await;
        call_ok(client, "CaptureFull", &()).await;
        call_ok(client, "CaptureScreen", &2u32).await;
        call_ok(client, "Launcher", &()).await;
        call_ok(client, "Settings", &()).await;
        let argv = vec!["capture".to_owned(), "--full".to_owned()];
        call_ok(client, "Invoke", &argv).await;
        stub.shutdown().await;
        assert_eq!(
            sink.commands(),
            vec![
                DaemonCommand::Capture(CaptureRequest {
                    copy: true,
                    delay_ms: 120,
                    region: Some("at-cursor".to_owned()),
                    ..CaptureRequest::default()
                }),
                DaemonCommand::CaptureFull,
                DaemonCommand::CaptureScreen(2),
                DaemonCommand::Launcher,
                DaemonCommand::Settings,
                DaemonCommand::Invoke(vec!["capture".to_owned(), "--full".to_owned()]),
            ]
        );
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn wrong_typed_option_replies_invalid_args() {
        let _guard = stub_guard();
        let (stub, sink) = recording_stub().await;
        let options: HashMap<String, Value<'_>> =
            HashMap::from([("copy".to_owned(), Value::Str(Str::from("yes")))]);
        let error = bounded(
            "Capture (invalid args)",
            stub.client()
                .call_method(Some(SERVICE), OBJECT_PATH, Some(IFACE), "Capture", &options),
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("a stringly bool must be rejected"));
        assert!(
            matches!(&error, zbus::Error::MethodError(name, _, _)
                if name.as_str() == "org.freedesktop.DBus.Error.InvalidArgs"),
            "unexpected error: {error}"
        );
        stub.shutdown().await;
        assert!(sink.commands().is_empty());
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn accepted_calls_touch_the_activity_stamp() {
        let _guard = stub_guard();
        let (stub, _sink) = recording_stub().await;
        let state = Arc::clone(stub.state());
        let before = state.idle_for(Instant::now());
        call_ok(stub.client(), "Launcher", &()).await;
        let after = state.idle_for(Instant::now());
        assert!(after <= before);
        stub.shutdown().await;
    }
}

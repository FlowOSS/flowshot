//! The `org.flowoss.FlowShot` session-bus interface.
//!
//! Naming (User Amendment #2, Metis #10): org namespace = `FlowOSS`,
//! product = `FlowShot`. This service must NEVER claim `org.flameshot.*`
//! or `org.flowshot.*` - coexistence with a parallel `Flameshot` install
//! is a hard requirement.
//!
//! Wire contract (own API; the CLI<->`D-Bus` mapping table is recorded in
//! todo 40 docs):
//!
//! | Member          | Signature | Purpose                              |
//! |-----------------|-----------|--------------------------------------|
//! | `Capture`       | `a{sv}`   | region capture with modifiers        |
//! | `CaptureFull`   | (none)    | full-desktop capture                 |
//! | `CaptureScreen` | `u`       | single output by index               |
//! | `Launcher`      | (none)    | manual-coordinate dialog (todo 37)   |
//! | `Settings`      | (none)    | settings surface (todo 36)           |
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

    fn accept(&self, command: DaemonCommand) {
        self.state.touch(self.clock.now());
        self.sink.dispatch(command);
    }
}

#[zbus::interface(name = "org.flowoss.FlowShot")]
impl FlowShotInterface {
    /// Region capture with the Amendment #2 modifier bag (`a{sv}`; keys
    /// per [`crate::request::CAPTURE_OPTION_KEYS`]).
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the zbus interface macro deserializes owned message arguments"
    )]
    fn capture(&self, options: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        let request = CaptureRequest::from_vardict(&options)
            .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))?;
        self.accept(DaemonCommand::Capture(request));
        Ok(())
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    /// Full-desktop capture.
    fn capture_full(&self) -> fdo::Result<()> {
        self.accept(DaemonCommand::CaptureFull);
        Ok(())
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    /// Single-output capture by index.
    fn capture_screen(&self, screen: u32) -> fdo::Result<()> {
        self.accept(DaemonCommand::CaptureScreen(screen));
        Ok(())
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    /// Open the capture launcher dialog (todo 37 surface).
    fn launcher(&self) -> fdo::Result<()> {
        self.accept(DaemonCommand::Launcher);
        Ok(())
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    /// Open the settings surface (todo 36).
    fn settings(&self) -> fdo::Result<()> {
        self.accept(DaemonCommand::Settings);
        Ok(())
    }

    /// Second-instance argv forwarding (single-instance parity UX: the
    /// losing process exits 0 after this call).
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the zbus wire contract keeps fdo::Result for error replies; this member never rejects"
    )]
    fn invoke(&self, argv: Vec<String>) -> fdo::Result<()> {
        tracing::info!(argv = ?argv, "invoke received");
        self.accept(DaemonCommand::Invoke(argv));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::RecordingSink;
    use crate::testsupport::{ServiceStub, bounded, spawn_service_stub, stub_guard};
    use std::time::Instant;
    use zbus::zvariant::{Str, Value};

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
        reason = "intentional cross-test serialization (todo-11 STUB_LOCK); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
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
        reason = "intentional cross-test serialization (todo-11 STUB_LOCK); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
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
        reason = "intentional cross-test serialization (todo-11 STUB_LOCK); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
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

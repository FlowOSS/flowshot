//! Single-instance handshake: atomic bus-name acquisition + argv
//! forwarding (Oracle r4 race-free-by-construction).
//!
//! The well-known name is the single-instance token: `RequestName` with
//! `DoNotQueue` is ATOMIC on the bus, so exactly one process ever receives
//! `PrimaryOwner`. Every loser forwards its command line through the
//! winner's `Invoke(argv)` method and exits 0 (parity UX). The same
//! primitives compose the CLI dispatch: winner becomes/spawns the
//! daemon, then self-invokes.

use std::time::Duration;

use zbus::Connection;
use zbus::connection::Builder;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

use crate::bus::{IFACE, OBJECT_PATH};
use crate::error::DaemonError;

/// Budget for the loser's `Invoke` round-trip (the winner only queues the
/// command, so this bounds bus latency, not capture time).
const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of an ownership attempt on the well-known name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// This connection is now the primary owner (it must serve).
    Acquired,
    /// Another process holds the name (this one must forward or defer).
    HeldByOther,
}

/// The pure reply mapping (unit-tested without a broker).
#[must_use]
pub const fn ownership_from_reply(reply: &RequestNameReply) -> Ownership {
    match reply {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ownership::Acquired,
        // InQueue cannot occur with DoNotQueue; forwarding to the current
        // owner is correct regardless.
        RequestNameReply::Exists | RequestNameReply::InQueue => Ownership::HeldByOther,
    }
}

/// Outcome of the full client-side handshake.
#[derive(Debug)]
pub enum Acquisition {
    /// No daemon was running: the caller owns the name (and per the CLI
    /// dispatch becomes/spawns the daemon, then self-invokes). Carries the
    /// name-holding connection.
    Owner(Connection),
    /// A running daemon received the forwarded argv; the caller exits 0.
    Forwarded,
}

/// Connects to the session bus, or to `bus_address` when given (private
/// test buses; `unix:path=...` form).
///
/// # Errors
///
/// [`DaemonError::Dbus`] when the address is malformed or the handshake
/// fails.
pub async fn connect(bus_address: Option<&str>) -> Result<Connection, DaemonError> {
    let connection = match bus_address {
        Some(address) => Builder::address(address)?.build().await?,
        None => Connection::session().await?,
    };
    Ok(connection)
}

/// Requests exclusive ownership of `service` (atomic on a brokered bus;
/// local self-identification on p2p).
///
/// # Errors
///
/// [`DaemonError::Dbus`] when the broker round-trip fails.
pub async fn request_ownership(
    connection: &Connection,
    service: &str,
) -> Result<Ownership, DaemonError> {
    let reply = connection
        .request_name_with_flags(service, RequestNameFlags::DoNotQueue.into())
        .await;
    match reply {
        Ok(reply) => Ok(ownership_from_reply(&reply)),
        // request_name_with_flags does not map replies to errors, but a
        // broker dialect that does still means: someone else holds it.
        Err(zbus::Error::NameTaken) => Ok(Ownership::HeldByOther),
        Err(error) => Err(DaemonError::Dbus(error)),
    }
}

/// Forwards `argv` to the running owner's `Invoke` method under a
/// 5-second budget.
///
/// # Errors
///
/// - [`DaemonError::ForeignNameHolder`] when the name is held by a process
///   that does not answer `Invoke` (failure QA: typed error +
///   hint) - whether it replies with an unknown-object error or stays
///   silent until the timeout;
/// - [`DaemonError::OwnerVanished`] when the owner exited mid-handshake;
/// - [`DaemonError::Dbus`] for transport failures.
pub async fn forward_argv(
    connection: &Connection,
    service: &str,
    argv: &[String],
) -> Result<(), DaemonError> {
    forward_argv_within(connection, service, argv, FORWARD_TIMEOUT).await
}

async fn forward_argv_within(
    connection: &Connection,
    service: &str,
    argv: &[String],
    timeout: Duration,
) -> Result<(), DaemonError> {
    let call = connection.call_method(Some(service), OBJECT_PATH, Some(IFACE), "Invoke", &argv);
    match tokio::time::timeout(timeout, call).await {
        Ok(Ok(_reply)) => Ok(()),
        Ok(Err(zbus::Error::MethodError(name, detail, _reply))) => {
            let detail = detail.unwrap_or_default();
            classify_method_error(service, name.as_str(), &detail)
        }
        Ok(Err(error)) => Err(DaemonError::Dbus(error)),
        // A functioning owner only queues the command, so silence for the
        // whole budget means the holder is not serving Invoke.
        Err(_elapsed) => Err(DaemonError::ForeignNameHolder {
            service: service.to_owned(),
            detail: format!("no reply within {}s", timeout.as_secs()),
        }),
    }
}

fn classify_method_error(service: &str, name: &str, detail: &str) -> Result<(), DaemonError> {
    const UNKNOWN_PREFIX: &str = "org.freedesktop.DBus.Error.Unknown";
    if name.starts_with(UNKNOWN_PREFIX) || name == "org.freedesktop.DBus.Error.Failed" {
        return Err(DaemonError::ForeignNameHolder {
            service: service.to_owned(),
            detail: format!("{name}: {detail}"),
        });
    }
    if name == "org.freedesktop.DBus.Error.NameHasNoOwner" {
        return Err(DaemonError::OwnerVanished);
    }
    Err(DaemonError::ForeignNameHolder {
        service: service.to_owned(),
        detail: format!("{name}: {detail}"),
    })
}

/// The full client-side handshake: acquire the name or forward `argv` to
/// whoever holds it. This is the composition the CLI dispatch calls.
///
/// # Errors
///
/// Any [`forward_argv`] / [`connect`] / [`request_ownership`] error.
pub async fn acquire_or_forward(
    bus_address: Option<&str>,
    service: &str,
    argv: &[String],
) -> Result<Acquisition, DaemonError> {
    let connection = connect(bus_address).await?;
    match request_ownership(&connection, service).await? {
        Ownership::Acquired => Ok(Acquisition::Owner(connection)),
        Ownership::HeldByOther => {
            let forwarded = forward_argv(&connection, service, argv).await;
            // zbus's async-io reactor has NO drop-time close (and
            // close() is the deterministic release on the tokio reactor
            // too): the loser connection is closed explicitly on EVERY path.
            close_quietly(connection).await;
            forwarded?;
            Ok(Acquisition::Forwarded)
        }
    }
}

/// Explicit close with diagnostics downgraded to debug (close is teardown;
/// its errors are not actionable).
pub(crate) async fn close_quietly(connection: Connection) {
    if let Err(error) = connection.close().await {
        tracing::debug!(%error, "the bus connection close reported an error");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SERVICE;
    use crate::command::{DaemonCommand, RecordingSink};
    use crate::testsupport::{ServiceStub, spawn_foreign_stub, spawn_service_stub, stub_guard};
    use std::sync::Arc;

    #[test]
    fn reply_mapping_is_exhaustive() {
        assert_eq!(
            ownership_from_reply(&RequestNameReply::PrimaryOwner),
            Ownership::Acquired
        );
        assert_eq!(
            ownership_from_reply(&RequestNameReply::AlreadyOwner),
            Ownership::Acquired
        );
        assert_eq!(
            ownership_from_reply(&RequestNameReply::Exists),
            Ownership::HeldByOther
        );
        assert_eq!(
            ownership_from_reply(&RequestNameReply::InQueue),
            Ownership::HeldByOther
        );
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn forward_argv_delivers_the_exact_argv_to_the_owner() {
        let _guard = stub_guard();
        let sink = RecordingSink::new();
        let stub: ServiceStub = spawn_service_stub(Arc::new(sink.clone())).await;
        let argv = vec![
            "capture".to_owned(),
            "screen".to_owned(),
            "1".to_owned(),
            "--copy".to_owned(),
        ];
        forward_argv(stub.client(), SERVICE, &argv)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(sink.commands(), vec![DaemonCommand::Invoke(argv)]);
        stub.shutdown().await;
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn foreign_name_holder_yields_typed_error_with_hint() {
        let _guard = stub_guard();
        let stub = spawn_foreign_stub().await;
        // The foreign peer never answers (a zbus peer without an object
        // server silently drops calls): the short budget keeps the test
        // fast while the classification stays the production one.
        let error = forward_argv_within(
            stub.client(),
            SERVICE,
            &["capture".to_owned()],
            Duration::from_millis(200),
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("a foreign holder must not answer Invoke"));
        let message = error.to_string();
        match error {
            DaemonError::ForeignNameHolder { service, detail } => {
                assert_eq!(service, SERVICE);
                assert!(!detail.is_empty());
                // The hint (Display) names the stale-registration remedy.
                assert!(message.contains("stale"));
            }
            other => panic!("expected ForeignNameHolder, got {other:?}"),
        }
        stub.shutdown().await;
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "intentional cross-test serialization (the STUB_LOCK precedent); each #[tokio::test] is a current-thread runtime, so the guard never crosses a task boundary"
    )]
    async fn request_ownership_on_p2p_is_local_self_identification() {
        // p2p connections have no broker: request_name always reports
        // PrimaryOwner locally - the atomic contention path
        // is covered by the private-broker integration test.
        let _guard = stub_guard();
        let sink = RecordingSink::new();
        let stub = spawn_service_stub(Arc::new(sink)).await;
        assert_eq!(
            request_ownership(stub.client(), SERVICE)
                .await
                .unwrap_or_else(|error| panic!("{error}")),
            Ownership::Acquired
        );
        stub.shutdown().await;
    }
}

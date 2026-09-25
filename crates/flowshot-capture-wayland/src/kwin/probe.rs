//! Availability probing for the `org.kde.KWin.ScreenShot2` fast path.
//!
//! `KWin`'s screenshot service is a session-bus name the Wayland registry
//! cannot see (the same gap the portal probe covers). Availability = the
//! name has an owner AND the object answers an interface introspection
//! (`Properties.Get` for the `Version` property the pinned contract
//! declares). [`KwinAvailability::observe_into`] extends the shared
//! [`CapabilityProbe`] so the negotiation ladder gates the `KWin` rung like
//! the native and portal ones.
//!
//! [`probe_kwin_blocking`] is the sync convenience for probe assembly
//! outside an async context: it runs the check on a worker thread owning a
//! private `tokio` runtime (the portal probe's isolation pattern), so it is
//! safe to call from any thread, including inside another executor.

use flowshot_capture::{BackendKind, CapabilityProbe};
use zbus::Connection;

use super::error::KwinError;
use super::run::KWIN_TIMEOUT;
use super::{IFACE, PATH, SERVICE};

/// The standard `D-Bus` bus-driver name.
const DBUS_BUS_NAME: &str = "org.freedesktop.DBus";
/// The standard `D-Bus` properties interface.
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";

/// Whether the session offers the `KWin` `ScreenShot2` fast path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KwinAvailability {
    /// `org.kde.KWin.ScreenShot2` owns its bus name and answered the
    /// `Version` introspection.
    pub screenshot2: bool,
}

impl KwinAvailability {
    /// Records the available backend into `probe`, extending the
    /// registry-driven capabilities with the `KWin` rung of the ladder.
    pub fn observe_into(&self, probe: &mut CapabilityProbe) {
        if self.screenshot2 {
            probe.observe(BackendKind::KwinScreenShot2);
        }
    }
}

/// Probes the session bus for the `ScreenShot2` service.
///
/// Infallible by design: no bus, no name owner, or a failed introspection
/// all mean "unavailable" (the negotiation ladder degrades, it never fails
/// on a probe). Reactor-agnostic: `zbus`'s `async-io` executor drives the
/// connection from any runtime.
pub async fn probe_kwin() -> KwinAvailability {
    match Connection::session().await {
        Ok(connection) => {
            let availability = probe_on(&connection).await;
            // Deterministic teardown: a dropped zbus handle leaves the socket
            // open with its reader task parked (async-io tasks outlive our
            // runtime); close() shuts the socket down so the descriptor is
            // released without waiting on the bus.
            if let Err(error) = connection.close().await {
                tracing::debug!(%error, "the KWin probe connection close reported an error");
            }
            availability
        }
        Err(error) => {
            tracing::debug!(%error, "no session D-Bus connection; KWin ScreenShot2 unavailable");
            KwinAvailability::default()
        }
    }
}

/// Probes over an established connection (the stub-test seam: a private
/// peer stands in for the session bus).
pub(crate) async fn probe_on(connection: &Connection) -> KwinAvailability {
    let owned = connection
        .call_method(
            Some(DBUS_BUS_NAME),
            "/org/freedesktop/DBus",
            Some(DBUS_BUS_NAME),
            "GetNameOwner",
            &(SERVICE,),
        )
        .await;
    if let Err(error) = owned {
        tracing::debug!(%error, "org.kde.KWin.ScreenShot2 has no bus name owner");
        return KwinAvailability::default();
    }
    let introspected = connection
        .call_method(
            Some(SERVICE),
            PATH,
            Some(PROPERTIES_IFACE),
            "Get",
            &(IFACE, "Version"),
        )
        .await;
    match introspected {
        Ok(reply) => {
            let version = reply
                .body()
                .deserialize::<zbus::zvariant::OwnedValue>()
                .ok()
                .and_then(|value| version_u32(&value));
            tracing::debug!(
                ?version,
                "KWin ScreenShot2 interface answered introspection"
            );
            KwinAvailability { screenshot2: true }
        }
        Err(error) => {
            tracing::debug!(%error, "KWin ScreenShot2 introspection failed; unavailable");
            KwinAvailability::default()
        }
    }
}

/// Extracts the `u32` out of the `Properties.Get` reply variant.
fn version_u32(value: &zbus::zvariant::Value<'_>) -> Option<u32> {
    match value {
        zbus::zvariant::Value::U32(version) => Some(*version),
        zbus::zvariant::Value::Value(nested) => version_u32(nested),
        _ => None,
    }
}

/// [`probe_kwin`] for synchronous callers: runs the probe on a worker thread
/// with a private runtime and blocks the calling thread until it completes
/// (bounded by the 15s budget; any failure degrades to "unavailable").
#[must_use]
pub fn probe_kwin_blocking() -> KwinAvailability {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("flowshot-kwin-probe".to_owned())
        .spawn(move || {
            let availability = probe_on_worker();
            if sender.send(availability).is_err() {
                tracing::debug!("KWin probe result discarded: the requester went away");
            }
        });
    match spawned {
        Ok(_worker) => futures::executor::block_on(receiver).unwrap_or_else(|error| {
            tracing::warn!(%error, "the KWin probe worker ended without a result");
            KwinAvailability::default()
        }),
        Err(error) => {
            tracing::warn!(%error, "the KWin probe thread could not be spawned");
            KwinAvailability::default()
        }
    }
}

/// The worker-side probe: a private runtime, the 15s budget, and the
/// degrade-to-default on any infrastructure failure.
fn probe_on_worker() -> KwinAvailability {
    let Ok(runtime) = crate::portal::run::build_runtime::<KwinError>() else {
        tracing::warn!("the KWin probe runtime could not be built; KWin unavailable");
        return KwinAvailability::default();
    };
    runtime.block_on(async {
        tokio::time::timeout(KWIN_TIMEOUT, probe_kwin())
            .await
            .unwrap_or_else(|_| {
                tracing::warn!(
                    timeout = ?KWIN_TIMEOUT,
                    "the KWin availability probe timed out; KWin unavailable"
                );
                KwinAvailability::default()
            })
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_capture::{DesktopEnv, negotiate};

    use super::*;

    #[test]
    fn availability_observes_exactly_the_kwin_rung() {
        let mut probe = CapabilityProbe::new(DesktopEnv::Kde, []);
        KwinAvailability { screenshot2: true }.observe_into(&mut probe);
        assert!(probe.supports(BackendKind::KwinScreenShot2));

        let mut absent = CapabilityProbe::new(DesktopEnv::Kde, []);
        KwinAvailability::default().observe_into(&mut absent);
        assert!(!absent.supports(BackendKind::KwinScreenShot2));
    }

    #[test]
    fn kde_ladder_orders_kwin_between_native_and_portals() {
        let mut probe = CapabilityProbe::new(
            DesktopEnv::Kde,
            [BackendKind::PortalScreenCast, BackendKind::PortalScreenshot],
        );
        KwinAvailability { screenshot2: true }.observe_into(&mut probe);
        let ladder = negotiate(&probe, None).unwrap();
        assert_eq!(
            ladder,
            vec![
                BackendKind::KwinScreenShot2,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
            ]
        );
    }

    #[test]
    fn version_reply_unwraps_nested_variants() {
        use zbus::zvariant::Value;
        assert_eq!(version_u32(&Value::U32(5)), Some(5));
        assert_eq!(version_u32(&Value::Value(Box::new(Value::U32(5)))), Some(5));
        assert_eq!(version_u32(&Value::Bool(true)), None);
    }
}

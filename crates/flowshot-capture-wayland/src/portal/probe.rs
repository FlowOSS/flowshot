//! Portal availability probing for the shared [`CapabilityProbe`].
//!
//! The portals are session-bus services, invisible to the Wayland registry,
//! so the registry-driven probe cannot see them (the gap the module docs of
//! [`crate::globals`] record). Availability = the `org.freedesktop.portal.Desktop`
//! bus name answers a `Properties.Get` for the interface's `version`
//! property: a frontend without `Screenshot` or `ScreenCast` support fails that
//! call per interface, which is exactly the granularity the negotiation
//! ladder needs.
//!
//! [`probe_portals_blocking`] is the sync convenience for probe assembly
//! outside an async context: it runs the check on a worker thread owning a
//! private `tokio` runtime (the same isolation the backends use, so it is
//! safe to call from any thread, including inside another executor).

use flowshot_capture::{BackendKind, CapabilityProbe};

use super::run::PORTAL_TIMEOUT;

/// The portal frontend's well-known bus name.
const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
/// The portal frontend's object path.
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
/// The standard D-Bus properties interface.
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";
/// The Screenshot portal interface.
const SCREENSHOT_IFACE: &str = "org.freedesktop.portal.Screenshot";
/// The `ScreenCast` portal interface.
const SCREENCAST_IFACE: &str = "org.freedesktop.portal.ScreenCast";

/// Which portal capture interfaces the session bus offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortalAvailability {
    /// `org.freedesktop.portal.Screenshot` answered a version query.
    pub screenshot: bool,
    /// `org.freedesktop.portal.ScreenCast` answered a version query.
    pub screencast: bool,
}

impl PortalAvailability {
    /// Records the available portal backends into `probe`, extending the
    /// registry-driven capabilities with the `D-Bus` side of the ladder.
    pub fn observe_into(&self, probe: &mut CapabilityProbe) {
        if self.screencast {
            probe.observe(BackendKind::PortalScreenCast);
        }
        if self.screenshot {
            probe.observe(BackendKind::PortalScreenshot);
        }
    }
}

/// Probes both portal interfaces on the session bus.
///
/// Infallible by design: no bus, no portal frontend, or a failed query all
/// mean "unavailable" (the negotiation ladder degrades, it never fails on a
/// probe). Must run inside a `tokio` runtime context (ashpd's `zbus` uses
/// the tokio reactor); callers outside one use [`probe_portals_blocking`].
pub async fn probe_portals() -> PortalAvailability {
    use ashpd::zbus;
    let Ok(connection) = zbus::Connection::session().await else {
        tracing::debug!("no session D-Bus connection; portals unavailable");
        return PortalAvailability::default();
    };
    PortalAvailability {
        screenshot: interface_available(&connection, SCREENSHOT_IFACE).await,
        screencast: interface_available(&connection, SCREENCAST_IFACE).await,
    }
}

/// [`probe_portals`] for synchronous callers: runs the probe on a worker
/// thread with a private runtime and blocks the calling thread until it
/// completes (bounded by the 15s portal budget; any failure degrades to
/// "unavailable").
#[must_use]
pub fn probe_portals_blocking() -> PortalAvailability {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("flowshot-portal-probe".to_owned())
        .spawn(move || {
            let availability = probe_on_worker();
            if sender.send(availability).is_err() {
                tracing::debug!("portal probe result discarded: the requester went away");
            }
        });
    match spawned {
        Ok(_worker) => futures::executor::block_on(receiver).unwrap_or_else(|error| {
            tracing::warn!(%error, "the portal probe worker ended without a result");
            PortalAvailability::default()
        }),
        Err(error) => {
            tracing::warn!(%error, "the portal probe thread could not be spawned");
            PortalAvailability::default()
        }
    }
}

/// The worker-side probe: a private runtime, the 15s budget, and the
/// degrade-to-default on any infrastructure failure.
fn probe_on_worker() -> PortalAvailability {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        tracing::warn!("the portal probe runtime could not be built; portals unavailable");
        return PortalAvailability::default();
    };
    runtime.block_on(async {
        tokio::time::timeout(PORTAL_TIMEOUT, probe_portals())
            .await
            .unwrap_or_else(|_| {
                tracing::warn!(
                    timeout = ?PORTAL_TIMEOUT,
                    "the portal availability probe timed out; portals unavailable"
                );
                PortalAvailability::default()
            })
    })
}

async fn interface_available(connection: &ashpd::zbus::Connection, interface: &str) -> bool {
    let queried = connection
        .call_method(
            Some(PORTAL_BUS),
            PORTAL_PATH,
            Some(PROPERTIES_IFACE),
            "Get",
            &(interface, "version"),
        )
        .await;
    match queried {
        Ok(_version) => true,
        Err(error) => {
            tracing::debug!(interface, %error, "portal interface unavailable");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use flowshot_capture::{DesktopEnv, negotiate};

    use super::*;

    #[test]
    fn availability_observes_exactly_the_available_rungs() {
        let mut probe = CapabilityProbe::new(DesktopEnv::Gnome, []);
        PortalAvailability {
            screenshot: true,
            screencast: true,
        }
        .observe_into(&mut probe);
        assert!(probe.supports(BackendKind::PortalScreenshot));
        assert!(probe.supports(BackendKind::PortalScreenCast));

        let mut partial = CapabilityProbe::new(DesktopEnv::Other, []);
        PortalAvailability {
            screenshot: true,
            screencast: false,
        }
        .observe_into(&mut partial);
        assert!(partial.supports(BackendKind::PortalScreenshot));
        assert!(!partial.supports(BackendKind::PortalScreenCast));
    }

    #[test]
    fn gnome_probe_with_portals_negotiates_screencast_before_screenshot() {
        // On GNOME the ScreenCast portal is the preferred
        // rung (own-overlay UX), Screenshot-interactive the rung-5 fallback.
        let mut probe = CapabilityProbe::new(DesktopEnv::Gnome, []);
        PortalAvailability {
            screenshot: true,
            screencast: true,
        }
        .observe_into(&mut probe);
        let ladder = negotiate(&probe, None).unwrap();
        assert_eq!(
            ladder,
            vec![BackendKind::PortalScreenCast, BackendKind::PortalScreenshot]
        );
    }

    #[test]
    fn portals_extend_a_native_ladder_without_reordering_it() {
        // Hyprland 2026: ICC + screencopy from the registry, portals from
        // the bus - the ladder keeps its exact priority order.
        let mut probe = CapabilityProbe::new(
            DesktopEnv::Hyprland,
            [BackendKind::ExtImageCopyCapture, BackendKind::WlrScreencopy],
        );
        PortalAvailability {
            screenshot: true,
            screencast: true,
        }
        .observe_into(&mut probe);
        let ladder = negotiate(&probe, None).unwrap();
        assert_eq!(
            ladder,
            vec![
                BackendKind::ExtImageCopyCapture,
                BackendKind::WlrScreencopy,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
            ]
        );
    }

    #[test]
    fn forced_portal_backend_wins_over_native_rungs() {
        let mut probe =
            CapabilityProbe::new(DesktopEnv::Hyprland, [BackendKind::ExtImageCopyCapture]);
        PortalAvailability {
            screenshot: true,
            screencast: true,
        }
        .observe_into(&mut probe);
        let forced = negotiate(&probe, Some(BackendKind::PortalScreenCast)).unwrap();
        assert_eq!(forced, vec![BackendKind::PortalScreenCast]);
    }
}

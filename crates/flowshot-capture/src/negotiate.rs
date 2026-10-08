//! Capability probing and the backend negotiation ladder.
//!
//! The platform crate fills a [`CapabilityProbe`] from what the session
//! actually offers (protocol globals seen, `D-Bus` name owners, portal
//! availability, desktop environment); [`negotiate`] turns that probe into an
//! ordered backend list. Negotiation is purely capability-driven: the desktop
//! environment is diagnostic context, never a filter, so a compositor that
//! grows a protocol is picked up without a release.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::CaptureError;
use crate::kind::BackendKind;

/// The desktop environment a probe was taken in.
///
/// Detection lives in the platform crates; this enum is the shared vocabulary.
/// `Other` is the honest fallback - never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopEnv {
    /// Hyprland.
    Hyprland,
    /// Sway.
    Sway,
    /// niri.
    Niri,
    /// COSMIC.
    Cosmic,
    /// KDE Plasma.
    Kde,
    /// GNOME.
    Gnome,
    /// Anything else, or unknown.
    #[default]
    Other,
}

/// The v1 negotiation ladder, in strict priority order: native zero-copy
/// compositor capture first, portal fallbacks next, `X11` last.
///
/// [`BackendKind::X11`] is the last rung by declaration, not by preference: a
/// session is either Wayland or `X11`, never both, so a probe that observes
/// `X11` observes no Wayland rung and the two families never compete for
/// position. Roadmap kinds ([`BackendKind::Windows`], [`BackendKind::MacOs`])
/// remain deliberately absent.
pub const NEGOTIATION_LADDER: [BackendKind; 6] = [
    BackendKind::ExtImageCopyCapture,
    BackendKind::WlrScreencopy,
    BackendKind::KwinScreenShot2,
    BackendKind::PortalScreenCast,
    BackendKind::PortalScreenshot,
    BackendKind::X11,
];

/// What the platform probe observed about a session's capture capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapabilityProbe {
    /// The backend kinds whose underlying protocol or service was observed
    /// (compositor globals seen, `D-Bus` name owned, portal available).
    ///
    /// Roadmap kinds placed here are ignored: [`CapabilityProbe::supports`]
    /// reports `false` for them in v1.
    pub available: BTreeSet<BackendKind>,
    /// The detected desktop environment. Informational for negotiation; used
    /// by cursor-position strategy selection and diagnostics.
    pub desktop: DesktopEnv,
}

impl CapabilityProbe {
    /// Creates a probe from the observed backend kinds.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::{BackendKind, CapabilityProbe, DesktopEnv};
    ///
    /// let probe = CapabilityProbe::new(
    ///     DesktopEnv::Niri,
    ///     [BackendKind::WlrScreencopy, BackendKind::PortalScreenshot],
    /// );
    /// assert!(probe.supports(BackendKind::WlrScreencopy));
    /// assert!(!probe.supports(BackendKind::ExtImageCopyCapture));
    /// ```
    #[must_use]
    pub fn new(desktop: DesktopEnv, available: impl IntoIterator<Item = BackendKind>) -> Self {
        Self {
            available: available.into_iter().collect(),
            desktop,
        }
    }

    /// Records one more observed backend kind.
    pub fn observe(&mut self, kind: BackendKind) {
        self.available.insert(kind);
    }

    /// Returns `true` when `kind` is a v1 backend *and* was observed in this
    /// probe. Roadmap kinds are never supported in v1, whatever `available`
    /// contains.
    #[must_use]
    pub fn supports(&self, kind: BackendKind) -> bool {
        !kind.is_roadmap() && self.available.contains(&kind)
    }
}

/// Chooses the capture backends for a probed session, best first.
///
/// Returns every ladder kind the probe supports, in
/// [`NEGOTIATION_LADDER`] order: `ext-image-copy-capture-v1` ->
/// `zwlr-screencopy-v1` -> `org.kde.KWin.ScreenShot2` ->
/// `org.freedesktop.portal.ScreenCast` ->
/// `org.freedesktop.portal.Screenshot` -> `X11`.
///
/// `config_override` is the `force_backend` config key. It wins over the
/// ladder: when set and supported, the result is exactly that one kind. A
/// forced kind the probe does *not* support (including roadmap kinds) fails
/// fast with a typed error naming it, instead of dying later at bind time.
///
/// # Errors
///
/// Returns [`CaptureError::NoBackendAvailable`] when no ladder kind is
/// supported - the message names every missing protocol - or when
/// `config_override` names an unsupported kind.
///
/// # Examples
///
/// ```
/// use flowshot_capture::{BackendKind, CapabilityProbe, DesktopEnv, negotiate};
///
/// let probe = CapabilityProbe::new(
///     DesktopEnv::Gnome,
///     [BackendKind::PortalScreenCast, BackendKind::PortalScreenshot],
/// );
/// let backends = negotiate(&probe, None).unwrap();
/// assert_eq!(
///     backends,
///     vec![BackendKind::PortalScreenCast, BackendKind::PortalScreenshot]
/// );
///
/// // A forced backend wins over ladder order:
/// let forced = negotiate(&probe, Some(BackendKind::PortalScreenshot)).unwrap();
/// assert_eq!(forced, vec![BackendKind::PortalScreenshot]);
///
/// // An empty probe is a typed error naming the missing protocols:
/// let empty = CapabilityProbe::default();
/// let err = negotiate(&empty, None).unwrap_err();
/// assert!(err.to_string().contains("ext-image-copy-capture-v1"));
/// ```
pub fn negotiate(
    probe: &CapabilityProbe,
    config_override: Option<BackendKind>,
) -> Result<Vec<BackendKind>, CaptureError> {
    if let Some(forced) = config_override {
        return if probe.supports(forced) {
            tracing::debug!(backend = %forced, "forced capture backend selected");
            Ok(vec![forced])
        } else {
            tracing::warn!(
                backend = %forced,
                desktop = ?probe.desktop,
                "forced capture backend is not available in this session"
            );
            Err(CaptureError::NoBackendAvailable {
                missing: vec![forced],
            })
        };
    }

    let backends: Vec<BackendKind> = NEGOTIATION_LADDER
        .into_iter()
        .filter(|kind| probe.supports(*kind))
        .collect();
    if backends.is_empty() {
        tracing::warn!(
            desktop = ?probe.desktop,
            "no capture backend available for the probed capabilities"
        );
        return Err(CaptureError::NoBackendAvailable {
            missing: NEGOTIATION_LADDER.to_vec(),
        });
    }
    tracing::debug!(?backends, desktop = ?probe.desktop, "negotiated capture backends");
    Ok(backends)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn probe(
        desktop: DesktopEnv,
        available: impl IntoIterator<Item = BackendKind>,
    ) -> CapabilityProbe {
        CapabilityProbe::new(desktop, available)
    }

    /// Hyprland >= 2026.02: ICC + screencopy + portals, no `KWin`.
    fn hyprland_2026() -> CapabilityProbe {
        probe(
            DesktopEnv::Hyprland,
            [
                BackendKind::ExtImageCopyCapture,
                BackendKind::WlrScreencopy,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
            ],
        )
    }

    fn portals_only() -> [BackendKind; 2] {
        [BackendKind::PortalScreenCast, BackendKind::PortalScreenshot]
    }

    #[test]
    fn hyprland_2026_gets_native_ladder_then_portals() {
        let backends = negotiate(&hyprland_2026(), None).unwrap();
        assert_eq!(
            backends,
            vec![
                BackendKind::ExtImageCopyCapture,
                BackendKind::WlrScreencopy,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
            ]
        );
    }

    #[test]
    fn niri_falls_to_screencopy_then_portals() {
        let niri = probe(
            DesktopEnv::Niri,
            [BackendKind::WlrScreencopy]
                .into_iter()
                .chain(portals_only()),
        );
        let backends = negotiate(&niri, None).unwrap();
        assert_eq!(
            backends,
            vec![
                BackendKind::WlrScreencopy,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
            ]
        );
    }

    #[test]
    fn gnome_gets_portals_only() {
        let gnome = probe(DesktopEnv::Gnome, portals_only());
        let backends = negotiate(&gnome, None).unwrap();
        assert_eq!(
            backends,
            vec![BackendKind::PortalScreenCast, BackendKind::PortalScreenshot]
        );
    }

    #[test]
    fn kde_prefers_the_kwin_fast_path() {
        let kde = probe(
            DesktopEnv::Kde,
            [BackendKind::KwinScreenShot2]
                .into_iter()
                .chain(portals_only()),
        );
        let backends = negotiate(&kde, None).unwrap();
        assert_eq!(
            backends,
            vec![
                BackendKind::KwinScreenShot2,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
            ]
        );
    }

    #[test]
    fn wlroots_without_portals_keeps_native_rungs() {
        let sway = probe(
            DesktopEnv::Sway,
            [BackendKind::ExtImageCopyCapture, BackendKind::WlrScreencopy],
        );
        let backends = negotiate(&sway, None).unwrap();
        assert_eq!(
            backends,
            vec![BackendKind::ExtImageCopyCapture, BackendKind::WlrScreencopy]
        );
    }

    #[test]
    fn force_backend_overrides_ladder_order() {
        let backends = negotiate(&hyprland_2026(), Some(BackendKind::WlrScreencopy)).unwrap();
        assert_eq!(backends, vec![BackendKind::WlrScreencopy]);
    }

    #[test]
    fn force_backend_last_rung_still_wins() {
        let backends = negotiate(&hyprland_2026(), Some(BackendKind::PortalScreenshot)).unwrap();
        assert_eq!(backends, vec![BackendKind::PortalScreenshot]);
    }

    #[test]
    fn force_backend_missing_capability_fails_fast_naming_it() {
        let gnome = probe(DesktopEnv::Gnome, portals_only());
        let err = negotiate(&gnome, Some(BackendKind::ExtImageCopyCapture)).unwrap_err();
        match &err {
            CaptureError::NoBackendAvailable { missing } => {
                assert_eq!(missing, &vec![BackendKind::ExtImageCopyCapture]);
            }
            other => panic!("expected NoBackendAvailable, got {other:?}"),
        }
        assert!(err.to_string().contains("ext-image-copy-capture-v1"));
    }

    #[test]
    fn force_roadmap_backend_is_rejected() {
        let err = negotiate(&hyprland_2026(), Some(BackendKind::Windows)).unwrap_err();
        assert!(matches!(err, CaptureError::NoBackendAvailable { .. }));
        assert!(err.to_string().contains("Windows.Graphics.Capture"));
    }

    #[test]
    fn force_x11_on_a_wayland_probe_fails_fast_naming_it() {
        let err = negotiate(&hyprland_2026(), Some(BackendKind::X11)).unwrap_err();
        match &err {
            CaptureError::NoBackendAvailable { missing } => {
                assert_eq!(missing, &vec![BackendKind::X11]);
            }
            other => panic!("expected NoBackendAvailable, got {other:?}"),
        }
        assert!(err.to_string().contains(BackendKind::X11.protocol_name()));
    }

    #[test]
    fn empty_probe_error_names_every_missing_protocol() {
        let empty = CapabilityProbe::default();
        let err = negotiate(&empty, None).unwrap_err();
        match &err {
            CaptureError::NoBackendAvailable { missing } => {
                assert_eq!(missing, &NEGOTIATION_LADDER.to_vec());
            }
            other => panic!("expected NoBackendAvailable, got {other:?}"),
        }
        let message = err.to_string();
        for kind in NEGOTIATION_LADDER {
            assert!(message.contains(kind.protocol_name()), "{message}");
        }
    }

    #[test]
    fn full_probe_preserves_exact_ladder_order() {
        let full = probe(DesktopEnv::Other, NEGOTIATION_LADDER);
        let backends = negotiate(&full, None).unwrap();
        assert_eq!(
            backends,
            vec![
                BackendKind::ExtImageCopyCapture,
                BackendKind::WlrScreencopy,
                BackendKind::KwinScreenShot2,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
                BackendKind::X11,
            ]
        );
    }

    #[test]
    fn desktop_env_does_not_change_capability_negotiation() {
        let as_hyprland = probe(DesktopEnv::Hyprland, portals_only());
        let as_gnome = probe(DesktopEnv::Gnome, portals_only());
        assert_eq!(
            negotiate(&as_hyprland, None).unwrap(),
            negotiate(&as_gnome, None).unwrap()
        );
    }

    #[test]
    fn roadmap_kinds_in_available_set_are_ignored() {
        let odd = probe(
            DesktopEnv::Other,
            [BackendKind::Windows, BackendKind::MacOs]
                .into_iter()
                .chain([BackendKind::PortalScreenshot]),
        );
        assert!(!odd.supports(BackendKind::Windows));
        assert!(!odd.supports(BackendKind::MacOs));
        let backends = negotiate(&odd, None).unwrap();
        assert_eq!(backends, vec![BackendKind::PortalScreenshot]);
    }

    #[test]
    fn observe_extends_the_probe_incrementally() {
        let mut growing = CapabilityProbe::default();
        assert!(matches!(
            negotiate(&growing, None),
            Err(CaptureError::NoBackendAvailable { .. })
        ));
        growing.observe(BackendKind::WlrScreencopy);
        assert_eq!(
            negotiate(&growing, None).unwrap(),
            vec![BackendKind::WlrScreencopy]
        );
    }

    #[test]
    fn single_capability_probe_returns_single_rung() {
        let icc_only = probe(DesktopEnv::Cosmic, [BackendKind::ExtImageCopyCapture]);
        assert_eq!(
            negotiate(&icc_only, None).unwrap(),
            vec![BackendKind::ExtImageCopyCapture]
        );
    }

    #[test]
    fn x11_only_probe_negotiates_the_x11_rung() {
        let x11 = probe(DesktopEnv::Other, [BackendKind::X11]);
        assert!(x11.supports(BackendKind::X11));
        assert_eq!(negotiate(&x11, None).unwrap(), vec![BackendKind::X11]);
    }

    #[test]
    fn force_x11_on_an_x11_probe_selects_it() {
        let x11 = probe(DesktopEnv::Other, [BackendKind::X11]);
        assert_eq!(
            negotiate(&x11, Some(BackendKind::X11)).unwrap(),
            vec![BackendKind::X11]
        );
    }

    #[test]
    fn wayland_probe_does_not_gain_the_x11_rung() {
        let wayland = probe(
            DesktopEnv::Sway,
            [
                BackendKind::ExtImageCopyCapture,
                BackendKind::WlrScreencopy,
                BackendKind::KwinScreenShot2,
                BackendKind::PortalScreenCast,
                BackendKind::PortalScreenshot,
            ],
        );
        let backends = negotiate(&wayland, None).unwrap();
        assert!(!backends.contains(&BackendKind::X11));
        assert_eq!(backends.len(), 5);
    }
}

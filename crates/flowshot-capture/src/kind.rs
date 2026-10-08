//! Backend identity: which capture mechanism a backend implements.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Identifies a capture backend implementation.
///
/// The first six variants form the negotiation ladder and are declared in
/// exact ladder priority order (see
/// [`NEGOTIATION_LADDER`](crate::negotiate::NEGOTIATION_LADDER)): native
/// zero-copy compositor capture first, portal fallbacks next, `X11` last.
/// `X11` sits last by declaration, not by preference: a session is either
/// Wayland or `X11`, never both, so a probe reports rungs from one family
/// only and the two never compete.
///
/// The last two variants are roadmap placeholders for non-Linux platforms.
/// They are documented, non-constructible outcomes of negotiation:
/// [`CapabilityProbe::supports`](crate::CapabilityProbe::supports)
/// reports `false` for them on every probe, so [`negotiate()`](crate::negotiate())
/// never returns them, and forcing one is a typed
/// [`CaptureError::NoBackendAvailable`](crate::CaptureError::NoBackendAvailable).
/// The platform audit (ADR-006) tracks turning them into real backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BackendKind {
    /// The `ext-image-copy-capture-v1` compositor protocol with cursor
    /// sessions (Hyprland >= 2026.02, wlroots >= 0.19, COSMIC, and friends).
    #[serde(rename = "ext-image-copy-capture")]
    ExtImageCopyCapture,
    /// The `zwlr-screencopy-v1` compositor protocol (niri, older wlroots).
    #[serde(rename = "wlr-screencopy")]
    WlrScreencopy,
    /// The `org.kde.KWin.ScreenShot2` `D-Bus` fast path (KDE Plasma).
    #[serde(rename = "kwin-screenshot2")]
    KwinScreenShot2,
    /// The `org.freedesktop.portal.ScreenCast` portal, single-frame via
    /// `PipeWire` (universal fallback).
    #[serde(rename = "portal-screencast")]
    PortalScreenCast,
    /// The `org.freedesktop.portal.Screenshot` portal (last-resort fallback,
    /// interactive picker UX on GNOME).
    #[serde(rename = "portal-screenshot")]
    PortalScreenshot,
    /// `X11` via `xcb` `GetImage` (X11-only sessions: i3, Xfce, Openbox).
    #[serde(rename = "x11")]
    X11,
    /// Roadmap: Windows.Graphics.Capture. Not available in v1.
    #[serde(rename = "windows")]
    Windows,
    /// Roadmap: `macOS` `ScreenCaptureKit`. Not available in v1.
    #[serde(rename = "macos")]
    MacOs,
}

impl BackendKind {
    /// Returns `true` for the roadmap platform variants that v1 never builds.
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::BackendKind;
    ///
    /// assert!(BackendKind::Windows.is_roadmap());
    /// assert!(!BackendKind::ExtImageCopyCapture.is_roadmap());
    /// assert!(!BackendKind::X11.is_roadmap());
    /// ```
    #[must_use]
    pub const fn is_roadmap(self) -> bool {
        matches!(self, Self::Windows | Self::MacOs)
    }

    /// The wire protocol or service name this backend speaks, used in
    /// diagnostics and in the [`CaptureError::NoBackendAvailable`] message.
    ///
    /// [`CaptureError::NoBackendAvailable`]: crate::CaptureError::NoBackendAvailable
    ///
    /// # Examples
    ///
    /// ```
    /// use flowshot_capture::BackendKind;
    ///
    /// assert_eq!(
    ///     BackendKind::WlrScreencopy.protocol_name(),
    ///     "zwlr-screencopy-v1"
    /// );
    /// ```
    #[must_use]
    pub const fn protocol_name(self) -> &'static str {
        match self {
            Self::ExtImageCopyCapture => "ext-image-copy-capture-v1",
            Self::WlrScreencopy => "zwlr-screencopy-v1",
            Self::KwinScreenShot2 => "org.kde.KWin.ScreenShot2",
            Self::PortalScreenCast => "org.freedesktop.portal.ScreenCast",
            Self::PortalScreenshot => "org.freedesktop.portal.Screenshot",
            Self::X11 => "X11 (xcb GetImage)",
            Self::Windows => "Windows.Graphics.Capture (roadmap, not built in v1)",
            Self::MacOs => "ScreenCaptureKit (roadmap, not built in v1)",
        }
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.protocol_name())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn declaration_order_matches_ladder_order() {
        let ladder = [
            BackendKind::ExtImageCopyCapture,
            BackendKind::WlrScreencopy,
            BackendKind::KwinScreenShot2,
            BackendKind::PortalScreenCast,
            BackendKind::PortalScreenshot,
        ];
        for pair in ladder.windows(2) {
            assert!(pair[0] < pair[1], "ladder order must match Ord");
        }
        for kind in ladder {
            assert!(kind < BackendKind::X11);
        }
    }

    #[test]
    fn display_shows_protocol_name() {
        assert_eq!(
            BackendKind::PortalScreenCast.to_string(),
            "org.freedesktop.portal.ScreenCast"
        );
    }
}

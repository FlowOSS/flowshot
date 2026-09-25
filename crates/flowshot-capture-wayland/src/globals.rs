//! Registry global tracking and capture-protocol feature detection.
//!
//! Detection is by interface name (the wire ABI, stable by protocol policy):
//! todo 6 binds only what output enumeration needs (`wl_output`,
//! `zxdg_output_manager_v1`); the capture backends bind their own managers
//! when they build sessions on top of this module.

use flowshot_capture::{BackendKind, CapabilityProbe, DesktopEnv};
use serde::Serialize;

/// Interface name of the `ext-image-copy-capture-v1` manager.
pub(crate) const EXT_IMAGE_COPY_CAPTURE_MANAGER: &str = "ext_image_copy_capture_manager_v1";
/// Interface name of the per-output capture source manager.
pub(crate) const EXT_OUTPUT_IMAGE_CAPTURE_SOURCE_MANAGER: &str =
    "ext_output_image_capture_source_manager_v1";
/// Interface name of the per-toplevel capture source manager.
const EXT_FOREIGN_TOPLEVEL_IMAGE_CAPTURE_SOURCE_MANAGER: &str =
    "ext_foreign_toplevel_image_capture_source_manager_v1";
/// Interface name of the `wlr-screencopy-unstable-v1` manager.
pub(crate) const WLR_SCREENCOPY_MANAGER: &str = "zwlr_screencopy_manager_v1";
/// Interface name of the `xdg-output-unstable-v1` manager.
pub(crate) const ZXDG_OUTPUT_MANAGER: &str = "zxdg_output_manager_v1";
/// Interface name of the shared-memory pool factory.
pub(crate) const WL_SHM: &str = "wl_shm";
/// Interface name of the `linux-dmabuf-v1` factory.
const ZWP_LINUX_DMABUF: &str = "zwp_linux_dmabuf_v1";
/// Interface name of the fractional-scale manager (current `wp_` spelling).
const WP_FRACTIONAL_SCALE_MANAGER: &str = "wp_fractional_scale_manager_v1";
/// Interface name of the fractional-scale manager (legacy `zwp_` spelling
/// still advertised by older compositors).
const ZWP_FRACTIONAL_SCALE_MANAGER: &str = "zwp_fractional_scale_manager_v1";
/// Interface name of the core output global.
pub(crate) const WL_OUTPUT: &str = "wl_output";
/// Interface name of the core seat global (pointer capability gates the
/// `ext-image-copy-capture-v1` cursor session; todo 8).
pub(crate) const WL_SEAT: &str = "wl_seat";

/// One global advertised by the compositor's `wl_registry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Global {
    /// Registry-unique numeric name (the bind handle).
    pub name: u32,
    /// Interface name, e.g. `wl_output`.
    pub interface: String,
    /// Version advertised by the compositor.
    pub version: u32,
}

impl Global {
    /// Creates a global record.
    #[must_use]
    pub fn new(name: u32, interface: impl Into<String>, version: u32) -> Self {
        Self {
            name,
            interface: interface.into(),
            version,
        }
    }
}

/// The capture-relevant protocols observed in the registry, with the version
/// the compositor advertised for each (`None` = not advertised).
///
/// `linux_dmabuf` and `fractional_scale_manager` are detect-only in v1
/// (shared-memory buffers only; integer output scale). Compositor-side
/// fractional scaling needs a surface round-trip this crate does not perform
/// yet - see the crate-level docs for the v1 limitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ProtocolGlobals {
    /// `ext_image_copy_capture_manager_v1` version.
    pub ext_image_copy_capture_manager: Option<u32>,
    /// `ext_output_image_capture_source_manager_v1` version.
    pub ext_output_image_capture_source_manager: Option<u32>,
    /// `ext_foreign_toplevel_image_capture_source_manager_v1` version.
    pub ext_foreign_toplevel_image_capture_source_manager: Option<u32>,
    /// `zwlr_screencopy_manager_v1` version.
    pub wlr_screencopy_manager: Option<u32>,
    /// `zxdg_output_manager_v1` version.
    pub xdg_output_manager: Option<u32>,
    /// `wl_shm` version.
    pub shm: Option<u32>,
    /// `zwp_linux_dmabuf_v1` version (detect-only in v1).
    pub linux_dmabuf: Option<u32>,
    /// Fractional-scale manager version, `wp_` or legacy `zwp_` spelling
    /// (detect-only in v1).
    pub fractional_scale_manager: Option<u32>,
}

impl ProtocolGlobals {
    /// Extracts the capture-relevant protocols from a raw global list.
    ///
    /// Unknown interfaces are ignored; the raw list stays available for
    /// diagnostics.
    #[must_use]
    pub fn from_globals(globals: &[Global]) -> Self {
        let version_of = |interface: &str| {
            globals
                .iter()
                .find(|global| global.interface == interface)
                .map(|global| global.version)
        };
        Self {
            ext_image_copy_capture_manager: version_of(EXT_IMAGE_COPY_CAPTURE_MANAGER),
            ext_output_image_capture_source_manager: version_of(
                EXT_OUTPUT_IMAGE_CAPTURE_SOURCE_MANAGER,
            ),
            ext_foreign_toplevel_image_capture_source_manager: version_of(
                EXT_FOREIGN_TOPLEVEL_IMAGE_CAPTURE_SOURCE_MANAGER,
            ),
            wlr_screencopy_manager: version_of(WLR_SCREENCOPY_MANAGER),
            xdg_output_manager: version_of(ZXDG_OUTPUT_MANAGER),
            shm: version_of(WL_SHM),
            linux_dmabuf: version_of(ZWP_LINUX_DMABUF),
            fractional_scale_manager: version_of(WP_FRACTIONAL_SCALE_MANAGER)
                .or_else(|| version_of(ZWP_FRACTIONAL_SCALE_MANAGER)),
        }
    }

    /// Maps the observed globals onto the shared [`CapabilityProbe`].
    ///
    /// `ExtImageCopyCapture` requires BOTH the capture manager and the
    /// per-output source manager: without a source manager no output capture
    /// source can be created, so the backend would fail at bind time. The
    /// foreign-toplevel source manager (window capture) is recorded in
    /// [`ProtocolGlobals`] but does not enable output capture. `KWin` and the
    /// portals are `D-Bus` services, invisible to the Wayland registry - the
    /// later backend todos extend the probe with those checks.
    #[must_use]
    pub fn to_capability_probe(&self, desktop: DesktopEnv) -> CapabilityProbe {
        let mut probe = CapabilityProbe::new(desktop, []);
        if self.ext_image_copy_capture_manager.is_some()
            && self.ext_output_image_capture_source_manager.is_some()
        {
            probe.observe(BackendKind::ExtImageCopyCapture);
        }
        if self.wlr_screencopy_manager.is_some() {
            probe.observe(BackendKind::WlrScreencopy);
        }
        probe
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn global(interface: &str, version: u32) -> Global {
        Global::new(1, interface, version)
    }

    /// The full set a 2026 Hyprland advertises, plus unrelated interfaces.
    fn hyprland_globals() -> Vec<Global> {
        vec![
            global(EXT_IMAGE_COPY_CAPTURE_MANAGER, 1),
            global(EXT_OUTPUT_IMAGE_CAPTURE_SOURCE_MANAGER, 1),
            global(EXT_FOREIGN_TOPLEVEL_IMAGE_CAPTURE_SOURCE_MANAGER, 1),
            global(WLR_SCREENCOPY_MANAGER, 3),
            global(ZXDG_OUTPUT_MANAGER, 3),
            global(WL_SHM, 1),
            global(ZWP_LINUX_DMABUF, 4),
            global(WP_FRACTIONAL_SCALE_MANAGER, 1),
            global("wl_compositor", 6),
        ]
    }

    #[test]
    fn hyprland_globals_enable_icc_and_screencopy() {
        let protocols = ProtocolGlobals::from_globals(&hyprland_globals());
        let probe = protocols.to_capability_probe(DesktopEnv::Hyprland);
        assert!(probe.supports(BackendKind::ExtImageCopyCapture));
        assert!(probe.supports(BackendKind::WlrScreencopy));
        assert!(!probe.supports(BackendKind::KwinScreenShot2));
        assert_eq!(probe.desktop, DesktopEnv::Hyprland);
    }

    #[test]
    fn icc_without_output_source_manager_is_not_available() {
        let mut globals = hyprland_globals();
        globals.retain(|global| global.interface != EXT_OUTPUT_IMAGE_CAPTURE_SOURCE_MANAGER);
        let protocols = ProtocolGlobals::from_globals(&globals);
        assert!(protocols.ext_image_copy_capture_manager.is_some());
        let probe = protocols.to_capability_probe(DesktopEnv::Other);
        assert!(!probe.supports(BackendKind::ExtImageCopyCapture));
        assert!(probe.supports(BackendKind::WlrScreencopy));
    }

    #[test]
    fn foreign_toplevel_source_alone_does_not_enable_icc() {
        let globals = vec![
            global(EXT_IMAGE_COPY_CAPTURE_MANAGER, 1),
            global(EXT_FOREIGN_TOPLEVEL_IMAGE_CAPTURE_SOURCE_MANAGER, 1),
        ];
        let probe = ProtocolGlobals::from_globals(&globals).to_capability_probe(DesktopEnv::Other);
        assert!(!probe.supports(BackendKind::ExtImageCopyCapture));
    }

    #[test]
    fn screencopy_only_session_maps_to_screencopy() {
        let globals = vec![global(WLR_SCREENCOPY_MANAGER, 3), global(WL_SHM, 1)];
        let protocols = ProtocolGlobals::from_globals(&globals);
        let probe = protocols.to_capability_probe(DesktopEnv::Niri);
        assert!(probe.supports(BackendKind::WlrScreencopy));
        assert!(!probe.supports(BackendKind::ExtImageCopyCapture));
        assert_eq!(protocols.wlr_screencopy_manager, Some(3));
        assert_eq!(protocols.shm, Some(1));
    }

    #[test]
    fn empty_registry_produces_empty_probe() {
        let protocols = ProtocolGlobals::from_globals(&[]);
        assert_eq!(protocols, ProtocolGlobals::default());
        let probe = protocols.to_capability_probe(DesktopEnv::Other);
        assert!(probe.available.is_empty());
    }

    #[test]
    fn versions_are_recorded_per_protocol() {
        let protocols = ProtocolGlobals::from_globals(&hyprland_globals());
        assert_eq!(protocols.ext_image_copy_capture_manager, Some(1));
        assert_eq!(protocols.xdg_output_manager, Some(3));
        assert_eq!(protocols.linux_dmabuf, Some(4));
        assert_eq!(protocols.fractional_scale_manager, Some(1));
    }

    #[test]
    fn legacy_zwp_fractional_scale_spelling_is_detected() {
        let globals = vec![global(ZWP_FRACTIONAL_SCALE_MANAGER, 1)];
        let protocols = ProtocolGlobals::from_globals(&globals);
        assert_eq!(protocols.fractional_scale_manager, Some(1));
    }

    #[test]
    fn unknown_interfaces_are_ignored() {
        let globals = vec![global("wl_compositor", 6), global("zwp_input_method_v2", 1)];
        let protocols = ProtocolGlobals::from_globals(&globals);
        assert_eq!(protocols, ProtocolGlobals::default());
    }
}

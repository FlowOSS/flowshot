//! The launcher window's option bag and its two callback seams (the
//! settings-window pattern: the lib stays pure, the binary layer owns
//! platform services).

use std::sync::Arc;

use flowshot_core::config::UiConfig;
use flowshot_core::geometry::OutputInfo;
use flowshot_core::tokens::DesignTokens;

use crate::egui_host::{ClipboardBridge, ThemeMode};
use crate::pins::WindowCustomizer;

use super::request::LauncherRequest;

/// The live output probe seam: the binary layer wires the capture
/// backend's probe (the daemon side already runs one for the tray
/// submenu); `None` probe = the dropdown offers manual geometry only
/// (headless-safe, the tray's infallible-probe contract).
#[derive(Clone)]
pub struct MonitorProbe(Arc<dyn Fn() -> Vec<OutputInfo> + Send + Sync + 'static>);

impl MonitorProbe {
    /// Wraps a probe function.
    pub fn new(probe: impl Fn() -> Vec<OutputInfo> + Send + Sync + 'static) -> Self {
        Self(Arc::new(probe))
    }

    /// Runs the probe (once, at window creation).
    #[must_use]
    pub fn probe(&self) -> Vec<OutputInfo> {
        (self.0)()
    }
}

impl std::fmt::Debug for MonitorProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MonitorProbe(..)")
    }
}

/// Receives the typed request when Capture is clicked (the binary layer
/// maps it onto the daemon's `Capture` / `CaptureScreen` commands - see the
/// module header). The window closes right after the callback returns.
#[derive(Clone)]
pub struct LaunchCallback(Arc<dyn Fn(&LauncherRequest) + Send + Sync + 'static>);

impl LaunchCallback {
    /// Wraps a dispatch function.
    pub fn new(dispatch: impl Fn(&LauncherRequest) + Send + Sync + 'static) -> Self {
        Self(Arc::new(dispatch))
    }

    pub(super) fn invoke(&self, request: &LauncherRequest) {
        (self.0)(request);
    }
}

impl std::fmt::Debug for LaunchCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LaunchCallback(..)")
    }
}

/// Everything the launcher window needs beyond the model.
#[derive(Clone, Debug, Default)]
pub struct LauncherWindowOptions {
    /// Design tokens driving the egui style projection.
    pub tokens: DesignTokens,
    /// The `[ui]` config group (accent/contrast/theme colors) the style is
    /// projected from; the binary layer passes the loaded config's group.
    pub ui_config: UiConfig,
    /// The resolved system dark/light preference (binary layer, ashpd
    /// Settings portal).
    pub system_theme: ThemeMode,
    /// The live output probe; `None` = manual geometry only.
    pub monitor_probe: Option<MonitorProbe>,
    /// The Capture dispatch seam; `None` = the request is only logged.
    pub on_capture: Option<LaunchCallback>,
    /// The clipboard seam for Ctrl+C/V in the geometry field.
    pub clipboard: Option<ClipboardBridge>,
    /// Window-attributes hook (binary layer sets `app_id=flowshot-launcher`,
    /// the `pins::WindowCustomizer` precedent).
    pub window_customizer: Option<WindowCustomizer>,
}

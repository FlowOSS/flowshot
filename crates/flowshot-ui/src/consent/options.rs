//! The consent window's option bag and its persistence seam (the
//! launcher/settings pattern: the lib stays pure, the binary layer owns
//! platform services AND the config write).

use std::sync::Arc;

use flowshot_core::config::{TelemetryConfig, UiConfig};
use flowshot_core::tokens::DesignTokens;

use crate::egui_host::ThemeMode;
use crate::pins::WindowCustomizer;

/// Receives the chosen telemetry config when the dialog closes through ANY
/// user dismissal path ("Save choice", "Not now", Esc, the window close) -
/// exactly one invocation per dialog lifetime. The binary layer merges it
/// into the loaded config and persists through
/// [`Config::save`](flowshot_core::config::Config::save) (the
/// migration-safe save path); the lib never writes the file. A programmatic
/// [`ConsentHandle::request_exit`](super::ConsentHandle::request_exit) is
/// NOT a user answer and invokes nothing.
#[derive(Clone)]
pub struct ConsentCallback(Arc<dyn Fn(&TelemetryConfig) + Send + Sync + 'static>);

impl ConsentCallback {
    /// Wraps a persistence function.
    pub fn new(save: impl Fn(&TelemetryConfig) + Send + Sync + 'static) -> Self {
        Self(Arc::new(save))
    }

    pub(super) fn invoke(&self, choice: TelemetryConfig) {
        (self.0)(&choice);
    }
}

impl std::fmt::Debug for ConsentCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConsentCallback(..)")
    }
}

/// Everything the consent window needs beyond the model.
#[derive(Clone, Debug, Default)]
pub struct ConsentWindowOptions {
    /// Design tokens driving the egui style projection.
    pub tokens: DesignTokens,
    /// The `[ui]` config group (accent/contrast colors) the style is
    /// projected from; the binary layer passes the loaded config's group.
    pub ui_config: UiConfig,
    /// The resolved system dark/light preference (binary layer, ashpd
    /// Settings portal).
    pub system_theme: ThemeMode,
    /// The persistence seam; `None` = the choice is only logged.
    pub on_choice: Option<ConsentCallback>,
    /// Window-attributes hook (the binary layer sets
    /// `app_id=flowshot-consent`, the `pins::WindowCustomizer` precedent).
    pub window_customizer: Option<WindowCustomizer>,
}

//! The window's option bag and its two callback seams.

use std::path::PathBuf;
use std::sync::Arc;

use flowshot_core::config::Config;
use flowshot_core::tokens::DesignTokens;

use crate::pins::WindowCustomizer;

use crate::egui_host::ClipboardBridge;
use crate::egui_host::theme::ThemeMode;

/// The save-path dialog seam (binary layer wires rfd; a `None` picker
/// disables the Browse button).
#[derive(Clone)]
pub struct PathPicker(Arc<dyn Fn() -> Option<String> + Send + Sync + 'static>);

impl PathPicker {
    /// Wraps a picker function.
    pub fn new(pick: impl Fn() -> Option<String> + Send + Sync + 'static) -> Self {
        Self(Arc::new(pick))
    }

    /// Runs the dialog; `None` = cancelled.
    #[must_use]
    pub fn pick(&self) -> Option<String> {
        (self.0)()
    }
}

impl std::fmt::Debug for PathPicker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PathPicker(..)")
    }
}

/// Notified with the validated config after a successful Apply write (the
/// binary layer's `ConfigChanged` signal emitter + live-reload trigger).
#[derive(Clone)]
pub struct AppliedCallback(Arc<dyn Fn(&Config) + Send + Sync + 'static>);

impl AppliedCallback {
    /// Wraps a notification function.
    pub fn new(notify: impl Fn(&Config) + Send + Sync + 'static) -> Self {
        Self(Arc::new(notify))
    }

    pub(super) fn invoke(&self, config: &Config) {
        (self.0)(config);
    }
}

impl std::fmt::Debug for AppliedCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppliedCallback(..)")
    }
}

/// Everything the window needs beyond the model.
#[derive(Clone, Debug)]
pub struct SettingsWindowOptions {
    /// The TOML file Apply writes (migration-safe through `Config::save`).
    pub config_path: PathBuf,
    /// The system dark/light preference resolved by the binary layer (ashpd
    /// Settings portal); `ThemeChoice::System` defers to it.
    pub system_theme: ThemeMode,
    /// Design tokens driving the egui style projection.
    pub tokens: DesignTokens,
    /// The save-path dialog seam; `None` disables Browse.
    pub path_picker: Option<PathPicker>,
    /// The clipboard seam for Ctrl+C/V in text fields.
    pub clipboard: Option<ClipboardBridge>,
    /// Post-write Apply notification (`ConfigChanged` emitter).
    pub on_applied: Option<AppliedCallback>,
    /// Window-attributes hook (binary layer sets `app_id=flowshot-settings`).
    pub window_customizer: Option<WindowCustomizer>,
}

impl SettingsWindowOptions {
    /// Options for `config_path` with every seam unset and the dark theme.
    #[must_use]
    pub fn new(config_path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
            system_theme: ThemeMode::default(),
            tokens: DesignTokens::default(),
            path_picker: None,
            clipboard: None,
            on_applied: None,
            window_customizer: None,
        }
    }
}

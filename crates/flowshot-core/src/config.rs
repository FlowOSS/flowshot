//! Grouped TOML configuration schema for `FlowShot`.
//!
//! The config is organized into semantic groups (`[capture]`, `[save]`,
//! `[editor]`, `[tools.*]`, `[pin]`, `[upload]`, `[ui]`, `[daemon]`) rather
//! than a flat key dump. A top-level `config_version` field drives forward
//! migration: older on-disk configs are upgraded step-by-step to
//! [`CONFIG_VERSION`] before deserialization.
//!
//! # Examples
//!
//! ```
//! use flowshot_core::config::Config;
//!
//! let config = Config::default();
//! let toml_text = config.to_toml_string().unwrap_or_else(|e| panic!("{e}"));
//! let reloaded = Config::from_toml_str(&toml_text).unwrap_or_else(|e| panic!("{e}"));
//! assert_eq!(config, reloaded);
//! ```

use std::fs;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tokens::{Palette, Typography};

/// Current config schema version written by this build.
pub const CONFIG_VERSION: u32 = 2;

/// TOML key holding the schema version.
const VERSION_KEY: &str = "config_version";

/// Errors produced while loading, migrating, or saving a [`Config`].
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The TOML text could not be parsed or did not match the schema.
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    /// The config could not be serialized to TOML.
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// Filesystem operation failed.
    #[error("config I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The top-level TOML value was not a table.
    #[error("top-level config must be a TOML table")]
    NotATable,
    /// `config_version` was present but not a non-negative integer.
    #[error("config_version must be a non-negative integer")]
    InvalidVersion,
    /// The file was written by a newer version of `FlowShot`.
    #[error(
        "config version {found} is newer than supported version {supported}; please upgrade FlowShot"
    )]
    FutureVersion {
        /// Version found in the file.
        found: u32,
        /// Highest version this build understands.
        supported: u32,
    },
    /// A migration step failed or no migration path exists.
    #[error("failed to migrate config from version {from}: {reason}")]
    Migration {
        /// Version the migration started from.
        from: u32,
        /// Human-readable failure reason.
        reason: String,
    },
}

/// A rectangular screen region, persisted as capture state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    /// X coordinate of the top-left corner.
    pub x: i32,
    /// Y coordinate of the top-left corner.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// `[capture]` — screen capture behavior and persisted capture state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    /// Hide the mouse cursor in captures.
    pub hide_cursor: bool,
    /// Remember the last selected region between sessions.
    pub save_last_region: bool,
    /// The last selected region (persisted state, absent until first capture).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_region: Option<Region>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            hide_cursor: false,
            save_last_region: true,
            last_region: None,
        }
    }
}

/// Clipboard image encoding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardFormat {
    /// Lossless PNG.
    #[default]
    Png,
    /// Lossy JPEG (quality controlled by [`SaveConfig::jpeg_quality`]).
    Jpeg,
}

/// What happens to a capture after the editor is closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveAction {
    /// Copy the image to the clipboard.
    #[default]
    Copy,
    /// Write the image to disk.
    Save,
    /// Open the image as a floating pin.
    Pin,
    /// Upload the image to the configured provider.
    Upload,
}

/// `[save]` — output path, file naming, and post-capture actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveConfig {
    /// Save directory; empty means the platform pictures directory.
    pub path: String,
    /// If true, always save to `path` without prompting.
    pub path_fixed: bool,
    /// Default file extension (`png`, `jpg`, ...).
    pub extension: String,
    /// Filename pattern using `strftime`-style placeholders.
    pub filename_pattern: String,
    /// JPEG quality (1-100) when saving or copying as JPEG.
    pub jpeg_quality: u8,
    /// Image format placed on the clipboard.
    pub clipboard_format: ClipboardFormat,
    /// Actions executed when the editor is closed.
    pub actions: Vec<SaveAction>,
}

impl Default for SaveConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            path_fixed: false,
            extension: "png".to_owned(),
            filename_pattern: "%F_%H-%M".to_owned(),
            jpeg_quality: 75,
            clipboard_format: ClipboardFormat::default(),
            actions: vec![SaveAction::Copy],
        }
    }
}

/// Shape of the pixel magnifier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MagnifierShape {
    /// Square magnifier.
    #[default]
    Square,
    /// Circular magnifier.
    Circle,
}

/// `[editor]` — annotation editor defaults.
// The boolean flags are independent user-facing settings mandated by the
// config schema; grouping them would obscure the TOML layout.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    /// Active drawing color (`#RRGGBB`).
    pub draw_color: String,
    /// Stroke thickness in pixels for drawing tools.
    pub draw_thickness: u32,
    /// Font family for the text tool.
    pub font_family: String,
    /// Font size for the text tool.
    pub font_size: u32,
    /// Show the pixel magnifier while selecting or drawing.
    pub magnifier: bool,
    /// Magnifier shape.
    pub magnifier_shape: MagnifierShape,
    /// HUD corner (1 = top-left, 2 = top-right, 3 = bottom-left, 4 = bottom-right).
    pub hud_position: u8,
    /// Milliseconds of inactivity before the HUD auto-hides.
    pub hud_hide_time: u32,
    /// Show a snapping grid in the editor.
    pub grid: bool,
    /// Maximum number of undo steps kept per session.
    pub undo_limit: u32,
    /// Swatches offered by the color picker.
    pub color_palette: Vec<String>,
    /// Double-clicking the selection copies it immediately.
    pub double_click_copies: bool,
    /// Show the side panel with tool options.
    pub side_panel: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            draw_color: "#FF0000".to_owned(),
            draw_thickness: 3,
            font_family: Typography::default().family,
            font_size: 8,
            magnifier: false,
            magnifier_shape: MagnifierShape::default(),
            hud_position: 4,
            hud_hide_time: 3000,
            grid: false,
            undo_limit: 100,
            color_palette: default_color_palette(),
            double_click_copies: false,
            side_panel: true,
        }
    }
}

/// Default color-picker swatches.
fn default_color_palette() -> Vec<String> {
    [
        "#000000", "#7F7F7F", "#880015", "#ED1C24", "#FF7F27", "#FFF200", "#22B14C", "#00A2E8",
        "#3F48CC", "#A349A4", "#FFFFFF", "#C3C3C3", "#B97A57", "#FFAEC9", "#FFC90E", "#EFE4B0",
        "#B5E61D", "#99D9EA", "#7092BE", "#C8BFE7",
    ]
    .iter()
    .map(|c| (*c).to_owned())
    .collect()
}

/// Arrow tool line style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArrowStyle {
    /// Straight shaft.
    #[default]
    Straight,
    /// Curved (quadratic) shaft.
    Curved,
}

/// `[tools.arrow]` — arrow tool options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrowToolConfig {
    /// Shaft style.
    pub style: ArrowStyle,
    /// Draw the arrow head at the start point instead of the end.
    pub reverse: bool,
}

/// `[tools.marker]` — highlighter marker options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MarkerToolConfig {
    /// Marker stroke width in pixels.
    pub size: u32,
}

impl Default for MarkerToolConfig {
    fn default() -> Self {
        Self { size: 5 }
    }
}

/// `[tools.pixelate]` — pixelate/blur tool options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PixelateToolConfig {
    /// Pixel block size in pixels.
    pub size: u32,
}

impl Default for PixelateToolConfig {
    fn default() -> Self {
        Self { size: 2 }
    }
}

/// `[tools.rectangle]` — rectangle tool options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RectangleToolConfig {
    /// Corner radius in pixels.
    pub corner_radius: u32,
}

impl Default for RectangleToolConfig {
    fn default() -> Self {
        Self { corner_radius: 1 }
    }
}

/// `[tools.counter]` — step counter tool options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CounterToolConfig {
    /// Starting counter value.
    pub size: u32,
    /// Draw an outline around counter badges.
    pub outline: bool,
}

impl Default for CounterToolConfig {
    fn default() -> Self {
        Self {
            size: 1,
            outline: true,
        }
    }
}

/// `[tools]` — per-tool option groups.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    /// Arrow tool options.
    pub arrow: ArrowToolConfig,
    /// Marker tool options.
    pub marker: MarkerToolConfig,
    /// Pixelate tool options.
    pub pixelate: PixelateToolConfig,
    /// Rectangle tool options.
    pub rectangle: RectangleToolConfig,
    /// Counter tool options.
    pub counter: CounterToolConfig,
}

/// `[pin]` — floating pinned-image behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PinConfig {
    /// Minimum window size (width and height) for pins, in pixels.
    pub min_size: u32,
}

impl Default for PinConfig {
    fn default() -> Self {
        Self { min_size: 100 }
    }
}

/// `[upload]` — image upload provider settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UploadConfig {
    /// Upload provider identifier (e.g. `imgur`).
    pub provider: String,
    /// Provider API client id; empty means unconfigured.
    pub client_id: String,
    /// Upload without asking for confirmation.
    pub without_confirmation: bool,
    /// Copy the upload URL to the clipboard when done.
    pub copy_url: bool,
    /// Maximum number of upload history entries kept.
    pub history_max: u32,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            provider: "imgur".to_owned(),
            client_id: String::new(),
            without_confirmation: false,
            copy_url: true,
            history_max: 25,
        }
    }
}

/// `[ui]` — application theming and toolbar layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Accent color (`#RRGGBB`), seeded from the design-token palette.
    pub accent_color: String,
    /// Contrast color (`#RRGGBB`), seeded from the design-token palette.
    pub contrast_color: String,
    /// Background dim opacity (0-255).
    pub dim_opacity: u8,
    /// Toolbar button order, by tool identifier.
    pub toolbar_buttons: Vec<String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        let palette = Palette::default();
        Self {
            accent_color: palette.accent,
            contrast_color: palette.contrast,
            dim_opacity: palette.dim_opacity,
            toolbar_buttons: default_toolbar_buttons(),
        }
    }
}

/// Default toolbar layout.
fn default_toolbar_buttons() -> Vec<String> {
    [
        "arrow",
        "rectangle",
        "circle",
        "marker",
        "text",
        "pixelate",
        "counter",
        "copy",
        "save",
        "pin",
        "upload",
        "undo",
    ]
    .iter()
    .map(|id| (*id).to_owned())
    .collect()
}

/// `[daemon]` — background daemon behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Show a system tray icon while the daemon runs.
    pub tray: bool,
}

/// Root configuration document.
///
/// Field order matters for TOML serialization: the scalar `config_version`
/// must precede all table-valued groups to keep output valid and
/// byte-stable across write/reload cycles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Schema version used for forward migration.
    pub config_version: u32,
    /// Capture behavior and persisted state.
    pub capture: CaptureConfig,
    /// Save path, naming, and post-capture actions.
    pub save: SaveConfig,
    /// Annotation editor defaults.
    pub editor: EditorConfig,
    /// Per-tool option groups.
    pub tools: ToolsConfig,
    /// Pin behavior.
    pub pin: PinConfig,
    /// Upload provider settings.
    pub upload: UploadConfig,
    /// UI theming and toolbar layout.
    pub ui: UiConfig,
    /// Daemon behavior.
    pub daemon: DaemonConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: CONFIG_VERSION,
            capture: CaptureConfig::default(),
            save: SaveConfig::default(),
            editor: EditorConfig::default(),
            tools: ToolsConfig::default(),
            pin: PinConfig::default(),
            upload: UploadConfig::default(),
            ui: UiConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
}

/// Signature of a single migration step: rewrites a config table from
/// version `N` into the shape expected by version `N + 1`.
type MigrationFn = fn(&mut toml::Table) -> Result<(), String>;

/// Ordered migration registry, keyed by the version each step migrates *from*.
///
/// Adding a new schema version means appending `(old_version, step_fn)` and
/// bumping [`CONFIG_VERSION`]; existing on-disk configs are upgraded
/// automatically on load.
const MIGRATIONS: &[(u32, MigrationFn)] = &[(0, migrate_v0_to_v1), (1, migrate_v1_to_v2)];

/// v0 (unversioned legacy files) -> v1: introduce `config_version`.
///
/// Legacy files already used the grouped layout, so no keys are rewritten;
/// the version stamp itself is applied by the migration driver.
#[allow(clippy::unnecessary_wraps)] // Uniform fallible signature for the registry.
fn migrate_v0_to_v1(_table: &mut toml::Table) -> Result<(), String> {
    Ok(())
}

/// v1 -> v2: rename `[save] filename_template` to `filename_pattern`.
///
/// If both keys exist, the explicit `filename_pattern` wins and the stale
/// `filename_template` is dropped.
#[allow(clippy::unnecessary_wraps)] // Uniform fallible signature for the registry.
fn migrate_v1_to_v2(table: &mut toml::Table) -> Result<(), String> {
    if let Some(save) = table.get_mut("save").and_then(toml::Value::as_table_mut)
        && let Some(pattern) = save.remove("filename_template")
        && !save.contains_key("filename_pattern")
    {
        save.insert("filename_pattern".to_owned(), pattern);
    }
    Ok(())
}

impl Config {
    /// Parse a config from TOML text, migrating older schema versions.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the text is not valid TOML matching the
    /// schema, if `config_version` is malformed, if it names a version newer
    /// than [`CONFIG_VERSION`], or if a migration step fails.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let mut value: toml::Value = toml::from_str(text)?;
        let table = value.as_table_mut().ok_or(ConfigError::NotATable)?;

        let mut version = match table.get(VERSION_KEY) {
            None => 0, // Unversioned legacy file.
            Some(raw) => {
                let integer = raw.as_integer().ok_or(ConfigError::InvalidVersion)?;
                u32::try_from(integer).map_err(|_| ConfigError::InvalidVersion)?
            }
        };

        if version > CONFIG_VERSION {
            return Err(ConfigError::FutureVersion {
                found: version,
                supported: CONFIG_VERSION,
            });
        }

        while version < CONFIG_VERSION {
            let (_, migrate) = MIGRATIONS
                .iter()
                .find(|(from, _)| *from == version)
                .ok_or_else(|| ConfigError::Migration {
                    from: version,
                    reason: "no migration registered for this version".to_owned(),
                })?;
            migrate(table).map_err(|reason| ConfigError::Migration {
                from: version,
                reason,
            })?;
            version += 1;
            table.insert(
                VERSION_KEY.to_owned(),
                toml::Value::Integer(i64::from(version)),
            );
        }

        let config: Self = toml::Value::Table(std::mem::take(table)).try_into()?;
        Ok(config)
    }

    /// Serialize the config to TOML text.
    ///
    /// The output is deterministic: writing and reloading a config produces
    /// byte-identical TOML.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Serialize`] if serialization fails.
    pub fn to_toml_string(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string(self)?)
    }

    /// Load the config from a TOML file, migrating older schema versions.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Io`] if the file cannot be read, plus any
    /// error from [`Config::from_toml_str`].
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path)?;
        Self::from_toml_str(&text)
    }

    /// Save the config to a TOML file, overwriting existing content.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Io`] if the file cannot be written, plus any
    /// error from [`Config::to_toml_string`].
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        fs::write(path, self.to_toml_string()?)?;
        Ok(())
    }
}

impl FromStr for Config {
    type Err = ConfigError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::from_toml_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_write_reload_is_byte_stable() -> Result<(), ConfigError> {
        let config = Config::default();
        let first = config.to_toml_string()?;
        let reloaded = Config::from_toml_str(&first)?;
        let second = reloaded.to_toml_string()?;
        assert_eq!(first, second, "TOML output must be byte-stable");
        assert_eq!(config, reloaded);
        Ok(())
    }

    #[test]
    fn default_config_file_roundtrip_is_byte_stable() -> Result<(), ConfigError> {
        let path = std::env::temp_dir().join(format!(
            "flowshot-core-config-test-{}.toml",
            std::process::id()
        ));
        let config = Config::default();
        config.save(&path)?;
        let first_bytes = fs::read(&path)?;
        let reloaded = Config::load(&path)?;
        assert_eq!(config, reloaded);
        reloaded.save(&path)?;
        let second_bytes = fs::read(&path)?;
        assert_eq!(first_bytes, second_bytes, "file bytes must be stable");
        fs::remove_file(&path)?;
        Ok(())
    }

    #[test]
    fn defaults_match_spec() {
        let config = Config::default();
        assert_eq!(config.config_version, CONFIG_VERSION);

        // [capture]
        assert!(!config.capture.hide_cursor);
        assert!(config.capture.save_last_region);
        assert_eq!(config.capture.last_region, None);

        // [save]
        assert_eq!(config.save.path, "");
        assert!(!config.save.path_fixed);
        assert_eq!(config.save.extension, "png");
        assert_eq!(config.save.filename_pattern, "%F_%H-%M");
        assert_eq!(config.save.jpeg_quality, 75);
        assert_eq!(config.save.clipboard_format, ClipboardFormat::Png);
        assert_eq!(config.save.actions, vec![SaveAction::Copy]);

        // [editor]
        assert_eq!(config.editor.draw_thickness, 3);
        assert_eq!(config.editor.font_size, 8);
        assert!(!config.editor.magnifier);
        assert_eq!(config.editor.magnifier_shape, MagnifierShape::Square);
        assert_eq!(config.editor.hud_position, 4);
        assert!(!config.editor.grid);
        assert_eq!(config.editor.undo_limit, 100);
        assert!(!config.editor.double_click_copies);
        assert!(config.editor.side_panel);
        assert!(!config.editor.color_palette.is_empty());

        // [tools.*]
        assert_eq!(config.tools.arrow.style, ArrowStyle::Straight);
        assert!(!config.tools.arrow.reverse);
        assert_eq!(config.tools.marker.size, 5);
        assert_eq!(config.tools.pixelate.size, 2);
        assert_eq!(config.tools.rectangle.corner_radius, 1);
        assert_eq!(config.tools.counter.size, 1);
        assert!(config.tools.counter.outline);

        // [pin]
        assert_eq!(config.pin.min_size, 100);

        // [upload]
        assert_eq!(config.upload.provider, "imgur");
        assert_eq!(config.upload.client_id, "");
        assert!(!config.upload.without_confirmation);
        assert!(config.upload.copy_url);
        assert_eq!(config.upload.history_max, 25);

        // [ui] — seeded from design tokens.
        let palette = Palette::default();
        assert_eq!(config.ui.accent_color, palette.accent);
        assert_eq!(config.ui.contrast_color, palette.contrast);
        assert_eq!(config.ui.dim_opacity, 190);
        assert!(!config.ui.toolbar_buttons.is_empty());

        // [daemon]
        assert!(!config.daemon.tray);
    }

    #[test]
    fn unversioned_legacy_config_migrates_to_current() -> Result<(), ConfigError> {
        let legacy = r#"
            [save]
            path = "/home/user/Pictures"
        "#;
        let config = Config::from_toml_str(legacy)?;
        assert_eq!(config.config_version, CONFIG_VERSION);
        assert_eq!(config.save.path, "/home/user/Pictures");
        // Untouched keys fall back to current defaults.
        assert_eq!(config.save.extension, "png");
        assert_eq!(config.save.filename_pattern, "%F_%H-%M");
        Ok(())
    }

    #[test]
    fn v1_filename_template_is_renamed_on_migration() -> Result<(), ConfigError> {
        let v1 = r#"
            config_version = 1

            [save]
            filename_template = "%Y-%m-%d_%H%M%S"
        "#;
        let config = Config::from_toml_str(v1)?;
        assert_eq!(config.config_version, CONFIG_VERSION);
        assert_eq!(config.save.filename_pattern, "%Y-%m-%d_%H%M%S");
        Ok(())
    }

    #[test]
    fn v1_migration_does_not_clobber_existing_filename_pattern() -> Result<(), ConfigError> {
        let v1 = r#"
            config_version = 1

            [save]
            filename_pattern = "kept"
            filename_template = "ignored"
        "#;
        let config = Config::from_toml_str(v1)?;
        // The rename only fills `filename_pattern` when it is absent; an
        // explicit new key wins and the stale template is dropped.
        assert_eq!(config.save.filename_pattern, "kept");
        Ok(())
    }

    #[test]
    fn future_version_is_rejected() {
        let result = Config::from_toml_str("config_version = 99");
        assert!(matches!(
            result,
            Err(ConfigError::FutureVersion {
                found: 99,
                supported: CONFIG_VERSION
            })
        ));
    }

    #[test]
    fn malformed_version_is_rejected() {
        assert!(matches!(
            Config::from_toml_str("config_version = \"abc\""),
            Err(ConfigError::InvalidVersion)
        ));
        assert!(matches!(
            Config::from_toml_str("config_version = -1"),
            Err(ConfigError::InvalidVersion)
        ));
    }

    #[test]
    fn partial_config_fills_group_defaults() -> Result<(), ConfigError> {
        let partial = r"
            config_version = 2

            [editor]
            draw_thickness = 7

            [tools.marker]
            size = 12
        ";
        let config = Config::from_toml_str(partial)?;
        assert_eq!(config.editor.draw_thickness, 7);
        assert_eq!(config.tools.marker.size, 12);
        // Sibling keys and untouched groups keep their defaults.
        assert_eq!(config.editor.font_size, 8);
        assert_eq!(config.tools.pixelate.size, 2);
        assert_eq!(config.save, SaveConfig::default());
        assert_eq!(config.ui, UiConfig::default());
        Ok(())
    }

    #[test]
    fn enums_parse_from_lowercase_toml_strings() -> Result<(), ConfigError> {
        let text = r#"
            config_version = 2

            [save]
            clipboard_format = "jpeg"
            actions = ["copy", "save", "pin", "upload"]

            [editor]
            magnifier_shape = "circle"

            [tools.arrow]
            style = "curved"
        "#;
        let config = Config::from_toml_str(text)?;
        assert_eq!(config.save.clipboard_format, ClipboardFormat::Jpeg);
        assert_eq!(
            config.save.actions,
            vec![
                SaveAction::Copy,
                SaveAction::Save,
                SaveAction::Pin,
                SaveAction::Upload
            ]
        );
        assert_eq!(config.editor.magnifier_shape, MagnifierShape::Circle);
        assert_eq!(config.tools.arrow.style, ArrowStyle::Curved);
        Ok(())
    }

    #[test]
    fn last_region_persists_across_roundtrip() -> Result<(), ConfigError> {
        let mut config = Config::default();
        config.capture.last_region = Some(Region {
            x: -10,
            y: 42,
            width: 1920,
            height: 1080,
        });
        let text = config.to_toml_string()?;
        let reloaded = Config::from_toml_str(&text)?;
        assert_eq!(reloaded.capture.last_region, config.capture.last_region);
        assert_eq!(reloaded.to_toml_string()?, text);
        Ok(())
    }

    #[test]
    fn absent_last_region_is_omitted_from_output() -> Result<(), ConfigError> {
        let text = Config::default().to_toml_string()?;
        let value: toml::Value = toml::from_str(&text)?;
        let capture = value
            .get("capture")
            .and_then(toml::Value::as_table)
            .ok_or(ConfigError::NotATable)?;
        assert!(!capture.contains_key("last_region"));
        assert!(capture.contains_key("save_last_region"));
        Ok(())
    }

    #[test]
    fn from_str_trait_delegates_to_toml_parser() -> Result<(), ConfigError> {
        let via_trait: Config = "config_version = 2".parse()?;
        assert_eq!(via_trait, Config::default());
        Ok(())
    }
}

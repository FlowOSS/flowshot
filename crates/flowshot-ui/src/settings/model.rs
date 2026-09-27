//! The settings model: typed edit state over the todo-2 [`Config`].
//!
//! One model instance backs the whole window: the tabs mutate it in place,
//! [`SettingsModel::validate`] gates Apply, and the window layer persists
//! `config()` through [`Config::save`] (migration-safe: the load path
//! already migrated the file to [`CONFIG_VERSION`], and the write stamps it
//! back - the todo-36 "Apply repairs a corrupt file" contract). A corrupt
//! load degrades to defaults plus a banner instead of failing (the CLI's
//! resilient-load rule).
//!
//! Shortcut state rides along as the editor's own [`ToolShortcuts`] (the
//! todo-25 rebind seams are the write path); persistence of rebinds waits
//! on the core `[shortcuts]` group (issues.md 2026-09-26 - core is not
//! editable from this crate), so rebinds are session state handed to the
//! binary layer through [`SettingsModel::shortcuts`].

use flowshot_core::config::Config;

use crate::editor::{ToolKind, ToolShortcuts};

use super::strings;
use super::theme::{ThemeMode, parse_hex_rgb};

/// The window's four tabs (F12 parity).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    /// Every config key, grouped.
    #[default]
    General,
    /// Theming, toolbar order, palette.
    Interface,
    /// Filename pattern + live preview.
    Filename,
    /// Per-action shortcut recorder.
    Shortcuts,
}

impl Tab {
    /// Every tab in display order.
    pub const ALL: [Self; 4] = [
        Self::General,
        Self::Interface,
        Self::Filename,
        Self::Shortcuts,
    ];

    /// The tab bar label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::General => strings::TAB_GENERAL,
            Self::Interface => strings::TAB_INTERFACE,
            Self::Filename => strings::TAB_FILENAME,
            Self::Shortcuts => strings::TAB_SHORTCUTS,
        }
    }
}

/// Theme selection (Interface tab). `System` defers to the preference the
/// binary layer resolved (ashpd Settings portal - purity gate keeps the
/// portal query out of this crate).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Always dark.
    Dark,
    /// Always light.
    Light,
    /// Follow the system preference injected by the binary layer.
    #[default]
    System,
}

impl ThemeChoice {
    /// Resolves the selection against the injected system preference.
    #[must_use]
    pub const fn resolve(self, system: ThemeMode) -> ThemeMode {
        match self {
            Self::Dark => ThemeMode::Dark,
            Self::Light => ThemeMode::Light,
            Self::System => system,
        }
    }
}

/// The window's transient message bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    /// The loaded file was corrupt; defaults are shown.
    CorruptConfig,
    /// Apply failed to write the file (carries the typed error's message).
    SaveFailed(String),
    /// Apply was blocked by validation issues.
    Validation,
}

/// A validation issue, tied to a field label from [`strings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldIssue {
    /// The offending field's label constant.
    pub label: &'static str,
    /// The rule that failed.
    pub message: &'static str,
}

impl std::fmt::Display for FieldIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.label, self.message)
    }
}

/// The shortcut slot the recorder is arming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderTarget {
    /// A tool activation key.
    Tool(ToolKind),
    /// Undo (Ctrl+…).
    Undo,
    /// Redo (Ctrl+Shift+…).
    Redo,
    /// Raise layer (z-order).
    Raise,
    /// Lower layer (z-order).
    Lower,
}

/// The highest undo steps Flameshot's handler accepts (F27: limit 100,
/// range 0..999).
pub const UNDO_LIMIT_MAX: u32 = 999;

/// The inclusive JPEG quality range.
pub const JPEG_QUALITY_RANGE: std::ops::RangeInclusive<u8> = 1..=100;

/// The edit state of the settings window.
#[derive(Debug, Clone)]
pub struct SettingsModel {
    config: Config,
    shortcuts: ToolShortcuts,
    theme: ThemeChoice,
    banner: Option<Banner>,
    dirty: bool,
    recorder: Option<RecorderTarget>,
    active_tab: Tab,
}

impl Default for SettingsModel {
    fn default() -> Self {
        Self::from_config(Config::default())
    }
}

impl SettingsModel {
    /// The model for an already-loaded config.
    #[must_use]
    pub fn from_config(config: Config) -> Self {
        Self {
            config,
            shortcuts: ToolShortcuts::default(),
            theme: ThemeChoice::default(),
            banner: None,
            dirty: false,
            recorder: None,
            active_tab: Tab::default(),
        }
    }

    /// Loads from TOML text; a corrupt document degrades to defaults with
    /// [`Banner::CorruptConfig`] (the todo-36 failure-path scenario) instead
    /// of failing - the same resilient-load rule the CLI and daemon follow.
    #[must_use]
    pub fn from_toml_str(text: &str) -> Self {
        if let Ok(config) = Config::from_toml_str(text) {
            Self::from_config(config)
        } else {
            let mut model = Self::from_config(Config::default());
            model.banner = Some(Banner::CorruptConfig);
            model
        }
    }

    /// The config under edit (tabs mutate through [`Self::config_mut`]).
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Mutable config access; callers set [`Self::mark_dirty`] when a widget
    /// reported a change.
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// The editor shortcut map under edit (todo-25 seam consumer).
    #[must_use]
    pub const fn shortcuts(&self) -> &ToolShortcuts {
        &self.shortcuts
    }

    /// Mutable shortcut-map access (the recorder's rebind seams).
    pub fn shortcuts_mut(&mut self) -> &mut ToolShortcuts {
        &mut self.shortcuts
    }

    /// The theme selection.
    #[must_use]
    pub const fn theme(&self) -> ThemeChoice {
        self.theme
    }

    /// Sets the theme selection.
    pub fn set_theme(&mut self, theme: ThemeChoice) {
        self.theme = theme;
        self.dirty = true;
    }

    /// The visible banner, when any.
    #[must_use]
    pub const fn banner(&self) -> Option<&Banner> {
        self.banner.as_ref()
    }

    /// Replaces the banner.
    pub fn set_banner(&mut self, banner: Option<Banner>) {
        self.banner = banner;
    }

    /// Whether edits are pending (drives the Apply affordance).
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Marks pending edits.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Clears the pending-edits flag (after a successful persist).
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// The selected tab.
    #[must_use]
    pub const fn active_tab(&self) -> Tab {
        self.active_tab
    }

    /// Mutable tab selection (the tab bar's `selectable_value` target).
    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.active_tab
    }

    /// Selects a tab.
    pub fn set_active_tab(&mut self, tab: Tab) {
        self.active_tab = tab;
    }

    /// The slot awaiting a key press, when recording.
    #[must_use]
    pub const fn recorder(&self) -> Option<RecorderTarget> {
        self.recorder
    }

    /// Arms the recorder for `target`.
    pub fn start_recording(&mut self, target: RecorderTarget) {
        self.recorder = Some(target);
    }

    /// Cancels an armed recorder (Esc).
    pub fn cancel_recording(&mut self) {
        self.recorder = None;
    }

    /// Feeds a captured key to the armed recorder through the todo-25
    /// rebind seams; returns true when a slot consumed it. Keys without a
    /// physical `KeyCode` (Colon/Pipe/Questionmark) cannot back a binding
    /// and are rejected without disarming the recorder.
    pub fn capture_recorder_key(&mut self, key: egui::Key) -> bool {
        let Some(target) = self.recorder else {
            return false;
        };
        let Some(code) = super::keymap::code_from_egui_key(key) else {
            return false;
        };
        match target {
            RecorderTarget::Tool(kind) => self.shortcuts.rebind(kind, Some(code)),
            RecorderTarget::Undo => {
                let redo = self.shortcuts.redo_key();
                self.shortcuts.rebind_undo_redo(code, redo);
            }
            RecorderTarget::Redo => {
                let undo = self.shortcuts.undo_key();
                self.shortcuts.rebind_undo_redo(undo, code);
            }
            RecorderTarget::Raise => {
                let lower = self.shortcuts.z_keys().1;
                self.shortcuts.rebind_z_order(Some(code), lower);
            }
            RecorderTarget::Lower => {
                let raise = self.shortcuts.z_keys().0;
                self.shortcuts.rebind_z_order(raise, Some(code));
            }
        }
        self.recorder = None;
        self.dirty = true;
        true
    }

    /// Restores factory defaults, preserving `config_version` (plan todo 36:
    /// "Reset = defaults preserving `config_version`").
    pub fn reset(&mut self) {
        self.config = Config {
            config_version: self.config.config_version,
            ..Config::default()
        };
        self.shortcuts = ToolShortcuts::default();
        self.banner = None;
        self.recorder = None;
        self.dirty = true;
    }

    /// Every validation issue in the current edit state (empty = Apply is
    /// allowed). Ranges per F27/todo-2: undo limit 0..=999, JPEG quality
    /// 1..=100, colors `#RRGGBB`.
    #[must_use]
    pub fn validate(&self) -> Vec<FieldIssue> {
        let mut issues = Vec::new();
        if self.config.editor.undo_limit > UNDO_LIMIT_MAX {
            issues.push(FieldIssue {
                label: strings::FIELD_UNDO_LIMIT,
                message: strings::ISSUE_UNDO_LIMIT,
            });
        }
        if !JPEG_QUALITY_RANGE.contains(&self.config.save.jpeg_quality) {
            issues.push(FieldIssue {
                label: strings::FIELD_JPEG_QUALITY,
                message: strings::ISSUE_JPEG_QUALITY,
            });
        }
        for (label, color) in [
            (strings::FIELD_DRAW_COLOR, &self.config.editor.draw_color),
            (strings::FIELD_ACCENT_COLOR, &self.config.ui.accent_color),
            (
                strings::FIELD_CONTRAST_COLOR,
                &self.config.ui.contrast_color,
            ),
        ] {
            if parse_hex_rgb(color).is_none() {
                issues.push(FieldIssue {
                    label,
                    message: strings::ISSUE_COLOR_FORMAT,
                });
            }
        }
        issues.extend(
            self.config
                .editor
                .color_palette
                .iter()
                .filter(|color| parse_hex_rgb(color).is_none())
                .map(|_| FieldIssue {
                    label: strings::FIELD_COLOR_PALETTE,
                    message: strings::ISSUE_COLOR_FORMAT,
                }),
        );
        issues
    }

    /// Whether Apply is allowed.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }
}

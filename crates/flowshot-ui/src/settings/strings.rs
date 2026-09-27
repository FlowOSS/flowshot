//! Settings-surface user-facing strings (Amendment #4 item 8: message-key
//! constants, no inline literals in logic). English-only v1; the i18n
//! catalog replaces the values behind these keys on the roadmap.

/// The settings window title (portable; the `flowshot-settings` `app_id` is
/// applied by the binary layer through the window-customizer seam - the
/// `pins::WindowCustomizer` precedent, purity gate).
pub const WINDOW_TITLE: &str = "FlowShot Settings";

/// Tab: every config key, grouped.
pub const TAB_GENERAL: &str = "General";
/// Tab: theming, toolbar order, palette.
pub const TAB_INTERFACE: &str = "Interface";
/// Tab: filename pattern editor.
pub const TAB_FILENAME: &str = "Filename Editor";
/// Tab: shortcut recorder.
pub const TAB_SHORTCUTS: &str = "Shortcuts";

/// Commit edits: migration-safe TOML write + applied callback.
pub const BUTTON_APPLY: &str = "Apply";
/// Restore defaults (`config_version` preserved).
pub const BUTTON_RESET: &str = "Reset";
/// Open the save-path picker (binary-layer rfd seam).
pub const BUTTON_BROWSE: &str = "Browse…";
/// Close the window.
pub const BUTTON_CLOSE: &str = "Close";
/// Move a list row up.
pub const BUTTON_MOVE_UP: &str = "↑";
/// Move a list row down.
pub const BUTTON_MOVE_DOWN: &str = "↓";
/// Remove a list row / palette swatch.
pub const BUTTON_REMOVE: &str = "✕";
/// Add a palette swatch.
pub const BUTTON_ADD_SWATCH: &str = "Add swatch";
/// Add a toolbar button from the known vocabulary.
pub const BUTTON_ADD: &str = "Add";
/// Start recording a shortcut.
pub const BUTTON_RECORD: &str = "Record";
/// Unbind a shortcut slot.
pub const BUTTON_CLEAR: &str = "Clear";
/// Restore the default filename pattern.
pub const BUTTON_RESET_PATTERN: &str = "Reset to default";

/// Banner shown when the config file failed to parse (defaults displayed;
/// Apply repairs the file - the todo-36 failure-path QA scenario).
pub const BANNER_CORRUPT_CONFIG: &str =
    "The config file could not be parsed; defaults are shown. Apply to repair the file.";
/// Banner prefix when the Apply write failed.
pub const BANNER_SAVE_FAILED: &str = "Could not save the config file";
/// Banner when Apply was blocked by validation.
pub const BANNER_VALIDATION: &str = "Fix the highlighted issues before applying.";

/// Group header: `[capture]`.
pub const GROUP_CAPTURE: &str = "Capture";
/// Group header: `[save]`.
pub const GROUP_SAVE: &str = "Save";
/// Group header: `[editor]`.
pub const GROUP_EDITOR: &str = "Editor";
/// Group header: `[tools.*]`.
pub const GROUP_TOOLS: &str = "Tools";
/// Group header: `[tools.arrow]`.
pub const GROUP_TOOL_ARROW: &str = "Arrow";
/// Group header: `[tools.marker]`.
pub const GROUP_TOOL_MARKER: &str = "Marker";
/// Group header: `[tools.pixelate]`.
pub const GROUP_TOOL_PIXELATE: &str = "Pixelate";
/// Group header: `[tools.rectangle]`.
pub const GROUP_TOOL_RECTANGLE: &str = "Rectangle";
/// Group header: `[tools.counter]`.
pub const GROUP_TOOL_COUNTER: &str = "Counter";
/// Group header: `[pin]`.
pub const GROUP_PIN: &str = "Pin";
/// Group header: `[upload]`.
pub const GROUP_UPLOAD: &str = "Upload";
/// Group header: `[daemon]`.
pub const GROUP_DAEMON: &str = "Daemon";

/// `[capture].hide_cursor`.
pub const FIELD_HIDE_CURSOR: &str = "Hide mouse cursor in captures";
/// `[capture].save_last_region`.
pub const FIELD_SAVE_LAST_REGION: &str = "Remember the last selected region";
/// `[save].path`.
pub const FIELD_SAVE_PATH: &str = "Save path (empty = Pictures)";
/// `[save].path_fixed`.
pub const FIELD_PATH_FIXED: &str = "Always save to this path without prompting";
/// `[save].extension`.
pub const FIELD_EXTENSION: &str = "Default file extension";
/// `[save].jpeg_quality`.
pub const FIELD_JPEG_QUALITY: &str = "JPEG quality (1-100)";
/// `[save].clipboard_format`.
pub const FIELD_CLIPBOARD_FORMAT: &str = "Clipboard format";
/// `[save].actions`.
pub const FIELD_SAVE_ACTIONS: &str = "Actions after capture";
/// `[editor].draw_color`.
pub const FIELD_DRAW_COLOR: &str = "Draw color";
/// `[editor].draw_thickness`.
pub const FIELD_DRAW_THICKNESS: &str = "Draw thickness";
/// `[editor].font_family`.
pub const FIELD_FONT_FAMILY: &str = "Text font family";
/// `[editor].font_size`.
pub const FIELD_FONT_SIZE: &str = "Text font size";
/// `[editor].magnifier`.
pub const FIELD_MAGNIFIER: &str = "Pixel magnifier";
/// `[editor].magnifier_shape`.
pub const FIELD_MAGNIFIER_SHAPE: &str = "Magnifier shape";
/// `[editor].hud_position`.
pub const FIELD_HUD_POSITION: &str = "Geometry HUD corner";
/// `[editor].hud_hide_time`.
pub const FIELD_HUD_HIDE_TIME: &str = "HUD auto-hide delay (ms, 0 = never)";
/// `[editor].grid`.
pub const FIELD_GRID: &str = "Snapping grid";
/// `[editor].undo_limit`.
pub const FIELD_UNDO_LIMIT: &str = "Undo steps (0-999)";
/// `[editor].double_click_copies`.
pub const FIELD_DOUBLE_CLICK_COPIES: &str = "Double-click copies the selection";
/// `[editor].side_panel`.
pub const FIELD_SIDE_PANEL: &str = "Side panel (Space toggles in the editor)";
/// `[tools.arrow].style`.
pub const FIELD_ARROW_STYLE: &str = "Shaft style";
/// `[tools.arrow].reverse`.
pub const FIELD_ARROW_REVERSE: &str = "Head at the start point";
/// `[tools.marker].size`.
pub const FIELD_MARKER_SIZE: &str = "Stroke width";
/// `[tools.pixelate].size`.
pub const FIELD_PIXELATE_SIZE: &str = "Block size";
/// `[tools.rectangle].corner_radius`.
pub const FIELD_CORNER_RADIUS: &str = "Corner radius";
/// `[tools.counter].size`.
pub const FIELD_COUNTER_START: &str = "Starting value";
/// `[tools.counter].outline`.
pub const FIELD_COUNTER_OUTLINE: &str = "Outline badges";
/// `[pin].min_size`.
pub const FIELD_PIN_MIN_SIZE: &str = "Minimum pin size (px)";
/// `[upload].provider`.
pub const FIELD_UPLOAD_PROVIDER: &str = "Provider";
/// `[upload].client_id`.
pub const FIELD_UPLOAD_CLIENT_ID: &str = "Client ID";
/// `[upload].without_confirmation`.
pub const FIELD_UPLOAD_NO_CONFIRM: &str = "Upload without confirmation";
/// `[upload].copy_url`.
pub const FIELD_UPLOAD_COPY_URL: &str = "Copy the URL after upload";
/// `[upload].history_max`.
pub const FIELD_UPLOAD_HISTORY_MAX: &str = "History entries kept";
/// `[daemon].tray`.
pub const FIELD_TRAY: &str = "System tray icon";
/// `[daemon].notifications`.
pub const FIELD_NOTIFICATIONS: &str = "Desktop notifications";
/// `[daemon].startup_launch`.
pub const FIELD_STARTUP_LAUNCH: &str = "Launch at system startup";
/// `[ui].accent_color`.
pub const FIELD_ACCENT_COLOR: &str = "Accent color";
/// `[ui].contrast_color`.
pub const FIELD_CONTRAST_COLOR: &str = "Contrast color";
/// `[ui].dim_opacity`.
pub const FIELD_DIM_OPACITY: &str = "Overlay dim opacity (0-255)";
/// `[ui].toolbar_buttons`.
pub const FIELD_TOOLBAR_BUTTONS: &str = "Toolbar button order";
/// `[editor].color_palette`.
pub const FIELD_COLOR_PALETTE: &str = "Color palette swatches";
/// Theme selector label.
pub const FIELD_THEME: &str = "Theme";
/// `[save].filename_pattern`.
pub const FIELD_FILENAME_PATTERN: &str = "Filename pattern";

/// Hint under an empty `[upload].client_id` (the todo-31 CLI gate).
pub const HINT_UPLOAD_UNCONFIGURED: &str =
    "Upload is disabled while the client ID is empty (`flowshot upload` exits with a hint).";
/// Hint under the filename pattern field.
pub const HINT_FILENAME_PATTERN: &str = "strftime placeholders: %F = date, %H-%M = time.";
/// Live-preview row label.
pub const LABEL_PREVIEW: &str = "Preview";
/// Hint while a shortcut slot awaits a key press.
pub const HINT_RECORDING: &str = "Press a key… (Esc cancels)";
/// A shortcut slot with no binding.
pub const LABEL_UNBOUND: &str = "Unbound";
/// Undo slot label.
pub const SHORTCUT_UNDO: &str = "Undo (Ctrl+…)";
/// Redo slot label.
pub const SHORTCUT_REDO: &str = "Redo (Ctrl+Shift+…)";
/// Raise-layer slot label.
pub const SHORTCUT_RAISE: &str = "Raise layer";
/// Lower-layer slot label.
pub const SHORTCUT_LOWER: &str = "Lower layer";
/// Note: global capture shortcuts are daemon-owned.
pub const HINT_GLOBAL_SHORTCUTS: &str = "Global capture shortcuts (Print…) are registered by the daemon via the shortcuts portal; \
     editor shortcuts below apply to the annotation editor.";

/// Validation: `[editor].undo_limit` out of 0..=999.
pub const ISSUE_UNDO_LIMIT: &str = "Undo steps must be within 0-999.";
/// Validation: `[save].jpeg_quality` out of 1..=100.
pub const ISSUE_JPEG_QUALITY: &str = "JPEG quality must be within 1-100.";
/// Validation: a color string is not `#RRGGBB`.
pub const ISSUE_COLOR_FORMAT: &str = "Colors must be #RRGGBB hex values.";

/// Enum label: PNG clipboard format.
pub const ENUM_PNG: &str = "png";
/// Enum label: JPEG clipboard format.
pub const ENUM_JPEG: &str = "jpeg";
/// Enum label: square magnifier.
pub const ENUM_SQUARE: &str = "square";
/// Enum label: circular magnifier.
pub const ENUM_CIRCLE: &str = "circle";
/// Enum label: straight arrow shaft.
pub const ENUM_STRAIGHT: &str = "straight";
/// Enum label: curved arrow shaft.
pub const ENUM_CURVED: &str = "curved";
/// Enum label: HUD top-left.
pub const ENUM_HUD_TOP_LEFT: &str = "Top-left";
/// Enum label: HUD top-right.
pub const ENUM_HUD_TOP_RIGHT: &str = "Top-right";
/// Enum label: HUD bottom-left.
pub const ENUM_HUD_BOTTOM_LEFT: &str = "Bottom-left";
/// Enum label: HUD bottom-right.
pub const ENUM_HUD_BOTTOM_RIGHT: &str = "Bottom-right";
/// Enum label: dark theme.
pub const ENUM_DARK: &str = "Dark";
/// Enum label: light theme.
pub const ENUM_LIGHT: &str = "Light";
/// Enum label: theme follows the system portal preference.
pub const ENUM_SYSTEM: &str = "Follow system";

/// Save-action label: copy to clipboard.
pub const ACTION_COPY: &str = "Copy to clipboard";
/// Save-action label: save to disk.
pub const ACTION_SAVE: &str = "Save to disk";
/// Save-action label: open as pin.
pub const ACTION_PIN: &str = "Pin";
/// Save-action label: upload.
pub const ACTION_UPLOAD: &str = "Upload";
/// Save-action label: copy the saved file's path.
pub const ACTION_COPY_PATH: &str = "Copy file path";
/// Save-action label: success toast.
pub const ACTION_NOTIFY: &str = "Notify";
/// Save-action label: open with the default application.
pub const ACTION_OPEN_WITH: &str = "Open with…";

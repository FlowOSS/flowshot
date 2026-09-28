//! Settings-surface unit tests: model round-trips, validation, reset,
//! recorder seams, theme projection, keymap consistency, and the filename
//! preview.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::cast_possible_truncation
)]

use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
use egui::Key;
use flowshot_core::config::{CONFIG_VERSION, Config, SaveConfig};
use flowshot_core::tokens::DesignTokens;
use winit::keyboard::KeyCode;

use super::model::{Banner, RecorderTarget, SettingsModel, Tab, ThemeChoice};
use super::tabs::FrameAction;
use crate::egui_host::keymap;
use crate::egui_host::theme::{self, ThemeMode, parse_hex_rgb};

fn fixed_time() -> chrono::DateTime<Local> {
    let date = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
    let time = NaiveTime::from_hms_opt(14, 30, 45).unwrap();
    Local
        .from_local_datetime(&date.and_time(time))
        .single()
        .unwrap_or_else(Local::now)
}

#[test]
fn default_model_config_roundtrips_byte_stable() {
    // Given: a fresh model over the default config
    let model = SettingsModel::default();
    // When: serialized, reloaded through the model loader, serialized again
    let first = model.config().to_toml_string().unwrap();
    let reloaded = SettingsModel::from_toml_str(&first);
    let second = reloaded.config().to_toml_string().unwrap();
    // Then: bytes are stable and the configs equal (todo-2 rule)
    assert_eq!(first, second);
    assert_eq!(model.config(), reloaded.config());
    assert_eq!(reloaded.banner(), None);
}

#[test]
fn edited_model_config_roundtrips_byte_stable() {
    // Given: a model with edits across several groups
    let mut model = SettingsModel::default();
    {
        let config = model.config_mut();
        config.ui.accent_color = "#FF0044".to_owned();
        config.editor.undo_limit = 42;
        config.save.jpeg_quality = 90;
        config.daemon.tray = true;
        config.upload.client_id = "abc123".to_owned();
        config.ui.toolbar_buttons = vec!["undo".to_owned(), "pencil".to_owned()];
    }
    // When: applied (serialized) and reloaded
    let first = model.config().to_toml_string().unwrap();
    let reloaded = SettingsModel::from_toml_str(&first);
    // Then: every edit survives byte-stably
    assert_eq!(reloaded.config().to_toml_string().unwrap(), first);
    assert_eq!(reloaded.config().ui.accent_color, "#FF0044");
    assert_eq!(reloaded.config().editor.undo_limit, 42);
    assert_eq!(reloaded.config().save.jpeg_quality, 90);
    assert!(reloaded.config().daemon.tray);
    assert_eq!(reloaded.config().upload.client_id, "abc123");
    assert_eq!(
        reloaded.config().ui.toolbar_buttons,
        vec!["undo".to_owned(), "pencil".to_owned()]
    );
}

#[test]
fn corrupt_toml_degrades_to_defaults_with_banner() {
    // Given: TOML that cannot parse
    // When: loaded through the model
    let model = SettingsModel::from_toml_str("config_version = ]]]not toml[[[");
    // Then: defaults are shown and the repair banner is up
    assert_eq!(model.banner(), Some(&Banner::CorruptConfig));
    assert_eq!(model.config(), &Config::default());
    assert!(model.is_valid());
}

#[test]
fn validation_rejects_out_of_range_numbers() {
    // Given: undo_limit above the F27 ceiling and jpeg_quality below 1
    let mut model = SettingsModel::default();
    model.config_mut().editor.undo_limit = 1000;
    model.config_mut().save.jpeg_quality = 0;
    // When: validated
    let issues = model.validate();
    // Then: both fields are flagged and Apply is blocked
    assert!(!model.is_valid());
    assert_eq!(issues.len(), 2);
    assert!(
        issues
            .iter()
            .any(|issue| issue.message == super::strings::ISSUE_UNDO_LIMIT)
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.message == super::strings::ISSUE_JPEG_QUALITY)
    );

    // Boundary values are accepted
    model.config_mut().editor.undo_limit = 999;
    model.config_mut().save.jpeg_quality = 1;
    assert!(model.is_valid());
    model.config_mut().save.jpeg_quality = 100;
    assert!(model.is_valid());
    model.config_mut().editor.undo_limit = 0;
    assert!(model.is_valid());
}

#[test]
fn validation_rejects_malformed_colors() {
    // Given: malformed hex in each color-bearing field
    let mut model = SettingsModel::default();
    model.config_mut().editor.draw_color = "red".to_owned();
    model.config_mut().ui.accent_color = "#12345".to_owned();
    model.config_mut().editor.color_palette = vec!["#000000".to_owned(), "oops".to_owned()];
    // When: validated
    let issues = model.validate();
    // Then: three color issues (draw, accent, one palette entry)
    assert_eq!(issues.len(), 3);
    assert!(!model.is_valid());
}

#[test]
fn reset_restores_defaults_preserving_config_version() {
    // Given: a heavily edited model
    let mut model = SettingsModel::default();
    model.config_mut().editor.undo_limit = 7;
    model.config_mut().ui.accent_color = "#010203".to_owned();
    model
        .shortcuts_mut()
        .rebind(crate::editor::ToolKind::Pencil, Some(KeyCode::KeyX));
    model.mark_clean();
    // When: reset
    model.reset();
    // Then: defaults are back, config_version preserved, state dirty
    let expected = Config {
        config_version: CONFIG_VERSION,
        ..Config::default()
    };
    assert_eq!(model.config(), &expected);
    assert_eq!(model.config().config_version, CONFIG_VERSION);
    assert!(model.is_dirty());
    assert_eq!(model.banner(), None);
    assert_eq!(
        model
            .shortcuts()
            .key_for_tool(crate::editor::ToolKind::Pencil),
        Some(KeyCode::KeyP)
    );
}

#[test]
fn recorder_binds_through_the_todo25_seams() {
    // Given: an armed tool recorder
    let mut model = SettingsModel::default();
    model.start_recording(RecorderTarget::Tool(crate::editor::ToolKind::Pencil));
    // When: X is captured
    assert!(model.capture_recorder_key(Key::X));
    // Then: the seam rebound P and dirty was marked
    assert_eq!(
        model.shortcuts().tool_for_key(KeyCode::KeyX),
        Some(crate::editor::ToolKind::Pencil)
    );
    assert_eq!(model.shortcuts().tool_for_key(KeyCode::KeyP), None);
    assert!(model.is_dirty());
    assert_eq!(model.recorder(), None);

    // Undo/redo slots keep their independent bindings
    let mut model = SettingsModel::default();
    model.start_recording(RecorderTarget::Undo);
    model.capture_recorder_key(Key::U);
    model.start_recording(RecorderTarget::Redo);
    model.capture_recorder_key(Key::Y);
    assert!(model.shortcuts().is_undo(KeyCode::KeyU));
    assert!(model.shortcuts().is_redo(KeyCode::KeyY));

    // Z-order slots consume rebind_z_order
    let mut model = SettingsModel::default();
    model.start_recording(RecorderTarget::Raise);
    model.capture_recorder_key(Key::K);
    model.start_recording(RecorderTarget::Lower);
    model.capture_recorder_key(Key::J);
    assert_eq!(
        model.shortcuts().z_for_key(KeyCode::KeyK),
        Some(crate::editor::ZOrderAction::Raise)
    );
    assert_eq!(
        model.shortcuts().z_for_key(KeyCode::KeyJ),
        Some(crate::editor::ZOrderAction::Lower)
    );
}

#[test]
fn recorder_rejects_unbindable_keys_without_disarming() {
    // Given: an armed recorder
    let mut model = SettingsModel::default();
    model.start_recording(RecorderTarget::Tool(crate::editor::ToolKind::Line));
    // When: a key with no physical KeyCode is captured
    assert!(!model.capture_recorder_key(Key::Colon));
    // Then: the recorder stays armed and nothing rebound
    assert!(model.recorder().is_some());
    assert_eq!(model.shortcuts().tool_for_key(KeyCode::Semicolon), None);
    // Cancelling disarms
    model.cancel_recording();
    assert_eq!(model.recorder(), None);
}

#[test]
fn keymap_roundtrips_and_merges_numpad() {
    assert_eq!(keymap::egui_key_from_code(KeyCode::KeyP), Some(Key::P));
    assert_eq!(keymap::code_from_egui_key(Key::P), Some(KeyCode::KeyP));
    assert_eq!(
        keymap::egui_key_from_code(KeyCode::NumpadEnter),
        Some(Key::Enter)
    );
    // First-match-wins: the reverse of a merged key is the main cluster
    assert_eq!(keymap::code_from_egui_key(Key::Enter), Some(KeyCode::Enter));
    assert_eq!(keymap::code_from_egui_key(Key::Num0), Some(KeyCode::Digit0));
    assert_eq!(keymap::code_from_egui_key(Key::Colon), None);
    assert_eq!(keymap::egui_key_from_code(KeyCode::LaunchApp1), None);
}

#[test]
fn theme_projects_tokens_and_config_into_visuals() {
    // Given: tokens + a config with a custom accent/contrast
    let tokens = DesignTokens::default();
    let ui = flowshot_core::config::UiConfig {
        accent_color: "#FF0000".to_owned(),
        contrast_color: "#000000".to_owned(),
        ..Default::default()
    };
    // When: the dark style is projected
    let style = theme::style(&tokens, &ui, ThemeMode::Dark);
    // Then: accent drives the selection stroke, contrast the panel fill
    assert_eq!(style.visuals.selection.stroke.color, egui::Color32::RED);
    assert_eq!(style.visuals.panel_fill, egui::Color32::BLACK);
    assert_eq!(style.visuals.hyperlink_color, egui::Color32::RED);
    // Radii come from the tokens
    assert_eq!(
        style.visuals.widgets.inactive.rounding,
        egui::Rounding::same(4.0)
    );

    // Light mode differs and keeps the accent
    let light = theme::style(&tokens, &ui, ThemeMode::Light);
    assert_ne!(light.visuals.panel_fill, style.visuals.panel_fill);
    assert_eq!(light.visuals.selection.stroke.color, egui::Color32::RED);
}

#[test]
fn theme_falls_back_to_tokens_on_malformed_config_colors() {
    // Given: a corrupt accent hex
    let tokens = DesignTokens::default();
    let ui = flowshot_core::config::UiConfig {
        accent_color: "not-a-color".to_owned(),
        ..Default::default()
    };
    // When: projected
    let style = theme::style(&tokens, &ui, ThemeMode::Dark);
    // Then: the token accent is used (no panic, no default-egui blue)
    let token_accent = parse_hex_rgb(&tokens.palette.accent).unwrap();
    assert_eq!(
        style.visuals.selection.stroke.color,
        egui::Color32::from_rgb(token_accent[0], token_accent[1], token_accent[2])
    );
}

#[test]
fn fonts_install_the_vendored_inter() {
    let fonts = theme::fonts();
    assert!(fonts.font_data.contains_key("Inter"));
    let proportional = fonts.families.get(&egui::FontFamily::Proportional).unwrap();
    assert_eq!(proportional.first().map(String::as_str), Some("Inter"));
}

#[test]
fn vendored_inter_is_a_real_truetype_file() {
    // Regression guard for the todo-19 corrupt-asset defect (issues.md
    // 2026-09-27: the vendored "TTFs" were GitHub 404 HTML pages, latent
    // until this surface became the first include_bytes consumer).
    let fonts = theme::fonts();
    let inter = fonts.font_data.get("Inter").unwrap();
    let magic: &[u8] = &inter.font[..4];
    assert!(
        magic == [0x00, 0x01, 0x00, 0x00]
            || magic == b"true"
            || magic == b"ttcf"
            || magic == b"OTTO",
        "Inter must be a TrueType/OpenType file, got magic {magic:02X?}"
    );
}

#[test]
fn hex_parser_is_strict() {
    assert_eq!(parse_hex_rgb("#6366F1"), Some([0x63, 0x66, 0xF1]));
    assert_eq!(parse_hex_rgb("#000000"), Some([0, 0, 0]));
    assert_eq!(parse_hex_rgb("6366F1"), None);
    assert_eq!(parse_hex_rgb("#12345"), None);
    assert_eq!(parse_hex_rgb("#1234567"), None);
    assert_eq!(parse_hex_rgb("#GGGGGG"), None);
    assert_eq!(parse_hex_rgb(""), None);
}

#[test]
fn filename_preview_expands_and_sanitizes() {
    let now = fixed_time();
    assert_eq!(super::preview_filename("%F_%H-%M", now), "2026-09-27_14-30");
    // Trailing bare % is stripped (F27 rule)
    assert_eq!(super::preview_filename("shot%", now), "shot");
    assert_eq!(super::preview_filename("100%%", now), "100%");
    // Sanitization: / -> U+2044, : -> -
    assert_eq!(super::preview_filename("a/b:c", now), "a\u{2044}b-c");
    // The shipped default matches the core schema default
    assert_eq!(
        super::preview_filename(&SaveConfig::default().filename_pattern, now),
        "2026-09-27_14-30"
    );
}

#[test]
fn theme_choice_resolves_against_the_system_preference() {
    assert_eq!(ThemeChoice::Dark.resolve(ThemeMode::Light), ThemeMode::Dark);
    assert_eq!(
        ThemeChoice::Light.resolve(ThemeMode::Dark),
        ThemeMode::Light
    );
    assert_eq!(
        ThemeChoice::System.resolve(ThemeMode::Light),
        ThemeMode::Light
    );
    assert_eq!(
        ThemeChoice::System.resolve(ThemeMode::Dark),
        ThemeMode::Dark
    );
}

#[test]
fn frame_action_merge_prefers_close() {
    assert_eq!(
        FrameAction::None.merge(FrameAction::Apply),
        FrameAction::Apply
    );
    assert_eq!(
        FrameAction::Apply.merge(FrameAction::Close),
        FrameAction::Close
    );
    assert_eq!(
        FrameAction::Close.merge(FrameAction::Apply),
        FrameAction::Close
    );
    assert_eq!(
        FrameAction::None.merge(FrameAction::None),
        FrameAction::None
    );
}

#[test]
fn tab_vocabulary_covers_the_four_f12_tabs() {
    assert_eq!(Tab::ALL.len(), 4);
    assert_eq!(Tab::default(), Tab::General);
    let labels: Vec<&str> = Tab::ALL.iter().map(|tab| tab.label()).collect();
    assert_eq!(
        labels
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
}

// --- the settings-rework layout grid + theme projection -------------------

use super::layout::FormMetrics;
use flowshot_core::config::UiConfig;

#[test]
fn form_metrics_derive_the_grid_from_tokens() {
    // Given: the default tokens (base 14, spacing 4/8/16, radii 4/8)
    let m = FormMetrics::from_tokens(&DesignTokens::default());
    // Then: the grid math follows the token derivations
    assert_eq!(m.base_size(), 14.0);
    assert_eq!(m.control_height(), 14.0 * 1.25 + 2.0 * 4.0);
    assert_eq!(m.row_pitch(), m.control_height() + 4.0);
    assert_eq!(m.window_margin(), 16.0);
    assert_eq!(m.card_padding(), 16.0);
    assert_eq!(m.gutter(), 8.0);
    assert_eq!(m.content_left(), 32.0);
    // The label column: em cap wins on wide layouts ...
    assert_eq!(m.label_width(1000.0), 14.0 * 22.0);
    // ... the 48% ratio wins on medium ones ...
    assert_eq!(m.label_width(600.0), 288.0);
    // ... and the control-column floor wins on narrow ones
    assert_eq!(m.label_width(160.0), 160.0 - 14.0 * 6.0 - 8.0);
    // The content column caps at 52em and centers past it ...
    assert_eq!(m.content_width(1000.0), 14.0 * 52.0);
    assert_eq!(m.content_offset(1000.0), (1000.0 - 14.0 * 52.0) / 2.0);
    assert_eq!(m.content_width(600.0), 600.0);
    assert_eq!(m.content_offset(600.0), 0.0);
    // ... and the control column's x adds the centering offset to
    // content_left + label + gutter (label measured on the capped inner)
    assert_eq!(m.control_x(1000.0), 136.0 + 32.0 + 308.0 + 8.0);
    // The label is measured on the capped INNER width (scroll width minus
    // the card padding): 632 scroll -> 600 inner -> the 48% ratio (288)
    assert_eq!(m.control_x(632.0), 32.0 + 288.0 + 8.0);
    // Field and combo caps keep short controls at an honest width
    assert_eq!(m.field_max_width(), 14.0 * 24.0);
    assert_eq!(m.combo_max_width(), 14.0 * 20.0);
}

#[test]
fn form_metrics_follow_token_changes() {
    // Given: tokens with a larger base size and generous spacing
    let mut tokens = DesignTokens::default();
    tokens.typography.base_size = 16;
    tokens.spacing.large = 24;
    tokens.spacing.small = 6;
    // When: projected
    let m = FormMetrics::from_tokens(&tokens);
    // Then: every derived metric moved with the tokens
    assert_eq!(m.content_left(), 48.0);
    assert_eq!(m.label_width(2000.0), 16.0 * 22.0);
    assert_eq!(m.control_height(), 16.0 * 1.25 + 2.0 * 6.0);
    assert_eq!(m.card_radius(), f32::from(tokens.radii.large as u8));
}

#[test]
fn settings_style_projects_token_geometry() {
    // Given: the default tokens
    let tokens = DesignTokens::default();
    let ui = UiConfig::default();
    // When: the settings style is projected
    let style = theme::settings_style(&tokens, &ui, ThemeMode::Dark);
    let m = FormMetrics::from_tokens(&tokens);
    // Then: the uniform control height floors every widget
    assert_eq!(style.spacing.interact_size.y, m.control_height());
    assert_eq!(style.spacing.icon_width, m.base_size());
    // The scrollbar is solid + reserved (a floating bar fades out at
    // idle and has no affordance); it owns the right window-margin band
    assert!(!style.spacing.scroll.floating);
    assert_eq!(style.spacing.scroll.bar_width, 8.0);
    // Crisp clipping at the scroll viewport
    assert_eq!(style.visuals.clip_rect_margin, 0.0);

    // Given: bigger typography + spacing tokens
    let mut big = DesignTokens::default();
    big.typography.base_size = 18;
    big.spacing.small = 6;
    // When: projected
    let style2 = theme::settings_style(&big, &ui, ThemeMode::Dark);
    // Then: the geometry follows (token change -> style change)
    let m2 = FormMetrics::from_tokens(&big);
    assert_eq!(style2.spacing.interact_size.y, m2.control_height());
    assert!(style2.spacing.interact_size.y > style.spacing.interact_size.y);
    assert_eq!(style2.spacing.icon_width, 18.0);
}

#[test]
fn surfaces_derive_from_the_contrast_token() {
    let tokens = DesignTokens::default();
    let ui = UiConfig::default();
    // Dark: the window IS the contrast token; the card lifts off it
    let dark = theme::surfaces_for(&tokens, &ui, ThemeMode::Dark);
    assert_eq!(dark.window, egui::Color32::from_rgb(0x0F, 0x17, 0x2A));
    assert_ne!(dark.card, dark.window);
    assert!(dark.card.r() > dark.window.r());
    assert!(dark.field.r() < dark.card.r());
    // Light: white cards on a contrast-tinted window
    let light = theme::surfaces_for(&tokens, &ui, ThemeMode::Light);
    assert_eq!(light.card, egui::Color32::WHITE);
    assert_ne!(light.window, light.card);
    // A different contrast token re-derives the whole scale (an empty
    // config color defers to the palette token - the resolution order)
    let mut teal = DesignTokens::default();
    teal.palette.contrast = "#102030".to_owned();
    let unconfigured = UiConfig {
        contrast_color: String::new(),
        ..Default::default()
    };
    let dark2 = theme::surfaces_for(&teal, &unconfigured, ThemeMode::Dark);
    assert_eq!(dark2.window, egui::Color32::from_rgb(0x10, 0x20, 0x30));
    assert_ne!(dark2.card, dark.card);
}

#[test]
fn fonts_register_the_hierarchy_weights() {
    let fonts = theme::fonts();
    assert!(fonts.font_data.contains_key(theme::MEDIUM_FAMILY));
    assert!(fonts.font_data.contains_key(theme::SEMIBOLD_FAMILY));
    let semibold = fonts
        .families
        .get(&egui::FontFamily::Name(theme::SEMIBOLD_FAMILY.into()))
        .unwrap();
    assert_eq!(
        semibold.first().map(String::as_str),
        Some(theme::SEMIBOLD_FAMILY)
    );
    // Regular Inter backs the weight families as the fallback chain
    assert_eq!(semibold.get(1).map(String::as_str), Some("Inter"));
    // The weight faces are real TrueType files (the todo-19 asset guard)
    for family in [theme::MEDIUM_FAMILY, theme::SEMIBOLD_FAMILY] {
        let face = fonts.font_data.get(family).unwrap();
        let magic: &[u8] = &face.font[..4];
        assert!(
            magic == [0x00, 0x01, 0x00, 0x00]
                || magic == b"true"
                || magic == b"ttcf"
                || magic == b"OTTO",
            "{family} must be a real font file, got magic {magic:02X?}"
        );
    }
}

/// Every row label the tabs render, measured against the label column.
const ROW_LABELS: [&str; 41] = [
    super::strings::FIELD_HIDE_CURSOR,
    super::strings::FIELD_SAVE_LAST_REGION,
    super::strings::FIELD_SAVE_PATH,
    super::strings::FIELD_PATH_FIXED,
    super::strings::FIELD_EXTENSION,
    super::strings::FIELD_JPEG_QUALITY,
    super::strings::FIELD_CLIPBOARD_FORMAT,
    super::strings::FIELD_DRAW_COLOR,
    super::strings::FIELD_DRAW_THICKNESS,
    super::strings::FIELD_FONT_FAMILY,
    super::strings::FIELD_FONT_SIZE,
    super::strings::FIELD_MAGNIFIER,
    super::strings::FIELD_MAGNIFIER_SHAPE,
    super::strings::FIELD_HUD_POSITION,
    super::strings::FIELD_HUD_HIDE_TIME,
    super::strings::FIELD_GRID,
    super::strings::FIELD_UNDO_LIMIT,
    super::strings::FIELD_DOUBLE_CLICK_COPIES,
    super::strings::FIELD_SIDE_PANEL,
    super::strings::FIELD_ARROW_STYLE,
    super::strings::FIELD_ARROW_REVERSE,
    super::strings::FIELD_MARKER_SIZE,
    super::strings::FIELD_PIXELATE_SIZE,
    super::strings::FIELD_CORNER_RADIUS,
    super::strings::FIELD_COUNTER_START,
    super::strings::FIELD_COUNTER_OUTLINE,
    super::strings::FIELD_PIN_MIN_SIZE,
    super::strings::FIELD_UPLOAD_PROVIDER,
    super::strings::FIELD_UPLOAD_CLIENT_ID,
    super::strings::FIELD_UPLOAD_NO_CONFIRM,
    super::strings::FIELD_UPLOAD_COPY_URL,
    super::strings::FIELD_UPLOAD_HISTORY_MAX,
    super::strings::FIELD_TRAY,
    super::strings::FIELD_NOTIFICATIONS,
    super::strings::FIELD_STARTUP_LAUNCH,
    super::strings::FIELD_ACCENT_COLOR,
    super::strings::FIELD_CONTRAST_COLOR,
    super::strings::FIELD_DIM_OPACITY,
    super::strings::FIELD_THEME,
    super::strings::FIELD_FILENAME_PATTERN,
    super::strings::LABEL_PREVIEW,
];

#[test]
fn label_column_fits_every_row_label_on_one_line() {
    // Given: the vendored Inter at the default token size
    let ctx = egui::Context::default();
    ctx.set_fonts(theme::fonts());
    // ctx.fonts() needs one begun frame (pixels_per_point is unknown before)
    let _ = ctx.run(egui::RawInput::default(), |_| {});
    let tokens = DesignTokens::default();
    let m = FormMetrics::from_tokens(&tokens);
    // The offscreen QA window (900px) leaves >= 800px of card content
    let label_width = m.label_width(800.0);
    let font = egui::FontId::proportional(m.base_size());
    for label in ROW_LABELS {
        // When: the label is shaped with the real font
        let width = ctx
            .fonts(|fonts| fonts.layout_no_wrap(label.into(), font.clone(), egui::Color32::WHITE))
            .size()
            .x;
        // Then: it fits the column without wrapping (uniform row height)
        assert!(
            width <= label_width,
            "{label:?} shapes to {width}px but the column is {label_width}px"
        );
    }
}

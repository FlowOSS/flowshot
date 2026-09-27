//! Settings-surface unit tests: model round-trips, validation, reset,
//! recorder seams, theme projection, keymap consistency, and the filename
//! preview.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
use egui::Key;
use flowshot_core::config::{CONFIG_VERSION, Config, SaveConfig};
use flowshot_core::tokens::DesignTokens;
use winit::keyboard::KeyCode;

use super::keymap;
use super::model::{Banner, RecorderTarget, SettingsModel, Tab, ThemeChoice};
use super::tabs::FrameAction;
use super::theme::{self, ThemeMode, parse_hex_rgb};

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

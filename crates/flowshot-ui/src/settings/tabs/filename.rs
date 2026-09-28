//! Filename Editor tab: `[save].filename_pattern` with a live preview and a
//! reset-to-default (plan todo 36 tab 3), as one section card.
//!
//! [`preview_filename`] mirrors the authoritative expander in
//! `flowshot-actions::export::pattern` (expand + sanitize); the actions
//! crate is not a lib dependency of this crate (purity gate), so the
//! preview keeps its own copy of the two rules - a doc-sync test in the
//! actions crate owns the authoritative behavior.

use chrono::{DateTime, Local};
use egui::Ui;
use flowshot_core::config::SaveConfig;

use super::super::model::SettingsModel;
use super::super::strings;
use super::TabContext;
use super::fields::text_field;
use super::form::{card, hint, readout, row};

pub(super) fn show(ui: &mut Ui, model: &mut SettingsModel, context: &TabContext<'_>) {
    let m = &context.metrics;
    let changed = card(ui, m, strings::TAB_FILENAME, |ui| {
        let config = model.config_mut();
        let mut changed = text_field(
            ui,
            m,
            strings::FIELD_FILENAME_PATTERN,
            &mut config.save.filename_pattern,
        );
        hint(ui, m, strings::HINT_FILENAME_PATTERN);
        let preview = preview_filename(&config.save.filename_pattern, Local::now());
        row(ui, m, strings::LABEL_PREVIEW, |ui| readout(ui, m, &preview));
        let reset = row(ui, m, "", |ui| {
            ui.button(strings::BUTTON_RESET_PATTERN).clicked()
        });
        if reset {
            config.save.filename_pattern = SaveConfig::default().filename_pattern;
            changed = true;
        }
        changed
    });
    if changed {
        model.mark_dirty();
    }
}

/// Expands a strftime-style pattern against `now` and sanitizes it for the
/// filesystem: a trailing bare `%` is stripped, `/` becomes U+2044 and `:`
/// becomes `-` (the F27 filename rules).
#[must_use]
pub fn preview_filename(pattern: &str, now: DateTime<Local>) -> String {
    let trimmed = if pattern.ends_with('%') && !pattern.ends_with("%%") {
        &pattern[..pattern.len() - 1]
    } else {
        pattern
    };
    now.format(trimmed)
        .to_string()
        .replace('/', "\u{2044}")
        .replace(':', "-")
}

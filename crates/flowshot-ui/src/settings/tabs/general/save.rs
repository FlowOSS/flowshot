//! The `[save]` group: path (+ the rfd Browse seam), naming, JPEG quality,
//! clipboard format, and the ordered post-capture action set.

use egui::Ui;
use flowshot_core::config::{ClipboardFormat, Config, SaveAction};

use crate::settings::fields::{combo, number, path_field, text_field, toggle};
use crate::settings::form::sub_section;
use crate::settings::layout::FormMetrics;
use crate::settings::model::JPEG_QUALITY_RANGE;
use crate::settings::strings;
use crate::settings::tabs::TabContext;

/// The post-capture action vocabulary, in canonical TOML order. Mirrors the
/// core `SaveAction` enum; [`action_label`]'s exhaustive match fails the
/// build if core grows a variant.
const ALL_SAVE_ACTIONS: [SaveAction; 7] = [
    SaveAction::Copy,
    SaveAction::Save,
    SaveAction::Pin,
    SaveAction::Upload,
    SaveAction::CopyPath,
    SaveAction::Notify,
    SaveAction::OpenWith,
];

fn action_label(action: SaveAction) -> &'static str {
    match action {
        SaveAction::Copy => strings::ACTION_COPY,
        SaveAction::Save => strings::ACTION_SAVE,
        SaveAction::Pin => strings::ACTION_PIN,
        SaveAction::Upload => strings::ACTION_UPLOAD,
        SaveAction::CopyPath => strings::ACTION_COPY_PATH,
        SaveAction::Notify => strings::ACTION_NOTIFY,
        SaveAction::OpenWith => strings::ACTION_OPEN_WITH,
    }
}

pub(super) fn show(ui: &mut Ui, config: &mut Config, context: &TabContext<'_>) -> bool {
    let m = &context.metrics;
    let mut changed = path_field(ui, m, &mut config.save.path, context.path_picker);
    changed |= toggle(
        ui,
        m,
        strings::FIELD_PATH_FIXED,
        &mut config.save.path_fixed,
    );
    changed |= text_field(ui, m, strings::FIELD_EXTENSION, &mut config.save.extension);
    changed |= number(
        ui,
        m,
        strings::FIELD_JPEG_QUALITY,
        JPEG_QUALITY_RANGE,
        &mut config.save.jpeg_quality,
    );
    changed |= combo(
        ui,
        m,
        strings::FIELD_CLIPBOARD_FORMAT,
        &mut config.save.clipboard_format,
        &[
            (ClipboardFormat::Png, strings::ENUM_PNG),
            (ClipboardFormat::Jpeg, strings::ENUM_JPEG),
        ],
    );
    changed |= sub_section(ui, m, strings::FIELD_SAVE_ACTIONS, |ui| {
        action_set(ui, m, config)
    });
    changed
}

/// The ordered action-set checklist; writes back in canonical TOML order
/// when any box changed (the pre-rework contract).
fn action_set(ui: &mut Ui, m: &FormMetrics, config: &mut Config) -> bool {
    let mut selected = config.save.actions.clone();
    let mut changed = false;
    for action in ALL_SAVE_ACTIONS {
        let mut enabled = selected.contains(&action);
        if toggle(ui, m, action_label(action), &mut enabled) {
            if enabled {
                selected.push(action);
            } else {
                selected.retain(|existing| *existing != action);
            }
            changed = true;
        }
    }
    if changed {
        config.save.actions = ALL_SAVE_ACTIONS
            .into_iter()
            .filter(|candidate| selected.contains(candidate))
            .collect();
    }
    changed
}

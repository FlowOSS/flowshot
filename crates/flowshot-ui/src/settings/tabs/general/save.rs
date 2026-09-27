//! The `[save]` group: path (+ the rfd Browse seam), naming, JPEG quality,
//! clipboard format, and the ordered post-capture action set.

use egui::Ui;
use flowshot_core::config::{ClipboardFormat, Config, SaveAction};

use crate::settings::model::JPEG_QUALITY_RANGE;
use crate::settings::strings;
use crate::settings::tabs::{TabContext, combo, drag_u8, text_field, toggle};

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
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(strings::FIELD_SAVE_PATH);
        changed |= ui.text_edit_singleline(&mut config.save.path).changed();
        let picker = context.path_picker;
        let browse = ui.add_enabled(picker.is_some(), egui::Button::new(strings::BUTTON_BROWSE));
        if browse.clicked()
            && let Some(picker) = picker
            && let Some(picked) = picker.pick()
        {
            config.save.path = picked;
            changed = true;
        }
    });
    changed |= toggle(ui, strings::FIELD_PATH_FIXED, &mut config.save.path_fixed);
    changed |= text_field(ui, strings::FIELD_EXTENSION, &mut config.save.extension);
    changed |= drag_u8(
        ui,
        strings::FIELD_JPEG_QUALITY,
        JPEG_QUALITY_RANGE,
        &mut config.save.jpeg_quality,
    );
    changed |= combo(
        ui,
        strings::FIELD_CLIPBOARD_FORMAT,
        &mut config.save.clipboard_format,
        &[
            (ClipboardFormat::Png, strings::ENUM_PNG),
            (ClipboardFormat::Jpeg, strings::ENUM_JPEG),
        ],
    );
    ui.label(strings::FIELD_SAVE_ACTIONS);
    let mut selected = config.save.actions.clone();
    let mut actions_changed = false;
    for action in ALL_SAVE_ACTIONS {
        let mut enabled = selected.contains(&action);
        if toggle(ui, action_label(action), &mut enabled) {
            if enabled {
                selected.push(action);
            } else {
                selected.retain(|existing| *existing != action);
            }
            actions_changed = true;
        }
    }
    if actions_changed {
        config.save.actions = ALL_SAVE_ACTIONS
            .into_iter()
            .filter(|candidate| selected.contains(candidate))
            .collect();
        changed = true;
    }
    changed
}

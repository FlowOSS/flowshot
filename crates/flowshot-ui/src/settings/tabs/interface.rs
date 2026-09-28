//! Interface tab: theme selection, `[ui]` accent/contrast pickers, dim
//! opacity, the `[ui].toolbar_buttons` order list, and the
//! `[editor].color_palette` swatch editor (plan todo 36 tab 2) - as three
//! section cards (Appearance / Toolbar button order / Color palette).
//!
//! The accent/contrast pickers re-theme this window LIVE: the frame closure
//! re-projects [`crate::egui_host::theme::settings_style`] from the model
//! every frame, which is the "live-apply where safe (tokens-driven)"
//! contract.
//!
//! The toolbar list is an order editor with move/remove/add controls (egui
//! 0.28 has no drag-list widget; up/down buttons are the honest equivalent,
//! recorded deviation). The add vocabulary mirrors the chrome's button ids:
//! every [`ToolKind`] plus the action ids the toolbar's icon match
//! consumes.

use egui::Ui;

use crate::editor::ToolKind;

use super::super::layout::FormMetrics;
use super::super::model::{SettingsModel, ThemeChoice};
use super::super::strings;
use super::TabContext;
use super::fields::{combo, hex_color, number};
use super::form::{card, row};
use crate::egui_host::theme::parse_hex_rgb;

/// The non-tool toolbar button ids (chrome/toolbar.rs icon vocabulary).
const TOOLBAR_ACTION_IDS: [&str; 8] = [
    "copy", "save", "pin", "upload", "undo", "redo", "open-app", "exit",
];

pub(super) fn show(ui: &mut Ui, model: &mut SettingsModel, context: &TabContext<'_>) {
    let m = &context.metrics;
    let mut theme_choice = model.theme();
    let mut theme_changed = false;
    let mut changed = card(ui, m, strings::GROUP_APPEARANCE, |ui| {
        theme_changed = combo(
            ui,
            m,
            strings::FIELD_THEME,
            &mut theme_choice,
            &[
                (ThemeChoice::Dark, strings::ENUM_DARK),
                (ThemeChoice::Light, strings::ENUM_LIGHT),
                (ThemeChoice::System, strings::ENUM_SYSTEM),
            ],
        );
        let mut changed = false;
        let config = model.config_mut();
        changed |= hex_color(
            ui,
            m,
            strings::FIELD_ACCENT_COLOR,
            &mut config.ui.accent_color,
        );
        changed |= hex_color(
            ui,
            m,
            strings::FIELD_CONTRAST_COLOR,
            &mut config.ui.contrast_color,
        );
        changed |= number(
            ui,
            m,
            strings::FIELD_DIM_OPACITY,
            0..=255,
            &mut config.ui.dim_opacity,
        );
        changed
    });
    if theme_changed {
        model.set_theme(theme_choice);
    }
    changed |= card(ui, m, strings::FIELD_TOOLBAR_BUTTONS, |ui| {
        let config = model.config_mut();
        toolbar_list(ui, m, &mut config.ui.toolbar_buttons)
    });
    changed |= card(ui, m, strings::FIELD_COLOR_PALETTE, |ui| {
        let config = model.config_mut();
        palette_editor(ui, &mut config.editor.color_palette)
    });
    if changed {
        model.mark_dirty();
    }
}

/// Capitalizes a button id for display ("open-app" -> "Open-app").
fn title(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn known_button_ids() -> impl Iterator<Item = &'static str> {
    ToolKind::ALL
        .into_iter()
        .map(ToolKind::id)
        .chain(TOOLBAR_ACTION_IDS)
}

fn toolbar_list(ui: &mut Ui, m: &FormMetrics, buttons: &mut Vec<String>) -> bool {
    let mut changed = false;
    let count = buttons.len();
    let mut swap: Option<(usize, usize)> = None;
    let mut remove: Option<usize> = None;
    for (index, id) in buttons.iter().enumerate() {
        row(ui, m, &title(id), |ui| {
            if ui
                .add_enabled(index > 0, egui::Button::new(strings::BUTTON_MOVE_UP))
                .clicked()
            {
                swap = Some((index, index - 1));
            }
            if ui
                .add_enabled(
                    index + 1 < count,
                    egui::Button::new(strings::BUTTON_MOVE_DOWN),
                )
                .clicked()
            {
                swap = Some((index, index + 1));
            }
            if ui.button(strings::BUTTON_REMOVE).clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some((from, to)) = swap {
        buttons.swap(from, to);
        changed = true;
    }
    if let Some(index) = remove {
        buttons.remove(index);
        changed = true;
    }

    let unused: Vec<&'static str> = known_button_ids()
        .filter(|id| !buttons.iter().any(|existing| existing == id))
        .collect();
    let combo_id = ui.id().with("toolbar-button-add");
    let mut candidate: String = ui
        .data(|data| data.get_temp(combo_id))
        .or_else(|| unused.first().map(|id| (*id).to_owned()))
        .unwrap_or_default();
    if !unused.iter().any(|id| *id == candidate) {
        candidate = unused.first().map_or(String::new(), |id| (*id).to_owned());
    }
    row(ui, m, "", |ui| {
        egui::ComboBox::from_id_source(combo_id)
            .selected_text(title(&candidate))
            .show_ui(ui, |ui| {
                for id in &unused {
                    ui.selectable_value(&mut candidate, (*id).to_owned(), title(id));
                }
            });
        if ui
            .add_enabled(
                !candidate.is_empty(),
                egui::Button::new(strings::BUTTON_ADD),
            )
            .clicked()
        {
            buttons.push(candidate.clone());
            changed = true;
        }
    });
    ui.data_mut(|data| data.insert_temp(combo_id, candidate));
    changed
}

fn palette_editor(ui: &mut Ui, palette: &mut Vec<String>) -> bool {
    let mut changed = false;
    let mut remove: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        for (index, entry) in palette.iter_mut().enumerate() {
            ui.scope(|ui| {
                ui.push_id(index, |ui| {
                    ui.horizontal(|ui| {
                        let mut rgb = parse_hex_rgb(entry).unwrap_or([0x7F, 0x7F, 0x7F]);
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            *entry = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                            changed = true;
                        }
                        if ui.small_button(strings::BUTTON_REMOVE).clicked() {
                            remove = Some(index);
                        }
                    });
                });
            });
        }
    });
    if ui.button(strings::BUTTON_ADD_SWATCH).clicked() {
        palette.push("#7F7F7F".to_owned());
        changed = true;
    }
    if let Some(index) = remove {
        palette.remove(index);
        changed = true;
    }
    changed
}

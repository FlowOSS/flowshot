//! Shortcuts tab: the per-action recorder over the todo-25 keybind seams
//! ([`ToolShortcuts::rebind`], [`ToolShortcuts::rebind_undo_redo`],
//! [`ToolShortcuts::rebind_z_order`]), as one section card of grid rows.
//!
//! Recording: a slot's Record button arms [`RecorderTarget`]; the next key
//! press seen in the frame input binds (Esc cancels). Keys without a
//! physical `KeyCode` (Colon/Pipe/Questionmark) cannot back a binding and
//! are rejected without disarming.
//!
//! Persistence: rebinds are session state on the model until the core
//! `[shortcuts]` config group lands (issues.md 2026-09-26); the binary
//! layer reads [`SettingsModel::shortcuts`] to project them into the
//! editor. GLOBAL capture shortcuts (Print…, todo 34) are daemon-owned
//! portal registrations - the hint above the card records the split
//! honestly.

use egui::{Align, Layout, RichText, Ui, vec2};
use winit::keyboard::KeyCode;

use crate::editor::ToolKind;

use super::super::layout::FormMetrics;
use super::super::model::{RecorderTarget, SettingsModel};
use super::super::strings;
use super::TabContext;
use super::form::{card, row};
use crate::egui_host::keymap;

/// The key-display cell width in em: sized for the recording hint (the
/// widest display text) so the Record/Clear buttons never shift while a
/// slot is armed. The cell forces this width via `set_min_width` -
/// `allocate_ui_with_layout` allocates the USED rect, so without the floor
/// a 1-char binding would collapse the cell and staircase the buttons.
const KEY_CELL_EM: f32 = 15.0;

/// One recorder row's data (the clear action stays a parameter - the
/// z-order slots' closures capture their sibling key).
struct Slot {
    target: RecorderTarget,
    label: String,
    current: Option<KeyCode>,
}

pub(super) fn show(ui: &mut Ui, model: &mut SettingsModel, context: &TabContext<'_>) {
    let m = &context.metrics;
    if model.recorder().is_some() {
        let captured = ui.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key, pressed: true, ..
                } => Some(*key),
                _ => None,
            })
        });
        if let Some(key) = captured {
            if key == egui::Key::Escape {
                model.cancel_recording();
            } else {
                model.capture_recorder_key(key);
            }
        }
    }

    ui.label(RichText::new(strings::HINT_GLOBAL_SHORTCUTS).weak());
    ui.add_space(m.medium());

    card(ui, m, strings::GROUP_EDITOR_SHORTCUTS, |ui| {
        for kind in ToolKind::ALL {
            let slot = Slot {
                target: RecorderTarget::Tool(kind),
                label: title(kind.id()),
                current: model.shortcuts().key_for_tool(kind),
            };
            slot_row(ui, model, m, &slot, |model| {
                model.shortcuts_mut().rebind(kind, None);
            });
        }

        ui.add_space(m.small());
        ui.separator();
        ui.add_space(m.small());

        let undo = model.shortcuts().undo_key();
        slot_row(
            ui,
            model,
            m,
            &Slot {
                target: RecorderTarget::Undo,
                label: strings::SHORTCUT_UNDO.to_owned(),
                current: Some(undo),
            },
            |_| {},
        );
        let redo = model.shortcuts().redo_key();
        slot_row(
            ui,
            model,
            m,
            &Slot {
                target: RecorderTarget::Redo,
                label: strings::SHORTCUT_REDO.to_owned(),
                current: Some(redo),
            },
            |_| {},
        );
        let (raise, lower) = model.shortcuts().z_keys();
        slot_row(
            ui,
            model,
            m,
            &Slot {
                target: RecorderTarget::Raise,
                label: strings::SHORTCUT_RAISE.to_owned(),
                current: raise,
            },
            |model| {
                let (_, lower) = model.shortcuts().z_keys();
                model.shortcuts_mut().rebind_z_order(None, lower);
            },
        );
        slot_row(
            ui,
            model,
            m,
            &Slot {
                target: RecorderTarget::Lower,
                label: strings::SHORTCUT_LOWER.to_owned(),
                current: lower,
            },
            |model| {
                let (raise, _) = model.shortcuts().z_keys();
                model.shortcuts_mut().rebind_z_order(raise, None);
            },
        );
        false
    });
}

/// Capitalizes a tool id for display.
fn title(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn key_label(code: KeyCode) -> String {
    keymap::egui_key_from_code(code).map_or_else(
        || format!("{code:?}"),
        |key| key.symbol_or_name().to_owned(),
    )
}

fn slot_row(
    ui: &mut Ui,
    model: &mut SettingsModel,
    m: &FormMetrics,
    slot: &Slot,
    clear: impl FnOnce(&mut SettingsModel),
) {
    let recording = model.recorder() == Some(slot.target);
    let display = if recording {
        strings::HINT_RECORDING.to_owned()
    } else {
        slot.current
            .map_or_else(|| strings::LABEL_UNBOUND.to_owned(), key_label)
    };
    let mut start = false;
    let mut clear_binding = false;
    row(ui, m, &slot.label, |ui| {
        let key_cell = vec2(m.base_size() * KEY_CELL_EM, m.control_height());
        ui.allocate_ui_with_layout(key_cell, Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(key_cell.x);
            let text = RichText::new(display).monospace();
            let text = if recording {
                text.color(ui.visuals().selection.stroke.color)
            } else {
                text
            };
            ui.label(text);
        });
        if ui.button(strings::BUTTON_RECORD).clicked() {
            start = true;
        }
        if ui
            .add_enabled(
                slot.current.is_some() && !recording,
                egui::Button::new(strings::BUTTON_CLEAR),
            )
            .clicked()
        {
            clear_binding = true;
        }
    });
    if start {
        model.start_recording(slot.target);
    }
    if clear_binding {
        clear(model);
        model.mark_dirty();
    }
}

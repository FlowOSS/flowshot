//! Shortcuts tab: the per-action recorder over the todo-25 keybind seams
//! ([`ToolShortcuts::rebind`], [`ToolShortcuts::rebind_undo_redo`],
//! [`ToolShortcuts::rebind_z_order`]).
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
//! portal registrations - the hint below records the split honestly.

use egui::Ui;
use winit::keyboard::KeyCode;

use crate::editor::ToolKind;

use super::super::model::{RecorderTarget, SettingsModel};
use super::super::strings;
use crate::egui_host::keymap;

pub(super) fn show(ui: &mut Ui, model: &mut SettingsModel) {
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

    ui.label(egui::RichText::new(strings::HINT_GLOBAL_SHORTCUTS).weak());
    ui.add_space(4.0);

    for kind in ToolKind::ALL {
        let current = model.shortcuts().key_for_tool(kind);
        row(
            ui,
            model,
            RecorderTarget::Tool(kind),
            &title(kind.id()),
            current,
            |model| {
                model.shortcuts_mut().rebind(kind, None);
            },
        );
    }

    ui.separator();
    let undo = model.shortcuts().undo_key();
    row(
        ui,
        model,
        RecorderTarget::Undo,
        strings::SHORTCUT_UNDO,
        Some(undo),
        |_| {},
    );
    let redo = model.shortcuts().redo_key();
    row(
        ui,
        model,
        RecorderTarget::Redo,
        strings::SHORTCUT_REDO,
        Some(redo),
        |_| {},
    );
    let (raise, lower) = model.shortcuts().z_keys();
    row(
        ui,
        model,
        RecorderTarget::Raise,
        strings::SHORTCUT_RAISE,
        raise,
        |model| {
            let (_, lower) = model.shortcuts().z_keys();
            model.shortcuts_mut().rebind_z_order(None, lower);
        },
    );
    row(
        ui,
        model,
        RecorderTarget::Lower,
        strings::SHORTCUT_LOWER,
        lower,
        |model| {
            let (raise, _) = model.shortcuts().z_keys();
            model.shortcuts_mut().rebind_z_order(raise, None);
        },
    );
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

fn row(
    ui: &mut Ui,
    model: &mut SettingsModel,
    target: RecorderTarget,
    label: &str,
    current: Option<KeyCode>,
    clear: impl FnOnce(&mut SettingsModel),
) {
    let recording = model.recorder() == Some(target);
    let display = if recording {
        strings::HINT_RECORDING.to_owned()
    } else {
        current.map_or_else(|| strings::LABEL_UNBOUND.to_owned(), key_label)
    };
    let mut start = false;
    let mut clear_binding = false;
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(
                    current.is_some() && !recording,
                    egui::Button::new(strings::BUTTON_CLEAR),
                )
                .clicked()
            {
                clear_binding = true;
            }
            if ui.button(strings::BUTTON_RECORD).clicked() {
                start = true;
            }
            ui.monospace(&display);
        });
    });
    if start {
        model.start_recording(target);
    }
    if clear_binding {
        clear(model);
        model.mark_dirty();
    }
}

//! The `[editor]` group (minus `color_palette`, which the Interface tab's
//! palette editor owns).

use egui::Ui;
use flowshot_core::config::{Config, MagnifierShape};

use crate::settings::model::UNDO_LIMIT_MAX;
use crate::settings::strings;
use crate::settings::tabs::{combo, drag_u32, hex_color, text_field, toggle};

pub(super) fn show(ui: &mut Ui, config: &mut Config) -> bool {
    let mut changed = false;
    changed |= hex_color(ui, strings::FIELD_DRAW_COLOR, &mut config.editor.draw_color);
    changed |= drag_u32(
        ui,
        strings::FIELD_DRAW_THICKNESS,
        1..=50,
        &mut config.editor.draw_thickness,
    );
    changed |= text_field(
        ui,
        strings::FIELD_FONT_FAMILY,
        &mut config.editor.font_family,
    );
    changed |= drag_u32(
        ui,
        strings::FIELD_FONT_SIZE,
        1..=200,
        &mut config.editor.font_size,
    );
    changed |= toggle(ui, strings::FIELD_MAGNIFIER, &mut config.editor.magnifier);
    changed |= combo(
        ui,
        strings::FIELD_MAGNIFIER_SHAPE,
        &mut config.editor.magnifier_shape,
        &[
            (MagnifierShape::Square, strings::ENUM_SQUARE),
            (MagnifierShape::Circle, strings::ENUM_CIRCLE),
        ],
    );
    changed |= combo(
        ui,
        strings::FIELD_HUD_POSITION,
        &mut config.editor.hud_position,
        &[
            (1_u8, strings::ENUM_HUD_TOP_LEFT),
            (2, strings::ENUM_HUD_TOP_RIGHT),
            (3, strings::ENUM_HUD_BOTTOM_LEFT),
            (4, strings::ENUM_HUD_BOTTOM_RIGHT),
        ],
    );
    changed |= drag_u32(
        ui,
        strings::FIELD_HUD_HIDE_TIME,
        0..=60_000,
        &mut config.editor.hud_hide_time,
    );
    changed |= toggle(ui, strings::FIELD_GRID, &mut config.editor.grid);
    changed |= drag_u32(
        ui,
        strings::FIELD_UNDO_LIMIT,
        0..=UNDO_LIMIT_MAX,
        &mut config.editor.undo_limit,
    );
    changed |= toggle(
        ui,
        strings::FIELD_DOUBLE_CLICK_COPIES,
        &mut config.editor.double_click_copies,
    );
    changed |= toggle(ui, strings::FIELD_SIDE_PANEL, &mut config.editor.side_panel);
    changed
}

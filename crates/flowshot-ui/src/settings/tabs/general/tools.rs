//! The `[tools.*]` groups: one nested collapsing group per tool.

use egui::Ui;
use flowshot_core::config::{ArrowStyle, Config};

use crate::settings::strings;
use crate::settings::tabs::{combo, drag_u32, group, toggle};

pub(super) fn show(ui: &mut Ui, config: &mut Config) -> bool {
    let mut changed = false;
    changed |= group(ui, strings::GROUP_TOOL_ARROW, |ui| {
        let mut changed = false;
        changed |= combo(
            ui,
            strings::FIELD_ARROW_STYLE,
            &mut config.tools.arrow.style,
            &[
                (ArrowStyle::Straight, strings::ENUM_STRAIGHT),
                (ArrowStyle::Curved, strings::ENUM_CURVED),
            ],
        );
        changed |= toggle(
            ui,
            strings::FIELD_ARROW_REVERSE,
            &mut config.tools.arrow.reverse,
        );
        changed
    });
    changed |= group(ui, strings::GROUP_TOOL_MARKER, |ui| {
        drag_u32(
            ui,
            strings::FIELD_MARKER_SIZE,
            1..=50,
            &mut config.tools.marker.size,
        )
    });
    changed |= group(ui, strings::GROUP_TOOL_PIXELATE, |ui| {
        drag_u32(
            ui,
            strings::FIELD_PIXELATE_SIZE,
            1..=50,
            &mut config.tools.pixelate.size,
        )
    });
    changed |= group(ui, strings::GROUP_TOOL_RECTANGLE, |ui| {
        drag_u32(
            ui,
            strings::FIELD_CORNER_RADIUS,
            0..=100,
            &mut config.tools.rectangle.corner_radius,
        )
    });
    changed |= group(ui, strings::GROUP_TOOL_COUNTER, |ui| {
        let mut changed = false;
        changed |= drag_u32(
            ui,
            strings::FIELD_COUNTER_START,
            0..=1000,
            &mut config.tools.counter.size,
        );
        changed |= toggle(
            ui,
            strings::FIELD_COUNTER_OUTLINE,
            &mut config.tools.counter.outline,
        );
        changed
    });
    changed
}

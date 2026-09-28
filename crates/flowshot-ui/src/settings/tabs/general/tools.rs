//! The `[tools.*]` groups: one titled sub-section per tool inside the Tools
//! card.

use egui::Ui;
use flowshot_core::config::{ArrowStyle, Config};

use crate::settings::layout::FormMetrics;
use crate::settings::strings;
use crate::settings::tabs::fields::{combo, number, toggle};
use crate::settings::tabs::form::sub_section;

pub(super) fn show(ui: &mut Ui, m: &FormMetrics, config: &mut Config) -> bool {
    let mut changed = sub_section(ui, m, strings::GROUP_TOOL_ARROW, |ui| {
        let mut changed = combo(
            ui,
            m,
            strings::FIELD_ARROW_STYLE,
            &mut config.tools.arrow.style,
            &[
                (ArrowStyle::Straight, strings::ENUM_STRAIGHT),
                (ArrowStyle::Curved, strings::ENUM_CURVED),
            ],
        );
        changed |= toggle(
            ui,
            m,
            strings::FIELD_ARROW_REVERSE,
            &mut config.tools.arrow.reverse,
        );
        changed
    });
    changed |= sub_section(ui, m, strings::GROUP_TOOL_MARKER, |ui| {
        number(
            ui,
            m,
            strings::FIELD_MARKER_SIZE,
            1..=50,
            &mut config.tools.marker.size,
        )
    });
    changed |= sub_section(ui, m, strings::GROUP_TOOL_PIXELATE, |ui| {
        number(
            ui,
            m,
            strings::FIELD_PIXELATE_SIZE,
            1..=50,
            &mut config.tools.pixelate.size,
        )
    });
    changed |= sub_section(ui, m, strings::GROUP_TOOL_RECTANGLE, |ui| {
        number(
            ui,
            m,
            strings::FIELD_CORNER_RADIUS,
            0..=100,
            &mut config.tools.rectangle.corner_radius,
        )
    });
    changed |= sub_section(ui, m, strings::GROUP_TOOL_COUNTER, |ui| {
        let mut changed = number(
            ui,
            m,
            strings::FIELD_COUNTER_START,
            0..=1000,
            &mut config.tools.counter.size,
        );
        changed |= toggle(
            ui,
            m,
            strings::FIELD_COUNTER_OUTLINE,
            &mut config.tools.counter.outline,
        );
        changed
    });
    changed
}

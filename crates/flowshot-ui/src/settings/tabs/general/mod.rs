//! General tab: every config key, grouped by its TOML table (F12 parity -
//! `[capture]`, `[save]`, `[editor]`, `[tools.*]`, `[pin]`, `[upload]`,
//! `[daemon]`, `[telemetry]`; the `[ui]` group lives in the Interface tab
//! and `[editor].color_palette` in its palette editor). One section card
//! per config-domain group.

mod editor;
mod save;
mod telemetry;
mod tools;
mod upload;

use egui::Ui;
use flowshot_core::config::Config;

use super::super::fields::{number, toggle};
use super::super::form::card;
use super::super::layout::FormMetrics;
use super::super::model::SettingsModel;
use super::super::strings;
use super::TabContext;

pub(super) fn show(ui: &mut Ui, model: &mut SettingsModel, context: &TabContext<'_>) {
    let m = &context.metrics;
    let mut changed = false;
    {
        let config = model.config_mut();
        changed |= card(ui, m, strings::GROUP_CAPTURE, |ui| {
            capture_group(ui, m, config)
        });
        changed |= card(ui, m, strings::GROUP_SAVE, |ui| {
            save::show(ui, config, context)
        });
        changed |= card(ui, m, strings::GROUP_EDITOR, |ui| {
            editor::show(ui, m, config)
        });
        changed |= card(ui, m, strings::GROUP_TOOLS, |ui| tools::show(ui, m, config));
        changed |= card(ui, m, strings::GROUP_PIN, |ui| pin_group(ui, m, config));
        changed |= card(ui, m, strings::GROUP_UPLOAD, |ui| {
            upload::show(ui, m, config)
        });
        changed |= card(ui, m, strings::GROUP_DAEMON, |ui| {
            daemon_group(ui, m, config)
        });
        changed |= card(ui, m, strings::GROUP_TELEMETRY, |ui| {
            telemetry::show(ui, m, config)
        });
    }
    if changed {
        model.mark_dirty();
    }
}

fn capture_group(ui: &mut Ui, m: &FormMetrics, config: &mut Config) -> bool {
    let mut changed = toggle(
        ui,
        m,
        strings::FIELD_HIDE_CURSOR,
        &mut config.capture.hide_cursor,
    );
    changed |= toggle(
        ui,
        m,
        strings::FIELD_SAVE_LAST_REGION,
        &mut config.capture.save_last_region,
    );
    changed
}

fn pin_group(ui: &mut Ui, m: &FormMetrics, config: &mut Config) -> bool {
    number(
        ui,
        m,
        strings::FIELD_PIN_MIN_SIZE,
        10..=1000,
        &mut config.pin.min_size,
    )
}

fn daemon_group(ui: &mut Ui, m: &FormMetrics, config: &mut Config) -> bool {
    let mut changed = toggle(ui, m, strings::FIELD_TRAY, &mut config.daemon.tray);
    changed |= toggle(
        ui,
        m,
        strings::FIELD_NOTIFICATIONS,
        &mut config.daemon.notifications,
    );
    changed |= toggle(
        ui,
        m,
        strings::FIELD_STARTUP_LAUNCH,
        &mut config.daemon.startup_launch,
    );
    changed
}

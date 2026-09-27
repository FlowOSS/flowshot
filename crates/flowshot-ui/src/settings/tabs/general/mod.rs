//! General tab: every config key, grouped by its TOML table (F12 parity -
//! `[capture]`, `[save]`, `[editor]`, `[tools.*]`, `[pin]`, `[upload]`,
//! `[daemon]`; the `[ui]` group lives in the Interface tab and
//! `[editor].color_palette` in its palette editor). One submodule per
//! config-domain group.

mod editor;
mod save;
mod tools;
mod upload;

use egui::Ui;
use flowshot_core::config::Config;

use super::super::model::SettingsModel;
use super::super::strings;
use super::{TabContext, group, toggle};

pub(super) fn show(ui: &mut Ui, model: &mut SettingsModel, context: &TabContext<'_>) {
    let mut changed = false;
    {
        let config = model.config_mut();
        changed |= group(ui, strings::GROUP_CAPTURE, |ui| capture_group(ui, config));
        changed |= group(ui, strings::GROUP_SAVE, |ui| {
            save::show(ui, config, context)
        });
        changed |= group(ui, strings::GROUP_EDITOR, |ui| editor::show(ui, config));
        changed |= group(ui, strings::GROUP_TOOLS, |ui| tools::show(ui, config));
        changed |= group(ui, strings::GROUP_PIN, |ui| pin_group(ui, config));
        changed |= group(ui, strings::GROUP_UPLOAD, |ui| upload::show(ui, config));
        changed |= group(ui, strings::GROUP_DAEMON, |ui| daemon_group(ui, config));
    }
    if changed {
        model.mark_dirty();
    }
}

fn capture_group(ui: &mut Ui, config: &mut Config) -> bool {
    let mut changed = false;
    changed |= toggle(
        ui,
        strings::FIELD_HIDE_CURSOR,
        &mut config.capture.hide_cursor,
    );
    changed |= toggle(
        ui,
        strings::FIELD_SAVE_LAST_REGION,
        &mut config.capture.save_last_region,
    );
    changed
}

fn pin_group(ui: &mut Ui, config: &mut Config) -> bool {
    super::drag_u32(
        ui,
        strings::FIELD_PIN_MIN_SIZE,
        10..=1000,
        &mut config.pin.min_size,
    )
}

fn daemon_group(ui: &mut Ui, config: &mut Config) -> bool {
    let mut changed = false;
    changed |= toggle(ui, strings::FIELD_TRAY, &mut config.daemon.tray);
    changed |= toggle(
        ui,
        strings::FIELD_NOTIFICATIONS,
        &mut config.daemon.notifications,
    );
    changed |= toggle(
        ui,
        strings::FIELD_STARTUP_LAUNCH,
        &mut config.daemon.startup_launch,
    );
    changed
}

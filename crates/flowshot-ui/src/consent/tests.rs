//! Consent-dialog unit tests: the trigger rule (who shows the dialog
//! when), the answer -> config state machine (ONLY "Save choice" writes
//! and settles; every dismissal records nothing and re-arms the prompt),
//! and the widget layer driven headlessly through a bare [`egui::Context`]
//! with synthetic raw input (the launcher-tests pattern).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use flowshot_core::config::{TelemetryConfig, UiConfig};
use flowshot_core::tokens::DesignTokens;

use super::model::{ConsentModel, PromptSurface, should_prompt};
use super::ui::{self as widgets, ConsentAction};
use crate::egui_host::theme::{ThemeMode, settings_style};
use crate::settings::FormMetrics;

const ALL_SURFACES: [PromptSurface; 3] = [
    PromptSurface::DaemonStartup,
    PromptSurface::OneShotRun,
    PromptSurface::SessionChild,
];

// --- the trigger rule ------------------------------------------------------

#[test]
fn daemon_startup_is_the_only_prompting_surface_while_unanswered() {
    // Given: a fresh install (every consent flag false)
    let fresh = TelemetryConfig::default();
    // Then: the daemon startup prompts ...
    assert!(should_prompt(&fresh, PromptSurface::DaemonStartup));
    // ... and no other surface ever pops a dialog mid-flow
    assert!(!should_prompt(&fresh, PromptSurface::OneShotRun));
    assert!(!should_prompt(&fresh, PromptSurface::SessionChild));
}

#[test]
fn no_surface_prompts_again_after_a_saved_answer() {
    // Given: the saved answers the dialog can write (the ONLY persisting
    // path - dismissals write nothing)
    let answered = [
        TelemetryConfig {
            enabled: false,
            include_technical_details: false,
            asked_on_first_launch: true,
        },
        TelemetryConfig {
            enabled: true,
            include_technical_details: false,
            asked_on_first_launch: true,
        },
        TelemetryConfig {
            enabled: true,
            include_technical_details: true,
            asked_on_first_launch: true,
        },
    ];
    // Then: the question is silenced everywhere, forever
    for config in &answered {
        for surface in ALL_SURFACES {
            assert!(!should_prompt(config, surface));
        }
    }
}

// --- the answer state machine ----------------------------------------------

#[test]
fn defaults_precheck_telemetry_and_leave_details_off() {
    // Given: a fresh dialog model
    let model = ConsentModel::default();
    // Then: the user directive's defaults - telemetry pre-checked (the
    // recommended opt-in, uncheckable before saving), details off ...
    assert!(model.send());
    assert!(!model.details());
    // ... so an unedited "Save choice" is a recorded tier-1 OPT-IN
    assert_eq!(
        model.saved_choice(),
        TelemetryConfig {
            enabled: true,
            include_technical_details: false,
            asked_on_first_launch: true,
        }
    );
}

#[test]
fn save_choice_maps_both_checkboxes_independently() {
    for (send, details) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut model = ConsentModel::default();
        *model.send_mut() = send;
        *model.details_mut() = details;
        assert_eq!(
            model.saved_choice(),
            TelemetryConfig {
                enabled: send,
                include_technical_details: details,
                asked_on_first_launch: true,
            }
        );
    }
}

#[test]
fn only_a_saved_choice_persists_and_dismissals_rearm_the_prompt() {
    // Given: the fresh config of an install that dismissed the dialog
    // ("Not now", Esc, or the window close)
    let fresh = TelemetryConfig::default();
    // Then: the dismissal carries NO config write ...
    assert_eq!(ConsentAction::Dismiss.persisted(), None);
    assert_eq!(ConsentAction::None.persisted(), None);
    // ... so the question stays unanswered and the NEXT daemon start asks
    // again (the user's re-ask directive: no silent-forever deferral)
    assert!(should_prompt(&fresh, PromptSurface::DaemonStartup));
    // While a save carries exactly the checkbox answers, settling the
    // prompt on every surface
    let saved = ConsentModel::default().saved_choice();
    assert_eq!(ConsentAction::Save(saved).persisted(), Some(&saved));
    assert!(!should_prompt(&saved, PromptSurface::DaemonStartup));
}

#[test]
fn an_answered_config_round_trips_to_a_silent_trigger() {
    // The full lifecycle: fresh config prompts -> answer -> the written
    // config (what Config::save persists) never prompts again.
    let fresh = TelemetryConfig::default();
    assert!(should_prompt(&fresh, PromptSurface::DaemonStartup));
    let written = ConsentModel::default().saved_choice();
    assert!(!should_prompt(&written, PromptSurface::DaemonStartup));
}

// --- the widget layer, headless (synthetic raw input) -----------------------

fn test_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::egui_host::theme::fonts());
    let style = settings_style(
        &DesignTokens::default(),
        &UiConfig::default(),
        ThemeMode::Dark,
    );
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
    ctx
}

fn run_frame(
    ctx: &egui::Context,
    model: &mut ConsentModel,
    events: Vec<egui::Event>,
) -> ConsentAction {
    let mut action = ConsentAction::None;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(500.0, 370.0),
        )),
        events,
        focused: true,
        ..Default::default()
    };
    let metrics = FormMetrics::from_tokens(&DesignTokens::default());
    let fill = ctx.style_of(ctx.theme()).visuals.panel_fill;
    let window_margin = metrics.window_margin();
    ctx.run_ui(input, |ui| {
        let panel = egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(fill).inner_margin(window_margin));
        panel.show(ui, |ui| {
            action = widgets::show(ui, model, &metrics);
        });
    })
    .drop_without_applying_deltas();
    action
}

fn key_event(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    }
}

#[test]
fn first_frame_is_inert() {
    let ctx = test_context();
    let mut model = ConsentModel::default();
    assert_eq!(run_frame(&ctx, &mut model, Vec::new()), ConsentAction::None);
}

#[test]
fn escape_dismisses_without_touching_the_checkboxes() {
    let ctx = test_context();
    let mut model = ConsentModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    // Drive BOTH boxes away from the defaults (send off, details on): Esc
    // must still dismiss, never save the current answers.
    *model.send_mut() = false;
    *model.details_mut() = true;
    assert_eq!(
        run_frame(&ctx, &mut model, vec![key_event(egui::Key::Escape)]),
        ConsentAction::Dismiss,
        "Esc is a dismissal: it closes without recording anything"
    );
}

#[test]
fn enter_saves_the_current_answers() {
    let ctx = test_context();
    let mut model = ConsentModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    // Drive both boxes away from the defaults (send off, details on).
    *model.send_mut() = false;
    *model.details_mut() = true;
    assert_eq!(
        run_frame(&ctx, &mut model, vec![key_event(egui::Key::Enter)]),
        ConsentAction::Save(TelemetryConfig {
            enabled: false,
            include_technical_details: true,
            asked_on_first_launch: true,
        })
    );
}

#[test]
fn enter_on_the_untouched_dialog_saves_the_prechecked_choice() {
    let ctx = test_context();
    let mut model = ConsentModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    assert_eq!(
        run_frame(&ctx, &mut model, vec![key_event(egui::Key::Enter)]),
        ConsentAction::Save(TelemetryConfig {
            enabled: true,
            include_technical_details: false,
            asked_on_first_launch: true,
        }),
        "the untouched dialog saves exactly what its checkboxes show"
    );
}

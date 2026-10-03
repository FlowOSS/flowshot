//! Consent-dialog unit tests: the trigger rule (who shows the dialog
//! when), the answer -> config state machine (written once, never shown
//! again; "not now" = both false), and the widget layer driven headlessly
//! through a bare [`egui::Context`] with synthetic raw input (the
//! launcher-tests pattern).

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
fn no_surface_prompts_again_after_any_recorded_answer() {
    // Given: every answer the dialog can write (saved choices + deferral)
    let answered = [
        ConsentModel::deferred(),
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
fn checkboxes_start_unchecked_gdpr_honest() {
    // Given: a fresh dialog model
    let model = ConsentModel::default();
    // Then: nothing is pre-ticked (the recommendation is text only) ...
    assert!(!model.send());
    assert!(!model.details());
    // ... so an unedited "Save choice" is a recorded OPT-OUT
    assert_eq!(
        model.saved_choice(),
        TelemetryConfig {
            enabled: false,
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
fn not_now_records_both_false_and_never_nags_again() {
    // Given/When: the deferred answer ("Not now", Esc, window close)
    let deferred = ConsentModel::deferred();
    // Then: both flags false, the question recorded ...
    assert_eq!(
        deferred,
        TelemetryConfig {
            enabled: false,
            include_technical_details: false,
            asked_on_first_launch: true,
        }
    );
    // ... so the trigger stays silent on every surface
    assert!(!should_prompt(&deferred, PromptSurface::DaemonStartup));
}

#[test]
fn an_answered_config_round_trips_to_a_silent_trigger() {
    // The full lifecycle: fresh config prompts -> answer -> the written
    // config (what Config::save persists) never prompts again.
    let fresh = TelemetryConfig::default();
    assert!(should_prompt(&fresh, PromptSurface::DaemonStartup));
    let mut model = ConsentModel::default();
    *model.send_mut() = true;
    let written = model.saved_choice();
    assert!(!should_prompt(&written, PromptSurface::DaemonStartup));
}

// --- the widget layer, headless (synthetic raw input) -----------------------

fn test_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::egui_host::theme::fonts());
    ctx.set_style(settings_style(
        &DesignTokens::default(),
        &UiConfig::default(),
        ThemeMode::Dark,
    ));
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
            egui::vec2(480.0, 290.0),
        )),
        events,
        focused: true,
        ..Default::default()
    };
    let metrics = FormMetrics::from_tokens(&DesignTokens::default());
    let panel = egui::CentralPanel::default().frame(
        egui::Frame::none()
            .fill(ctx.style().visuals.panel_fill)
            .inner_margin(egui::Margin::same(metrics.window_margin())),
    );
    let _ = ctx.run(input, |ctx| {
        panel.show(ctx, |ui| {
            action = widgets::show(ui, model, &metrics);
        });
    });
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
fn escape_defers_without_touching_the_checkboxes() {
    let ctx = test_context();
    let mut model = ConsentModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    *model.send_mut() = true;
    assert_eq!(
        run_frame(&ctx, &mut model, vec![key_event(egui::Key::Escape)]),
        ConsentAction::NotNow,
        "Esc is a dismissal, not a save of the current answers"
    );
}

#[test]
fn enter_saves_the_current_answers() {
    let ctx = test_context();
    let mut model = ConsentModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    *model.send_mut() = true;
    *model.details_mut() = true;
    assert_eq!(
        run_frame(&ctx, &mut model, vec![key_event(egui::Key::Enter)]),
        ConsentAction::Save(TelemetryConfig {
            enabled: true,
            include_technical_details: true,
            asked_on_first_launch: true,
        })
    );
}

#[test]
fn enter_on_the_untouched_dialog_saves_the_opt_out() {
    let ctx = test_context();
    let mut model = ConsentModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    assert_eq!(
        run_frame(&ctx, &mut model, vec![key_event(egui::Key::Enter)]),
        ConsentAction::Save(ConsentModel::default().saved_choice())
    );
}

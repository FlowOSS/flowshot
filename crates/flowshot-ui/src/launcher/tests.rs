//! Launcher-dialog unit tests: the geometry grammar boundary, the target
//! state machine, the dispatch seam (a chosen mode emits the right typed
//! request), delay carry-through, and the widget layer driven headlessly
//! through a bare [`egui::Context`] with synthetic raw input (the same
//! events the `test-drive` injector feeds the live window).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use egui::Key;
use flowshot_core::config::UiConfig;
use flowshot_core::geometry::{LogicalRect, OutputInfo, PhysicalSize, Transform};
use flowshot_core::tokens::DesignTokens;

use super::model::{LauncherModel, Target};
use super::request::{GeometryIssue, LauncherRequest, RegionGeometry};
use super::ui::{self as widgets, LauncherAction};
use crate::egui_host::theme::{ThemeMode, style};

fn output(connector: &str, name: &str) -> OutputInfo {
    OutputInfo::new(
        connector,
        name,
        LogicalRect::from_raw(0.0, 0.0, 100.0, 100.0),
        PhysicalSize::from_raw(100, 100),
        1.0,
        Transform::Normal,
    )
    .unwrap()
}

fn geometry(width: u32, height: u32, x: Option<i32>, y: Option<i32>) -> RegionGeometry {
    RegionGeometry {
        width,
        height,
        x,
        y,
    }
}

// --- the geometry grammar boundary ---------------------------------------

#[test]
fn parses_the_full_cli_grammar() {
    assert_eq!(
        RegionGeometry::parse("100x100+0+0").unwrap(),
        geometry(100, 100, Some(0), Some(0))
    );
    assert_eq!(
        RegionGeometry::parse("640x480").unwrap(),
        geometry(640, 480, None, None)
    );
    assert_eq!(
        RegionGeometry::parse("100x100-10+20").unwrap(),
        geometry(100, 100, Some(-10), Some(20))
    );
    assert_eq!(
        RegionGeometry::parse("  2560x1440+1920+0  ").unwrap(),
        geometry(2560, 1440, Some(1920), Some(0))
    );
    assert_eq!(
        RegionGeometry::parse("1x1").unwrap(),
        geometry(1, 1, None, None)
    );
}

#[test]
fn rejects_malformed_geometry_with_the_typed_issue() {
    for token in [
        "abc",
        "100",
        "100x",
        "x100",
        "0x100",
        "100x0",
        "100x100+0",
        "100x100+0+0+0",
        "100X100",
        "100x100 0 0",
        "-100x100",
        "100x100+",
        "100x100+a+b",
        "99999999999x100",
        "100x100+99999999999+0",
        "100x100.5+0+0",
    ] {
        assert_eq!(
            RegionGeometry::parse(token),
            Err(GeometryIssue::Malformed),
            "token {token:?} must be malformed"
        );
    }
}

#[test]
fn blank_input_is_the_empty_issue_not_malformed() {
    assert_eq!(RegionGeometry::parse(""), Err(GeometryIssue::Empty));
    assert_eq!(RegionGeometry::parse("   "), Err(GeometryIssue::Empty));
}

#[test]
fn token_format_roundtrips_through_the_parser() {
    for parsed in [
        geometry(100, 100, Some(0), Some(0)),
        geometry(640, 480, None, None),
        geometry(50, 60, Some(-10), Some(-20)),
        geometry(1, 2, Some(i32::MAX), Some(i32::MIN)),
    ] {
        let token = parsed.to_token();
        assert_eq!(
            RegionGeometry::parse(&token).unwrap(),
            parsed,
            "token {token}"
        );
    }
    assert_eq!(
        geometry(100, 100, Some(-10), Some(20)).to_token(),
        "100x100-10+20"
    );
    assert_eq!(geometry(640, 480, None, None).to_token(), "640x480");
}

// --- the target state machine + dispatch seam -----------------------------

#[test]
fn manual_target_with_invalid_geometry_dispatches_nothing() {
    let mut model = LauncherModel::default();
    assert_eq!(model.target(), Target::Manual);
    assert_eq!(model.request(), None);
    *model.geometry_text_mut() = "garbage".to_owned();
    assert_eq!(model.request(), None);
}

#[test]
fn manual_target_with_valid_geometry_dispatches_the_region_request() {
    let mut model = LauncherModel::default();
    *model.geometry_text_mut() = "100x100+0+0".to_owned();
    *model.delay_ms_mut() = 2500;
    assert_eq!(
        model.request(),
        Some(LauncherRequest::Region {
            geometry: geometry(100, 100, Some(0), Some(0)),
            delay_ms: 2500,
        })
    );
}

#[test]
fn monitor_target_dispatches_the_screen_request_regardless_of_geometry() {
    let mut model = LauncherModel::from_outputs(vec![
        output("DP-3", "Dell U2723QE (DP-3)"),
        output("HDMI-A-1", ""),
    ]);
    assert_eq!(
        model
            .monitors()
            .iter()
            .map(|entry| entry.label.clone())
            .collect::<Vec<_>>(),
        vec!["Screen 0: Dell U2723QE (DP-3)", "Screen 1: HDMI-A-1"]
    );
    *model.target_mut() = Target::Monitor(1);
    *model.delay_ms_mut() = 750;
    // The geometry entry is irrelevant (and invalid) in monitor mode.
    *model.geometry_text_mut() = "nonsense".to_owned();
    assert_eq!(
        model.request(),
        Some(LauncherRequest::Screen {
            screen: 1,
            delay_ms: 750,
        })
    );
}

#[test]
fn switching_back_to_manual_revalidates_the_geometry_entry() {
    let mut model = LauncherModel::from_outputs(vec![output("DP-3", "Dell (DP-3)")]);
    *model.target_mut() = Target::Monitor(0);
    assert!(model.request().is_some());
    *model.target_mut() = Target::Manual;
    assert_eq!(model.request(), None);
    *model.geometry_text_mut() = "10x10+5+5".to_owned();
    assert_eq!(
        model.request(),
        Some(LauncherRequest::Region {
            geometry: geometry(10, 10, Some(5), Some(5)),
            delay_ms: 0,
        })
    );
}

#[test]
fn delay_rides_both_request_variants_at_the_u32_boundary() {
    let mut model = LauncherModel::from_outputs(vec![output("DP-3", "Dell (DP-3)")]);
    *model.delay_ms_mut() = u32::MAX;
    *model.geometry_text_mut() = "10x10".to_owned();
    assert_eq!(
        model.request(),
        Some(LauncherRequest::Region {
            geometry: geometry(10, 10, None, None),
            delay_ms: u32::MAX,
        })
    );
    *model.target_mut() = Target::Monitor(0);
    assert_eq!(
        model.request(),
        Some(LauncherRequest::Screen {
            screen: 0,
            delay_ms: u32::MAX,
        })
    );
}

// --- the widget layer, headless (synthetic raw input) ---------------------

fn test_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_style(style(
        &DesignTokens::default(),
        &UiConfig::default(),
        ThemeMode::Dark,
    ));
    ctx
}

fn run_frame(
    ctx: &egui::Context,
    model: &mut LauncherModel,
    events: Vec<egui::Event>,
) -> LauncherAction {
    let mut action = LauncherAction::None;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(400.0, 232.0),
        )),
        events,
        focused: true,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            action = widgets::show(ui, model);
        });
    });
    action
}

fn key_event(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    }
}

#[test]
fn first_frame_is_inert_and_arms_the_geometry_focus() {
    let ctx = test_context();
    let mut model = LauncherModel::default();
    assert_eq!(
        run_frame(&ctx, &mut model, Vec::new()),
        LauncherAction::None
    );
    assert!(!model.take_focus(), "focus arming is one-shot");
    // Typed text lands in the geometry field on the NEXT frame (the field
    // holds the focus the arming requested).
    run_frame(
        &ctx,
        &mut model,
        vec![egui::Event::Text("100x100+0+0".to_owned())],
    );
    assert_eq!(model.geometry_text(), "100x100+0+0");
    assert_eq!(
        model.request(),
        Some(LauncherRequest::Region {
            geometry: geometry(100, 100, Some(0), Some(0)),
            delay_ms: 0,
        })
    );
}

#[test]
fn escape_cancels_without_a_request() {
    let ctx = test_context();
    let mut model = LauncherModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    *model.geometry_text_mut() = "100x100+0+0".to_owned();
    assert_eq!(
        run_frame(&ctx, &mut model, vec![key_event(Key::Escape)]),
        LauncherAction::Cancel
    );
}

#[test]
fn enter_in_the_geometry_field_dispatches_the_typed_request() {
    let ctx = test_context();
    let mut model = LauncherModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    run_frame(
        &ctx,
        &mut model,
        vec![egui::Event::Text("100x100+0+0".to_owned())],
    );
    let action = run_frame(&ctx, &mut model, vec![key_event(Key::Enter)]);
    assert_eq!(
        action,
        LauncherAction::Capture(LauncherRequest::Region {
            geometry: geometry(100, 100, Some(0), Some(0)),
            delay_ms: 0,
        })
    );
}

#[test]
fn enter_with_malformed_geometry_stays_inert() {
    let ctx = test_context();
    let mut model = LauncherModel::default();
    run_frame(&ctx, &mut model, Vec::new());
    run_frame(&ctx, &mut model, vec![egui::Event::Text("oops".to_owned())]);
    let action = run_frame(&ctx, &mut model, vec![key_event(Key::Enter)]);
    assert_eq!(
        action,
        LauncherAction::None,
        "malformed geometry must not dispatch"
    );
}

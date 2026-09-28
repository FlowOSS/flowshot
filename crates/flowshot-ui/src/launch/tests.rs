//! Launch-flow tests: the pure resolution math, the production
//! funnel path (via the `test-drive` injection seam), and the region-memory
//! TOML round-trip through the sink callback.

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use flowshot_core::config::{Config, Region};
use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, LogicalSize, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use winit::event::MouseButton;
use winit::keyboard::{KeyCode, ModifiersState};

use super::*;
use crate::input::{Action, SyntheticInput};
use crate::router::{InputRouter, WindowSlot};
use crate::state::OverlayCore;

/// Dual mixed-DPI fixture (the state.rs precedent): DP-1 1920x1080 @ 1x at
/// (0,0); DP-2 3840x2160 physical @ 2x at logical (1920,0). Union bounds:
/// (0,0,3840,1080).
fn dual_layout() -> OutputLayout {
    let make = |connector: &str, rect: LogicalRect, size: PhysicalSize, scale: f64| {
        OutputInfo::new(connector, connector, rect, size, scale, Transform::Normal)
            .expect("valid fixture output")
    };
    OutputLayout::new(vec![
        make(
            "DP-1",
            LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(1920, 1080),
            1.0,
        ),
        make(
            "DP-2",
            LogicalRect::from_raw(1920.0, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(3840, 2160),
            2.0,
        ),
    ])
}

fn dual_core() -> OverlayCore {
    OverlayCore::new(InputRouter::new(dual_layout(), vec![0, 1]))
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> LogicalRect {
    LogicalRect::from_raw(x, y, width, height)
}

fn point(x: f64, y: f64) -> LogicalPoint {
    LogicalPoint::from_raw(x, y)
}

fn size(width: f64, height: f64) -> LogicalSize {
    LogicalSize::from_raw(width, height)
}

/// The acceptance fixture: `--region 200x100+50+50`.
fn explicit_request(instant: bool) -> LaunchRequest {
    LaunchRequest {
        preselect: Preselect::Region {
            size: size(200.0, 100.0),
            origin: Some(point(50.0, 50.0)),
        },
        instant,
        ..LaunchRequest::default()
    }
}

fn centered_preselect(width: f64, height: f64) -> Preselect {
    Preselect::Region {
        size: size(width, height),
        origin: None,
    }
}

fn move_to(core: &mut OverlayCore, slot: usize, x: f64, y: f64) -> crate::input::RouteReport {
    core.inject_event(SyntheticInput::pointer_moved(WindowSlot::new(slot), x, y))
}

fn button(core: &mut OverlayCore, pressed: bool) -> crate::input::RouteReport {
    core.inject_event(SyntheticInput::pointer_button(
        WindowSlot::new(0),
        MouseButton::Left,
        pressed,
    ))
}

fn tap(core: &mut OverlayCore, key: KeyCode) -> crate::input::RouteReport {
    core.inject_event(SyntheticInput::key_press(WindowSlot::new(0), key))
}

// ---------------------------------------------------------------------------
// The pure resolution math (preselect centers/clamps; logical->physical).
// ---------------------------------------------------------------------------

#[test]
fn explicit_region_seeds_as_given() {
    let preselect = Preselect::Region {
        size: size(200.0, 100.0),
        origin: Some(point(150.0, 100.0)),
    };
    assert_eq!(
        preselect.resolve(None, &dual_layout()),
        InitialSelection::Ready(Some(rect(150.0, 100.0, 200.0, 100.0)))
    );
    // Explicit coordinates win over a resolved cursor (never re-centered).
    assert_eq!(
        preselect.resolve(Some(point(960.0, 540.0)), &dual_layout()),
        InitialSelection::Ready(Some(rect(150.0, 100.0, 200.0, 100.0)))
    );
}

#[test]
fn explicit_region_crops_to_the_layout_intersection() {
    let preselect = Preselect::Region {
        size: size(200.0, 100.0),
        origin: Some(point(3800.0, 1000.0)),
    };
    assert_eq!(
        preselect.resolve(None, &dual_layout()),
        InitialSelection::Ready(Some(rect(3800.0, 1000.0, 40.0, 80.0)))
    );
}

#[test]
fn explicit_region_outside_the_layout_is_no_preselect() {
    let preselect = Preselect::Region {
        size: size(200.0, 100.0),
        origin: Some(point(5000.0, 5000.0)),
    };
    assert_eq!(
        preselect.resolve(None, &dual_layout()),
        InitialSelection::Ready(None)
    );
}

#[test]
fn region_below_the_engine_minimum_expands_to_ten() {
    let preselect = Preselect::Region {
        size: size(5.0, 4.0),
        origin: Some(point(100.0, 100.0)),
    };
    assert_eq!(
        preselect.resolve(None, &dual_layout()),
        InitialSelection::Ready(Some(rect(100.0, 100.0, 10.0, 10.0)))
    );
}

#[test]
fn centered_region_centers_at_the_cursor() {
    let initial =
        centered_preselect(200.0, 100.0).resolve(Some(point(960.0, 540.0)), &dual_layout());
    assert_eq!(
        initial,
        InitialSelection::Ready(Some(rect(860.0, 490.0, 200.0, 100.0)))
    );
}

#[test]
fn centered_region_clamps_at_the_top_left_edge_preserving_size() {
    let initial = centered_preselect(200.0, 100.0).resolve(Some(point(50.0, 30.0)), &dual_layout());
    assert_eq!(
        initial,
        InitialSelection::Ready(Some(rect(0.0, 0.0, 200.0, 100.0)))
    );
}

#[test]
fn centered_region_clamps_at_the_bottom_right_edge_preserving_size() {
    let initial =
        centered_preselect(200.0, 100.0).resolve(Some(point(3830.0, 1070.0)), &dual_layout());
    assert_eq!(
        initial,
        InitialSelection::Ready(Some(rect(3640.0, 980.0, 200.0, 100.0)))
    );
}

#[test]
fn centered_region_oversize_crops_to_the_layout_bounds() {
    let initial =
        centered_preselect(5000.0, 2000.0).resolve(Some(point(960.0, 540.0)), &dual_layout());
    assert_eq!(
        initial,
        InitialSelection::Ready(Some(rect(0.0, 0.0, 3840.0, 1080.0)))
    );
}

#[test]
fn centered_region_defers_when_the_cursor_is_unresolved() {
    let initial = centered_preselect(200.0, 100.0).resolve(None, &dual_layout());
    assert_eq!(
        initial,
        InitialSelection::Deferred(PendingPreselect::CenterRegion {
            size: size(200.0, 100.0)
        })
    );
}

#[test]
fn output_at_cursor_preselects_the_whole_output() {
    let initial = Preselect::OutputAtCursor.resolve(Some(point(2000.0, 500.0)), &dual_layout());
    assert_eq!(
        initial,
        InitialSelection::Ready(Some(rect(1920.0, 0.0, 1920.0, 1080.0)))
    );
}

#[test]
fn output_at_cursor_defers_when_the_cursor_is_unresolved() {
    assert_eq!(
        Preselect::OutputAtCursor.resolve(None, &dual_layout()),
        InitialSelection::Deferred(PendingPreselect::OutputAtCursor)
    );
}

#[test]
fn output_at_cursor_outside_every_output_is_no_preselect() {
    assert_eq!(
        Preselect::OutputAtCursor.resolve(Some(point(5000.0, 5000.0)), &dual_layout()),
        InitialSelection::Ready(None)
    );
}

#[test]
fn last_region_restores_and_a_missing_one_is_no_preselect() {
    let persisted = Region {
        x: 100,
        y: 50,
        width: 200,
        height: 100,
    };
    assert_eq!(
        Preselect::LastRegion(Some(persisted)).resolve(None, &dual_layout()),
        InitialSelection::Ready(Some(rect(100.0, 50.0, 200.0, 100.0)))
    );
    assert_eq!(
        Preselect::LastRegion(None).resolve(None, &dual_layout()),
        InitialSelection::Ready(None)
    );
    // A persisted rect from a departed monitor layout crops/degrades.
    let departed = Region {
        x: 9000,
        y: 9000,
        width: 200,
        height: 100,
    };
    assert_eq!(
        Preselect::LastRegion(Some(departed)).resolve(None, &dual_layout()),
        InitialSelection::Ready(None)
    );
}

#[test]
fn preselect_none_is_a_bare_launch() {
    assert_eq!(
        Preselect::None.resolve(Some(point(10.0, 10.0)), &dual_layout()),
        InitialSelection::Ready(None)
    );
}

#[test]
fn screen_target_picks_the_output_containing_the_cursor() {
    // The `capture screen` no-arg acceptance: the output-at-cursor behavior.
    let layout = dual_layout();
    let output = output_at_cursor(&layout, Some(point(2000.0, 500.0)));
    assert_eq!(output.map(|info| info.connector.as_str()), Some("DP-2"));
    let output = output_at_cursor(&layout, Some(point(960.0, 540.0)));
    assert_eq!(output.map(|info| info.connector.as_str()), Some("DP-1"));
    // Unresolved cursor (AwaitFirstMotion) and off-layout: the caller owns
    // the fallback.
    assert!(output_at_cursor(&layout, None).is_none());
    assert!(output_at_cursor(&layout, Some(point(5000.0, 5000.0))).is_none());
}

#[test]
fn logical_region_exports_physical_first_per_output_scale() {
    // --region coordinate space is global LOGICAL px; the export converts
    // with EACH output's own scale (never averaged) - the scale-2 contract.
    let layout = dual_layout();
    let crops = layout.crop_rects(rect(2000.0, 100.0, 200.0, 100.0));
    assert_eq!(crops.len(), 1);
    assert_eq!(crops[0].output.connector, "DP-2");
    assert_eq!(
        (
            crops[0].physical.x.0,
            crops[0].physical.y.0,
            crops[0].physical.width.0,
            crops[0].physical.height.0
        ),
        (160, 200, 400, 200)
    );
    // Spanning rect: DP-1 crops 1:1, DP-2 crops at 2x - one logical rect,
    // per-output physical-first (#4894 spanning + #4871 scale fix).
    let crops = layout.crop_rects(rect(1820.0, 100.0, 200.0, 100.0));
    assert_eq!(crops.len(), 2);
    assert_eq!(crops[0].output.connector, "DP-1");
    assert_eq!(
        (
            crops[0].physical.x.0,
            crops[0].physical.y.0,
            crops[0].physical.width.0,
            crops[0].physical.height.0
        ),
        (1820, 100, 100, 100)
    );
    assert_eq!(crops[1].output.connector, "DP-2");
    assert_eq!(
        (
            crops[1].physical.x.0,
            crops[1].physical.y.0,
            crops[1].physical.width.0,
            crops[1].physical.height.0
        ),
        (0, 200, 200, 200)
    );
}

#[test]
fn region_conversion_roundtrips_and_rejects_out_of_range() {
    let region = resolve::region_of(rect(50.4, -20.6, 200.0, 100.0));
    assert_eq!(
        region,
        Some(Region {
            x: 50,
            y: -21,
            width: 200,
            height: 100
        })
    );
    assert_eq!(resolve::region_of(rect(f64::NAN, 0.0, 10.0, 10.0)), None);
    assert_eq!(resolve::region_of(rect(1e30, 0.0, 10.0, 10.0)), None);
}

#[test]
fn tracing_tokens_are_stable() {
    // QA log asserts consume these tokens (structural, never prose).
    assert_eq!(Preselect::None.token(), "none");
    assert_eq!(centered_preselect(10.0, 10.0).token(), "region-centered");
    assert_eq!(
        Preselect::Region {
            size: size(10.0, 10.0),
            origin: Some(point(0.0, 0.0))
        }
        .token(),
        "region-explicit"
    );
    assert_eq!(Preselect::OutputAtCursor.token(), "output-at-cursor");
    assert_eq!(Preselect::LastRegion(None).token(), "last-region-absent");
    assert_eq!(
        PendingPreselect::CenterRegion {
            size: size(10.0, 10.0)
        }
        .token(),
        "center-region"
    );
    assert_eq!(PendingPreselect::OutputAtCursor.token(), "output-at-cursor");
}

// ---------------------------------------------------------------------------
// The production funnel path (OverlayCore + inject_event).
// ---------------------------------------------------------------------------

#[test]
fn launch_seeds_the_selection_engine() {
    let mut core = dual_core();
    core.launch(explicit_request(false));
    assert_eq!(
        core.selection().rect(),
        Some(rect(50.0, 50.0, 200.0, 100.0))
    );
}

#[test]
fn launch_before_the_layout_applies_at_the_spawn_hook() {
    // Production order: the binary layer calls launch() before run(); the
    // router layout arrives on Resumed (handler spawn -> apply_launch).
    let mut core = OverlayCore::new(InputRouter::new(OutputLayout::new(Vec::new()), Vec::new()));
    core.launch(LaunchRequest {
        preselect: centered_preselect(200.0, 100.0),
        cursor: Some(point(960.0, 540.0)),
        ..LaunchRequest::default()
    });
    assert_eq!(core.selection().rect(), None);
    core.router_mut().install(dual_layout(), vec![0, 1]);
    core.apply_launch();
    assert_eq!(
        core.selection().rect(),
        Some(rect(860.0, 490.0, 200.0, 100.0))
    );
    // The hook is idempotent (a second call must not re-seed).
    core.selection_mut().set_rect(None);
    core.apply_launch();
    assert_eq!(core.selection().rect(), None);
}

#[test]
fn await_first_motion_defers_then_applies_on_the_first_injected_motion() {
    let mut core = dual_core();
    core.launch(LaunchRequest {
        preselect: centered_preselect(200.0, 100.0),
        cursor: None,
        ..LaunchRequest::default()
    });
    assert_eq!(core.selection().rect(), None);
    // Slot 0 is scale 1 at the origin: local physical == global logical.
    let report = move_to(&mut core, 0, 960.0, 540.0);
    assert_eq!(
        core.selection().rect(),
        Some(rect(860.0, 490.0, 200.0, 100.0))
    );
    // The seeded selection spans windows: EVERY window redraws.
    assert!(report.actions.contains(&Action::Redraw(WindowSlot::new(0))));
    assert!(report.actions.contains(&Action::Redraw(WindowSlot::new(1))));
}

#[test]
fn await_first_motion_resolves_the_output_at_cursor() {
    let mut core = dual_core();
    core.launch(LaunchRequest {
        preselect: Preselect::OutputAtCursor,
        cursor: None,
        ..LaunchRequest::default()
    });
    assert_eq!(core.selection().rect(), None);
    // Slot 1 = DP-2 (scale 2, logical origin (1920,0)): local physical
    // (100,100) -> global logical (1970,50) -> DP-2.
    move_to(&mut core, 1, 100.0, 100.0);
    assert_eq!(
        core.selection().rect(),
        Some(rect(1920.0, 0.0, 1920.0, 1080.0))
    );
}

#[test]
fn deferred_preselect_is_one_shot() {
    let mut core = dual_core();
    core.launch(LaunchRequest {
        preselect: centered_preselect(200.0, 100.0),
        cursor: None,
        ..LaunchRequest::default()
    });
    move_to(&mut core, 0, 960.0, 540.0);
    let seeded = core.selection().rect();
    // A later motion is just a motion: the pending preselect is consumed.
    move_to(&mut core, 0, 100.0, 100.0);
    assert_eq!(core.selection().rect(), seeded);
}

#[test]
fn instant_accepts_on_the_first_release_over_the_preselect() {
    // The plan acceptance shape: --region 200x100+50+50 --instant, click at
    // (150,100) -> Accept with EXACTLY the preselected rect.
    let mut core = dual_core();
    core.launch(explicit_request(true));
    move_to(&mut core, 0, 150.0, 100.0);
    button(&mut core, true);
    let report = button(&mut core, false);
    assert!(report.actions.contains(&Action::Accept));
    assert_eq!(
        core.selection().rect(),
        Some(rect(50.0, 50.0, 200.0, 100.0))
    );
}

#[test]
fn instant_accepts_a_dragged_selection_on_release() {
    let mut core = dual_core();
    core.launch(LaunchRequest {
        instant: true,
        ..LaunchRequest::default()
    });
    move_to(&mut core, 0, 100.0, 100.0);
    button(&mut core, true);
    move_to(&mut core, 0, 300.0, 250.0);
    let report = button(&mut core, false);
    assert!(report.actions.contains(&Action::Accept));
    assert_eq!(
        core.selection().rect(),
        Some(rect(100.0, 100.0, 200.0, 150.0))
    );
}

#[test]
fn instant_click_outside_the_preselect_clears_without_accepting() {
    let mut core = dual_core();
    core.launch(explicit_request(true));
    move_to(&mut core, 0, 500.0, 500.0);
    button(&mut core, true);
    let report = button(&mut core, false);
    assert!(!report.actions.contains(&Action::Accept));
    assert_eq!(core.selection().rect(), None);
    // The arming survives: the next drag's release accepts.
    button(&mut core, true);
    move_to(&mut core, 0, 700.0, 700.0);
    let report = button(&mut core, false);
    assert!(report.actions.contains(&Action::Accept));
    assert_eq!(
        core.selection().rect(),
        Some(rect(500.0, 500.0, 200.0, 200.0))
    );
}

#[test]
fn instant_fires_once_and_enter_disarms_it() {
    let mut core = dual_core();
    core.launch(explicit_request(true));
    move_to(&mut core, 0, 150.0, 100.0);
    button(&mut core, true);
    let report = button(&mut core, false);
    assert!(report.actions.contains(&Action::Accept));
    // A second release never re-fires (the session is completing).
    button(&mut core, true);
    let report = button(&mut core, false);
    assert!(!report.actions.contains(&Action::Accept));
    // Enter on a preselect disarms the instant release too (any accept
    // completes the session).
    let mut core = dual_core();
    core.launch(explicit_request(true));
    let report = tap(&mut core, KeyCode::Enter);
    assert!(report.actions.contains(&Action::Accept));
    move_to(&mut core, 0, 150.0, 100.0);
    button(&mut core, true);
    let report = button(&mut core, false);
    assert!(!report.actions.contains(&Action::Accept));
}

#[test]
fn enter_on_preselect_accepts_without_instant() {
    let mut core = dual_core();
    core.launch(explicit_request(false));
    let report = tap(&mut core, KeyCode::Enter);
    assert!(report.actions.contains(&Action::Accept));
}

#[test]
fn release_without_instant_never_accepts() {
    let mut core = dual_core();
    core.launch(explicit_request(false));
    move_to(&mut core, 0, 150.0, 100.0);
    button(&mut core, true);
    let report = button(&mut core, false);
    assert!(!report.actions.contains(&Action::Accept));
    assert_eq!(
        core.selection().rect(),
        Some(rect(50.0, 50.0, 200.0, 100.0))
    );
}

// ---------------------------------------------------------------------------
// Region memory: the sink seam + the TOML round-trip.
// ---------------------------------------------------------------------------

fn recording_sink() -> (RegionSink, Arc<Mutex<Option<Region>>>) {
    let recorded = Arc::new(Mutex::new(None));
    let sink_target = Arc::clone(&recorded);
    let sink: RegionSink = Box::new(move |region: Region| {
        *sink_target.lock().unwrap() = Some(region);
    });
    (sink, recorded)
}

#[test]
fn region_memory_persists_on_accept_and_copy_only_when_enabled() {
    let mut core = dual_core();
    core.launch(LaunchRequest {
        save_last_region: true,
        ..explicit_request(false)
    });
    let (sink, recorded) = recording_sink();
    core.set_region_sink(Some(sink));
    tap(&mut core, KeyCode::Enter);
    assert_eq!(
        *recorded.lock().unwrap(),
        Some(Region {
            x: 50,
            y: 50,
            width: 200,
            height: 100
        })
    );
    // Ctrl+C (a capture-completing copy) persists too.
    *recorded.lock().unwrap() = None;
    core.inject_event(SyntheticInput::modifiers(
        WindowSlot::new(0),
        ModifiersState::CONTROL,
    ));
    tap(&mut core, KeyCode::KeyC);
    assert_eq!(
        *recorded.lock().unwrap(),
        Some(Region {
            x: 50,
            y: 50,
            width: 200,
            height: 100
        })
    );
    // save_last_region = false: the sink never fires.
    let mut core = dual_core();
    core.launch(explicit_request(false));
    let (sink, recorded) = recording_sink();
    core.set_region_sink(Some(sink));
    tap(&mut core, KeyCode::Enter);
    assert_eq!(*recorded.lock().unwrap(), None);
}

#[test]
fn instant_release_persists_the_region() {
    let mut core = dual_core();
    core.launch(LaunchRequest {
        save_last_region: true,
        ..explicit_request(true)
    });
    let (sink, recorded) = recording_sink();
    core.set_region_sink(Some(sink));
    move_to(&mut core, 0, 150.0, 100.0);
    button(&mut core, true);
    let report = button(&mut core, false);
    assert!(report.actions.contains(&Action::Accept));
    assert_eq!(
        *recorded.lock().unwrap(),
        Some(Region {
            x: 50,
            y: 50,
            width: 200,
            height: 100
        })
    );
}

#[test]
fn last_region_persist_restore_roundtrip_via_toml() {
    // The plan acceptance: persist the last rect in the TOML, restore it via
    // --last-region. The sink owns the file (the DrawColorSink pattern; the
    // binary layer installs the same closure in production).
    static NEXT_ID: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "flowshot-launch-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");

    let mut core = dual_core();
    core.launch(LaunchRequest {
        save_last_region: true,
        ..explicit_request(false)
    });
    core.set_region_sink(Some(Box::new({
        let path = path.clone();
        move |region: Region| {
            let mut config = Config::load(&path).unwrap_or_default();
            config.capture.last_region = Some(region);
            config.save(&path).unwrap();
        }
    })));
    tap(&mut core, KeyCode::Enter);

    // The TOML carries the rect; a fresh launch restores it (clamped).
    let loaded = Config::load(&path).unwrap();
    assert_eq!(
        loaded.capture.last_region,
        Some(Region {
            x: 50,
            y: 50,
            width: 200,
            height: 100
        })
    );
    let mut restored = dual_core();
    restored.launch(LaunchRequest {
        preselect: Preselect::LastRegion(loaded.capture.last_region),
        ..LaunchRequest::default()
    });
    assert_eq!(
        restored.selection().rect(),
        Some(rect(50.0, 50.0, 200.0, 100.0))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

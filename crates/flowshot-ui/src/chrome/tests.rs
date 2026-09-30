//! The chrome acceptance table: the toolbar config order and
//! anchor/flip geometry, the color-wheel circular geometry and pick flow
//! (color + selected-object mutate + persistence sink, each ONE undo unit),
//! the side-panel per-tool control visibility table, the config gate, the
//! Space/Esc visibility contracts, and the layer list (click = select,
//! drag = reorder = ONE undo unit, raise/lower buttons).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use std::f32::consts::{PI, TAU};
use std::sync::{Arc, Mutex};

use super::*;
use crate::editor::{
    EditorTools, MAX_TOOL_SIZE, Tool, ToolKind, register_pixelate_tools, register_shape_tools,
    register_text_tool, scene_color_from_hex,
};
use crate::input::SyntheticInput;
use crate::render::{DisplayList, Rect};
use crate::router::{InputRouter, WindowSlot};
use crate::state::OverlayCore;
use crate::widgets::ICON_ATLAS_ID;
use flowshot_core::config::UiConfig;
use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::scene::{Color as SceneColor, Rect as SceneRect, RectObject, ToolObjectData};
use winit::event::MouseButton;
use winit::keyboard::{KeyCode, ModifiersState};

const SLOT: WindowSlot = WindowSlot::new(0);
/// The selection every funnel fixture carries (global logical; slot 0 is
/// 1920x1080 @ 1x at (0,0), so local physical == global logical).
const SEL: LogicalRect = LogicalRect::from_raw(50.0, 150.0, 600.0, 450.0);

#[derive(Debug)]
struct CounterStub;
impl Tool for CounterStub {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Counter
    }
}
#[derive(Debug)]
struct SelectionStub;
impl Tool for SelectionStub {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Selection
    }
}
#[derive(Debug)]
struct MoveStub;
impl Tool for MoveStub {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Move
    }
}

fn fixture_output() -> OutputInfo {
    OutputInfo::new(
        "DP-1",
        "DP-1",
        LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
        PhysicalSize::from_raw(1920, 1080),
        1.0,
        Transform::Normal,
    )
    .expect("valid fixture output")
}

fn fixture() -> OverlayCore {
    let layout = OutputLayout::new(vec![fixture_output()]);
    let mut core = OverlayCore::new(InputRouter::new(layout, vec![0]));
    let registry = core.editor_mut().registry_mut();
    register_shape_tools(registry);
    register_text_tool(registry);
    register_pixelate_tools(registry);
    registry.register(ToolKind::Counter, || Box::new(CounterStub));
    registry.register(ToolKind::Selection, || Box::new(SelectionStub));
    registry.register(ToolKind::Move, || Box::new(MoveStub));
    core.selection_mut().set_rect(Some(SEL));
    core.sync_cascade();
    core
}

fn move_to(core: &mut OverlayCore, x: f64, y: f64) {
    core.inject_event(SyntheticInput::pointer_moved(SLOT, x, y));
}

fn click(core: &mut OverlayCore, button: MouseButton, pressed: bool) {
    core.inject_event(SyntheticInput::pointer_button(SLOT, button, pressed));
}

fn left_click_at(core: &mut OverlayCore, x: f64, y: f64) {
    move_to(core, x, y);
    click(core, MouseButton::Left, true);
    click(core, MouseButton::Left, false);
}

fn tap(core: &mut OverlayCore, key_code: KeyCode) {
    core.inject_event(SyntheticInput::key_press(SLOT, key_code));
}

fn ctrl_z(core: &mut OverlayCore) {
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::CONTROL));
    core.inject_event(SyntheticInput::key_press(SLOT, KeyCode::KeyZ));
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::empty()));
}

fn commit_rect(core: &mut OverlayCore, x: f32, y: f32) -> usize {
    core.editor_mut().commit_object(Box::new(RectObject::new(
        SceneRect::new(x, y, 60.0, 40.0),
        SceneColor::new(255, 0, 0, 255),
        2.0,
        false,
    )))
}

fn rect_data(core: &OverlayCore, id: usize) -> RectObject {
    match core.editor().scene().get_object(id).unwrap().to_data() {
        ToolObjectData::Rectangle(object) => object,
        other => panic!("fixture object is a rectangle, got {other:?}"),
    }
}

fn center(rect: Rect) -> (f64, f64) {
    (f64::from(rect.center().x), f64::from(rect.center().y))
}

fn panel_layout(core: &OverlayCore) -> SidePanelLayout {
    side_panel::layout(
        core.editor(),
        core.selection().rect().unwrap(),
        core.chrome().tokens(),
        1.0,
        &fixture_output(),
    )
}

fn toolbar_buttons(core: &OverlayCore, index: usize) -> Rect {
    let (_, buttons) = core.chrome().toolbar.layout(
        core.selection().rect().unwrap(),
        core.chrome().tokens(),
        1.0,
        &fixture_output(),
    );
    buttons[index]
}

fn open_panel(core: &mut OverlayCore) {
    tap(core, KeyCode::Space);
    assert!(
        core.chrome().panel_shown(core.editor()),
        "Space opens the panel"
    );
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

#[test]
fn toolbar_layout_from_config() {
    let mut chrome = ChromeState::new();
    let config = UiConfig {
        toolbar_buttons: vec![
            "pencil".to_string(),
            "unknown".to_string(),
            "copy".to_string(),
        ],
        ..UiConfig::default()
    };
    chrome.configure(&config);

    assert_eq!(chrome.toolbar.buttons.len(), 3);
    assert_eq!(
        chrome.toolbar.buttons[0],
        crate::chrome::toolbar::ToolbarButton::Tool(ToolKind::Pencil)
    );
    assert_eq!(
        chrome.toolbar.buttons[1],
        crate::chrome::toolbar::ToolbarButton::Action("unknown".to_string())
    );
    assert_eq!(
        chrome.toolbar.buttons[2],
        crate::chrome::toolbar::ToolbarButton::Action("copy".to_string())
    );
}

#[test]
fn toolbar_order_reshuffles_on_config_reload() {
    // Plan acceptance: "button order shuffles after config edit + reload".
    let mut chrome = ChromeState::new();
    assert_eq!(
        chrome.toolbar.buttons.len(),
        UiConfig::default().toolbar_buttons.len(),
        "the default projection seeds the config order"
    );
    chrome.configure(&UiConfig {
        toolbar_buttons: vec!["undo".to_string(), "pencil".to_string()],
        ..UiConfig::default()
    });
    assert_eq!(
        chrome.toolbar.buttons,
        vec![
            ToolbarButton::Action("undo".to_string()),
            ToolbarButton::Tool(ToolKind::Pencil)
        ]
    );
    chrome.configure(&UiConfig {
        toolbar_buttons: vec!["pencil".to_string(), "undo".to_string()],
        ..UiConfig::default()
    });
    assert_eq!(
        chrome.toolbar.buttons[0],
        ToolbarButton::Tool(ToolKind::Pencil)
    );
    assert_eq!(
        chrome.toolbar.buttons[1],
        ToolbarButton::Action("undo".to_string())
    );
}

#[test]
fn toolbar_anchors_below_flips_above_and_stays_on_screen() {
    // Plan acceptance: anchored below mid-screen, flipped above near the
    // bottom edge; failure QA: extreme corner -> fully on-screen.
    let chrome = ChromeState::new();
    let tokens = chrome.tokens();
    let output = fixture_output();
    let layout_at = |sel: LogicalRect| chrome.toolbar.layout(sel, tokens, 1.0, &output);

    let mid = layout_at(LogicalRect::from_raw(400.0, 200.0, 600.0, 450.0)).0;
    assert!(
        mid.origin.y > 200.0 + 450.0,
        "below the selection mid-screen"
    );

    let low = layout_at(LogicalRect::from_raw(400.0, 600.0, 600.0, 450.0)).0;
    assert!(
        low.origin.y + low.size.height <= 600.0,
        "flipped above near the bottom edge"
    );

    let corner = layout_at(LogicalRect::from_raw(0.0, 1000.0, 100.0, 72.0)).0;
    assert!(corner.origin.x >= 0.0 && corner.origin.y >= 0.0);
    assert!(
        corner.origin.x + corner.size.width <= 1920.0
            && corner.origin.y + corner.size.height <= 1080.0,
        "extreme corner: fully on-screen, got {corner:?}"
    );
}

#[test]
fn toolbar_tooltip_waits_the_delay_then_shows_the_active_binding() {
    let mut core = fixture();
    let (x, y) = center(toolbar_buttons(&core, 0)); // default order: arrow
    move_to(&mut core, x, y);
    let hovered = std::time::Instant::now();

    let painted_texts = |core: &OverlayCore, now: std::time::Instant| -> Vec<String> {
        let mut list = DisplayList::new();
        core.chrome().paint_into(
            &mut list,
            core.editor(),
            core.selection(),
            ICON_ATLAS_ID,
            &fixture_output(),
            now,
        );
        list.iter()
            .filter_map(|command| match command {
                crate::render::Command::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect()
    };

    // During the 400ms delay the toolbar paints no text at all.
    assert!(painted_texts(&core, hovered).is_empty());
    let shown = hovered + crate::widgets::TOOLTIP_DELAY;
    assert!(
        painted_texts(&core, shown).contains(&"Arrow — pointed arrow [A]".to_owned()),
        "the tooltip shows the description plus the bound key"
    );

    // A rebind moves the tooltip key (it reads the ACTIVE binding).
    core.editor_mut()
        .shortcuts_mut()
        .rebind(ToolKind::Arrow, Some(KeyCode::KeyW));
    assert!(painted_texts(&core, shown).contains(&"Arrow — pointed arrow [W]".to_owned()));

    // Leaving the button hides the tooltip.
    move_to(&mut core, x, y + 200.0);
    assert!(painted_texts(&core, shown + crate::widgets::TOOLTIP_DELAY).is_empty());
}

#[test]
fn toolbar_tool_button_activates_and_blocks_the_draw_underneath() {
    // Widget parity: a press on the toolbar never reaches the F27 chain -
    // the pencil (active!) starts no stroke and the button's tool activates.
    let mut core = fixture();
    tap(&mut core, KeyCode::KeyP);
    assert_eq!(core.editor().active_tool(), Some(ToolKind::Pencil));
    let (x, y) = center(toolbar_buttons(&core, 0)); // default order: arrow
    move_to(&mut core, x, y);
    click(&mut core, MouseButton::Left, true);
    assert!(
        !core.editor().is_drawing(),
        "no draw session under the toolbar"
    );
    move_to(&mut core, x + 40.0, y + 40.0);
    click(&mut core, MouseButton::Left, false);
    assert_eq!(core.editor().scene().object_count(), 0);
    assert_eq!(
        core.selection().rect(),
        Some(SEL),
        "the press never reached the region engine"
    );
    assert_eq!(core.editor().active_tool(), Some(ToolKind::Arrow));
}

#[test]
fn toolbar_undo_button_steps_the_journal() {
    let mut core = fixture();
    commit_rect(&mut core, 100.0, 200.0);
    assert_eq!(core.editor().scene().object_count(), 1);
    let (x, y) = center(toolbar_buttons(&core, 11)); // default order: undo
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().scene().object_count(), 0);
}

// ---------------------------------------------------------------------------
// Color wheel
// ---------------------------------------------------------------------------

#[test]
fn color_wheel_propagation() {
    let mut chrome = ChromeState::new();
    let pos = LogicalPoint::new(
        flowshot_core::geometry::Logical(100.0),
        flowshot_core::geometry::Logical(100.0),
    );
    chrome.show_color_wheel(pos);
    assert!(chrome.color_wheel.visible);
    assert_eq!(chrome.color_wheel.position, pos);
    chrome.hide_color_wheel();
    assert!(!chrome.color_wheel.visible);
}

#[test]
fn wheel_geometry_follows_the_f27_formula() {
    // radius = 3*count + buttonBaseSize; equal angles; rainbow trails.
    let mut core = fixture();
    let at = LogicalPoint::from_raw(960.0, 540.0);
    core.chrome_mut().show_color_wheel(at);
    let wheel = core.chrome().color_wheel.layout(
        core.editor(),
        core.chrome().tokens(),
        1.0,
        &fixture_output(),
    );
    let count = core.editor().config().editor.color_palette.len();
    assert_eq!(wheel.swatches.len(), count);
    let radius = 3.0 * count as f32 + BUTTON_BASE_SIZE;
    let hub = wheel.rect.center();
    let slots = count + 1;
    for (index, swatch) in wheel.swatches.iter().enumerate() {
        let expected = index as f32 * TAU / slots as f32 - PI / 2.0;
        let c = swatch.center();
        assert!((c.x - (hub.x + radius * expected.cos())).abs() < 0.01);
        assert!((c.y - (hub.y + radius * expected.sin())).abs() < 0.01);
        assert!(wheel.rect.contains(c), "bounds contain every swatch");
    }
    let expected = count as f32 * TAU / slots as f32 - PI / 2.0;
    let rainbow = wheel.rainbow.center();
    assert!((rainbow.x - (hub.x + radius * expected.cos())).abs() < 0.01);
    assert!((rainbow.y - (hub.y + radius * expected.sin())).abs() < 0.01);
}

#[test]
fn right_click_opens_the_wheel_and_a_swatch_pick_writes_every_seam() {
    // Plan acceptance: pick color -> draw color changes; F27: the pick
    // persists (sink) and the selected object recolors as ONE undo unit.
    let mut core = fixture();
    let picked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&picked);
    core.chrome_mut()
        .set_draw_color_sink(Some(Box::new(move |hex: &str| {
            sink.lock().unwrap().push(hex.to_owned());
        })));
    let id = commit_rect(&mut core, 100.0, 200.0);
    core.editor_mut().select_layer(id);
    let depth_before = core.editor().undo_stack().undo_depth();

    move_to(&mut core, 300.0, 300.0);
    click(&mut core, MouseButton::Right, true);
    click(&mut core, MouseButton::Right, false);
    assert!(core.chrome().color_wheel.visible);
    assert!(
        core.selection().cascade().picker_visible(),
        "stage 5 synced"
    );

    let wheel = core.chrome().color_wheel.layout(
        core.editor(),
        core.chrome().tokens(),
        1.0,
        &fixture_output(),
    );
    let hex = core.editor().config().editor.color_palette[7].clone();
    let (x, y) = center(wheel.swatches[7]);
    left_click_at(&mut core, x, y);

    let expected = scene_color_from_hex(&hex).unwrap();
    assert_eq!(core.editor().color(), expected);
    let color = rect_data(&core, id).color;
    assert_eq!(
        (color.r, color.g, color.b),
        (expected.r, expected.g, expected.b)
    );
    assert_eq!(
        core.editor().undo_stack().undo_depth(),
        depth_before + 1,
        "the recolor is exactly ONE undo unit"
    );
    assert_eq!(*picked.lock().unwrap(), vec![hex]);
    assert!(
        !core.chrome().color_wheel.visible,
        "the pick hides the wheel"
    );
    assert!(!core.selection().cascade().picker_visible());

    ctrl_z(&mut core);
    let color = rect_data(&core, id).color;
    assert_eq!((color.r, color.g, color.b), (255, 0, 0), "undo restores");
}

// ---------------------------------------------------------------------------
// Side panel: visibility, gate, per-tool table
// ---------------------------------------------------------------------------

#[test]
fn space_toggles_the_panel_and_esc_hides_it() {
    let mut core = fixture();
    assert!(
        !core.chrome().panel_shown(core.editor()),
        "hidden by default"
    );
    open_panel(&mut core);
    assert!(core.selection().cascade().panel_visible(), "stage 3 synced");
    // No tool/object occupied: the first Esc pops the panel stage.
    tap(&mut core, KeyCode::Escape);
    assert!(!core.chrome().panel_shown(core.editor()));
    assert!(!core.selection().cascade().panel_visible());
    assert!(!core.exit_requested(), "the cascade did not reach close");
    // Space reopens (the toggle is reversible).
    open_panel(&mut core);
}

#[test]
fn panel_config_gate_hides_everything() {
    let mut core = fixture();
    commit_rect(&mut core, 100.0, 200.0);
    open_panel(&mut core);
    let mut shown = DisplayList::new();
    core.chrome().paint_into(
        &mut shown,
        core.editor(),
        core.selection(),
        ICON_ATLAS_ID,
        &fixture_output(),
        std::time::Instant::now(),
    );

    let mut tools: EditorTools = core.editor().config().clone();
    tools.editor.side_panel = false;
    core.editor_mut().configure(tools);
    core.sync_cascade();

    assert!(!core.chrome().panel_shown(core.editor()));
    assert!(!core.selection().cascade().panel_visible(), "stage 3 empty");
    tap(&mut core, KeyCode::Space);
    assert!(!core.chrome().panel_shown(core.editor()), "Space is inert");
    let mut hidden = DisplayList::new();
    core.chrome().paint_into(
        &mut hidden,
        core.editor(),
        core.selection(),
        ICON_ATLAS_ID,
        &fixture_output(),
        std::time::Instant::now(),
    );
    assert!(hidden.len() < shown.len(), "the panel paints nothing");
    // A press where the panel row sat falls through to the F27 chain.
    let row = panel_layout(&core).layer_rows[0].1;
    let (x, y) = center(row);
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().selected_object(), None);
}

#[test]
fn per_tool_control_visibility_table() {
    let mut core = fixture();
    open_panel(&mut core);
    // (kind, size slider, arrow rows, counter outline, pixelate mode)
    let table: [(ToolKind, bool, bool, bool, bool); 13] = [
        (ToolKind::Pencil, true, false, false, false),
        (ToolKind::Line, true, false, false, false),
        (ToolKind::Arrow, true, true, false, false),
        (ToolKind::Selection, false, false, false, false),
        (ToolKind::Rectangle, true, false, false, false),
        (ToolKind::Circle, true, false, false, false),
        (ToolKind::Marker, true, false, false, false),
        (ToolKind::Text, true, false, false, false),
        (ToolKind::Counter, true, false, true, false),
        (ToolKind::Pixelate, true, false, false, true),
        (ToolKind::Blur, true, false, false, true),
        (ToolKind::Invert, false, false, false, false),
        (ToolKind::Move, false, false, false, false),
    ];
    for (kind, size, arrow, counter, pixelate) in table {
        core.editor_mut().activate_tool(kind);
        let panel = panel_layout(&core);
        assert_eq!(panel.size_slider.is_some(), size, "{kind:?} size slider");
        assert_eq!(panel.arrow_style.is_some(), arrow, "{kind:?} arrow style");
        assert_eq!(
            panel.arrow_reverse.is_some(),
            arrow,
            "{kind:?} arrow reverse"
        );
        assert_eq!(
            panel.counter_outline.is_some(),
            counter,
            "{kind:?} counter outline"
        );
        assert_eq!(
            panel.pixelate_mode.is_some(),
            pixelate,
            "{kind:?} pixelate mode"
        );
        // The layer section is tool-independent.
        assert!(panel.raise_button.is_some() && panel.lower_button.is_some());
    }
    core.editor_mut().deactivate_tool();
    let panel = panel_layout(&core);
    assert!(panel.size_slider.is_none(), "no tool: no tool options");
    assert!(panel.raise_button.is_some());
}

#[test]
fn size_label_table_matches_the_dispatch_slots() {
    assert_eq!(size_label(ToolKind::Pencil), Some("Thickness"));
    assert_eq!(size_label(ToolKind::Rectangle), Some("Corner radius"));
    assert_eq!(size_label(ToolKind::Text), Some("Font size"));
    assert_eq!(size_label(ToolKind::Marker), Some("Marker size"));
    assert_eq!(size_label(ToolKind::Pixelate), Some("Block size"));
    assert_eq!(size_label(ToolKind::Counter), Some("Counter size"));
    assert_eq!(size_label(ToolKind::Invert), None);
    assert_eq!(size_label(ToolKind::Selection), None);
    assert_eq!(size_label(ToolKind::Move), None);
}

// ---------------------------------------------------------------------------
// Side panel controls -> the editor seams
// ---------------------------------------------------------------------------

#[test]
fn size_slider_writes_the_slot_and_the_object_as_one_unit() {
    let mut core = fixture();
    let id = commit_rect(&mut core, 100.0, 200.0);
    open_panel(&mut core);
    tap(&mut core, KeyCode::KeyR); // rectangle: the slot IS the corner radius
    core.editor_mut().select_layer(id);
    let depth_before = core.editor().undo_stack().undo_depth();

    let slider = panel_layout(&core).size_slider.unwrap();
    let (x, y) = center(slider); // 50% -> size 25
    left_click_at(&mut core, x, y);

    assert_eq!(core.editor().tool_size(), MAX_TOOL_SIZE / 2);
    assert_eq!(rect_data(&core, id).corner_radius, 25.0);
    assert_eq!(
        core.editor().undo_stack().undo_depth(),
        depth_before + 1,
        "the property change is exactly ONE undo unit"
    );
    ctrl_z(&mut core);
    assert_eq!(rect_data(&core, id).corner_radius, 0.0, "undo restores");
    assert_eq!(
        core.editor().tool_size(),
        MAX_TOOL_SIZE / 2,
        "the runtime slot is not journaled (the tool-framework contract)"
    );
}

#[test]
fn arrow_rows_write_config_and_preserve_the_runtime_size() {
    let mut core = fixture();
    open_panel(&mut core);
    tap(&mut core, KeyCode::KeyA);
    core.inject_event(SyntheticInput::wheel(SLOT, 120)); // 3 -> 4
    assert_eq!(core.editor().tool_size(), 4);

    let panel = panel_layout(&core);
    let (x, y) = center(panel.arrow_style.unwrap());
    left_click_at(&mut core, x, y);
    assert_eq!(
        core.editor().config().tools.arrow.style,
        flowshot_core::config::ArrowStyle::Curved
    );
    assert_eq!(
        core.editor().tool_size(),
        4,
        "configure() must not stomp the wheel adjustment"
    );

    let panel = panel_layout(&core);
    let (x, y) = center(panel.arrow_reverse.unwrap());
    left_click_at(&mut core, x, y);
    assert!(core.editor().config().tools.arrow.reverse);
}

#[test]
fn counter_outline_row_toggles_the_config() {
    let mut core = fixture();
    open_panel(&mut core);
    core.editor_mut().activate_tool(ToolKind::Counter);
    assert!(core.editor().config().tools.counter.outline);
    let panel = panel_layout(&core);
    let (x, y) = center(panel.counter_outline.unwrap());
    left_click_at(&mut core, x, y);
    assert!(!core.editor().config().tools.counter.outline);
}

#[test]
fn pixelate_mode_row_swaps_the_tool_kind() {
    // The ToolKind::Blur panel exposure (blur ships unbound).
    let mut core = fixture();
    open_panel(&mut core);
    tap(&mut core, KeyCode::KeyB);
    assert_eq!(core.editor().active_tool(), Some(ToolKind::Pixelate));
    let panel = panel_layout(&core);
    let (x, y) = center(panel.pixelate_mode.unwrap());
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().active_tool(), Some(ToolKind::Blur));
    let panel = panel_layout(&core);
    let (x, y) = center(panel.pixelate_mode.unwrap());
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().active_tool(), Some(ToolKind::Pixelate));
}

// ---------------------------------------------------------------------------
// Layer list (the z-order model)
// ---------------------------------------------------------------------------

#[test]
fn layer_row_click_selects_the_layer() {
    let mut core = fixture();
    let bottom = commit_rect(&mut core, 100.0, 200.0);
    let top = commit_rect(&mut core, 300.0, 400.0);
    open_panel(&mut core);
    let panel = panel_layout(&core);
    assert_eq!(panel.layer_rows.len(), 2);
    assert_eq!(panel.layer_rows[0].0, bottom, "row 0 = bottom-most (z asc)");
    assert_eq!(panel.layer_rows[1].0, top);

    let (x, y) = center(panel.layer_rows[1].1);
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().selected_object(), Some(top));
    assert!(
        core.selection().cascade().object_selected(),
        "stage 2 synced"
    );
    let (x, y) = center(panel_layout(&core).layer_rows[0].1);
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().selected_object(), Some(bottom));
}

#[test]
fn layer_drag_reorder_is_one_undo_unit() {
    let mut core = fixture();
    let bottom = commit_rect(&mut core, 100.0, 200.0);
    let top = commit_rect(&mut core, 300.0, 400.0);
    open_panel(&mut core);
    let depth_before = core.editor().undo_stack().undo_depth();
    assert_eq!(core.editor().scene().z_order(), &[bottom, top]);

    let panel = panel_layout(&core);
    let (from_x, from_y) = center(panel.layer_rows[0].1);
    let (to_x, to_y) = center(panel.layer_rows[1].1);
    move_to(&mut core, from_x, from_y);
    click(&mut core, MouseButton::Left, true);
    move_to(&mut core, to_x, to_y);
    click(&mut core, MouseButton::Left, false);

    assert_eq!(core.editor().scene().z_order(), &[top, bottom]);
    assert_eq!(
        core.editor().undo_stack().undo_depth(),
        depth_before + 1,
        "the reorder is exactly ONE undo unit"
    );
    ctrl_z(&mut core);
    assert_eq!(
        core.editor().scene().z_order(),
        &[bottom, top],
        "undo restores the order"
    );
}

#[test]
fn layer_drop_outside_a_row_cancels_the_reorder() {
    let mut core = fixture();
    let bottom = commit_rect(&mut core, 100.0, 200.0);
    let top = commit_rect(&mut core, 300.0, 400.0);
    open_panel(&mut core);
    let depth_before = core.editor().undo_stack().undo_depth();
    let panel = panel_layout(&core);
    let (from_x, from_y) = center(panel.layer_rows[0].1);
    move_to(&mut core, from_x, from_y);
    click(&mut core, MouseButton::Left, true);
    move_to(&mut core, 300.0, 300.0); // away from the panel
    click(&mut core, MouseButton::Left, false);
    assert_eq!(core.editor().scene().z_order(), &[bottom, top]);
    assert_eq!(core.editor().undo_stack().undo_depth(), depth_before);
    assert_eq!(
        core.editor().selected_object(),
        Some(bottom),
        "the press selection survives"
    );
}

#[test]
fn raise_and_lower_buttons_step_the_selection_one_unit_each() {
    let mut core = fixture();
    let bottom = commit_rect(&mut core, 100.0, 200.0);
    let top = commit_rect(&mut core, 300.0, 400.0);
    open_panel(&mut core);
    core.editor_mut().select_layer(bottom);
    let depth_before = core.editor().undo_stack().undo_depth();

    let panel = panel_layout(&core);
    let (x, y) = center(panel.raise_button.unwrap());
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().scene().z_order(), &[top, bottom]);
    assert_eq!(core.editor().undo_stack().undo_depth(), depth_before + 1);

    let panel = panel_layout(&core);
    let (x, y) = center(panel.lower_button.unwrap());
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().scene().z_order(), &[bottom, top]);
    assert_eq!(core.editor().undo_stack().undo_depth(), depth_before + 2);

    // The edge no-op records NOTHING (the no-op guard).
    let panel = panel_layout(&core);
    let (x, y) = center(panel.lower_button.unwrap());
    left_click_at(&mut core, x, y);
    assert_eq!(core.editor().undo_stack().undo_depth(), depth_before + 2);
}

#[test]
fn esc_mid_drag_cancels_the_pending_reorder() {
    let mut core = fixture();
    let bottom = commit_rect(&mut core, 100.0, 200.0);
    let top = commit_rect(&mut core, 300.0, 400.0);
    open_panel(&mut core);
    let panel = panel_layout(&core);
    let (from_x, from_y) = center(panel.layer_rows[0].1);
    move_to(&mut core, from_x, from_y);
    click(&mut core, MouseButton::Left, true);
    // Esc pops the object stage (the row press selected) and cancels the grab.
    tap(&mut core, KeyCode::Escape);
    let panel = panel_layout(&core);
    let (to_x, to_y) = center(panel.layer_rows[1].1);
    move_to(&mut core, to_x, to_y);
    click(&mut core, MouseButton::Left, false);
    assert_eq!(
        core.editor().scene().z_order(),
        &[bottom, top],
        "no reorder after Esc"
    );
}

// ---------------------------------------------------------------------------
// HUD seam
// ---------------------------------------------------------------------------

#[test]
fn digit_notifier_timing() {
    let mut chrome = ChromeState::new();
    chrome.show_size_hud();
    assert!(chrome.hud.visible);
    chrome.hide_size_hud();
    assert!(!chrome.hud.visible);
}

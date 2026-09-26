//! The todo-22 text-tool suite: the session state machine (insert/delete/
//! navigate/multiline/wrap), the plan's IME acceptance sequences
//! (preedit -> commit, cancel-path discard), CJK glyph coverage via swash,
//! the tool lifecycle (commit/empty-commit/padding/re-edit/paint), and the
//! full funnel integration through [`OverlayCore::inject_event`] (click ->
//! type -> Ctrl+Return, click-outside commit, Esc cancel, digits-are-text,
//! IME commit, re-edit replacement as ONE undo unit).

#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::cast_precision_loss
)]

use std::time::Instant;

use cosmic_text::{Buffer, Metrics, Shaping, SwashCache};
use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalSize, Transform,
};
use flowshot_core::scene::{
    Color as SceneColor, PaintSink, Point as ScenePoint, Rect as SceneRect, TextObject,
    ToolObjectData,
};
use winit::event::{Ime, MouseButton};
use winit::keyboard::{KeyCode, ModifiersState};

use crate::editor::*;
use crate::input::SyntheticInput;
use crate::router::{InputRouter, WindowSlot};
use crate::state::OverlayCore;

use super::text::TEXT_PADDING;
use super::text_font::with_font_system;
use super::text_session::{TextSession, family_attrs};

const DRAW_RED: SceneColor = SceneColor::new(255, 0, 0, 255);
const POINT_16: f32 = 16.0;
const LINE_16: f32 = 16.0 * 1.2;

const LETTERS: [KeyCode; 26] = [
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
];

const DIGITS: [KeyCode; 10] = [
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

/// The physical code a real keyboard delivers for an ASCII char (the text
/// payload carries the insertion; the code must just avoid the edit-key
/// special arms).
fn code_for(character: char) -> KeyCode {
    match character {
        'a'..='z' => LETTERS[character as usize - 'a' as usize],
        'A'..='Z' => LETTERS[character as usize - 'A' as usize],
        '0'..='9' => DIGITS[character as usize - '0' as usize],
        // Space and every non-alphanumeric carrier: the text payload does
        // the insertion, the code only must not hit an edit-key special arm.
        _ => KeyCode::Space,
    }
}

fn code_for_first(text: &str) -> KeyCode {
    code_for(text.chars().next().unwrap_or(' '))
}

fn session() -> TextSession {
    TextSession::new("", POINT_16, None)
}

fn key(code: KeyCode, text: Option<&str>) -> EditKey<'_> {
    EditKey {
        code,
        text,
        repeat: false,
    }
}

// ---------------------------------------------------------------------------
// A. The session state machine
// ---------------------------------------------------------------------------

#[test]
fn insert_backspace_delete_and_navigate() {
    let mut session = session();
    assert!(session.is_empty());
    session.insert_str("abc");
    assert_eq!(session.text(), "abc");
    session.backspace();
    assert_eq!(session.text(), "ab");
    session.move_cursor(cosmic_text::Motion::Home, false);
    session.delete();
    assert_eq!(session.text(), "b");
    // End, then one Left: the cursor sits before the trailing char.
    session.move_cursor(cosmic_text::Motion::End, false);
    session.move_cursor(cosmic_text::Motion::Left, false);
    session.insert_str("Y");
    assert_eq!(session.text(), "Yb");
}

#[test]
fn word_motion_jumps_into_the_buffer() {
    let mut session = session();
    session.insert_str("beta gamma");
    session.move_cursor(cosmic_text::Motion::Home, false);
    session.move_cursor(cosmic_text::Motion::NextWord, false);
    session.insert_str("|");
    let text = session.text();
    let index = text.find('|').expect("marker inserted");
    assert!(
        index > 0 && index + 1 < text.len(),
        "word motion landed mid-buffer: {text}"
    );
}

#[test]
fn enter_creates_multiline_and_bakes_hard_breaks() {
    let mut session = session();
    session.insert_str("a");
    session.enter();
    session.insert_str("b");
    assert_eq!(session.text(), "a\nb");
    assert_eq!(session.baked_text(), "a\nb", "hard breaks bake as-is");
}

#[test]
fn ime_acceptance_preedit_sequence_then_commit() {
    // Plan acceptance: Preedit("ni") -> Preedit("nih") -> Commit("日")
    // yields buffer "日".
    let mut session = session();
    session.set_preedit("ni", None);
    assert_eq!(session.text(), "", "preedit never enters the buffer");
    assert_eq!(session.preedit().map(|p| p.text.as_str()), Some("ni"));
    session.set_preedit("nih", Some((3, 3)));
    assert_eq!(session.preedit().map(|p| p.text.as_str()), Some("nih"));
    session.commit_ime("日");
    assert_eq!(session.text(), "日");
    assert!(session.preedit().is_none());
}

#[test]
fn ime_cancel_path_disabled_without_commit_discards() {
    // Plan acceptance: the Cancel path (Disabled without Commit) discards.
    let mut session = session();
    session.insert_str("x");
    session.set_preedit("にほん", None);
    session.cancel_ime();
    assert_eq!(session.text(), "x");
    assert!(session.preedit().is_none());
}

#[test]
fn preedit_empty_string_clears_the_composition() {
    // winit sends Preedit("") before a commit or cancel (wayland backend).
    let mut session = session();
    session.set_preedit("ni", None);
    session.set_preedit("", None);
    assert!(session.preedit().is_none());
}

#[test]
fn shift_motion_selects_and_insert_replaces_the_selection() {
    let mut session = session();
    session.insert_str("abc");
    session.move_cursor(cosmic_text::Motion::Home, false);
    session.move_cursor(cosmic_text::Motion::Right, true);
    assert!(!session.selection_spans().is_empty(), "selection highlight");
    session.insert_str("Z");
    assert_eq!(session.text(), "Zbc");
}

#[test]
fn with_text_selects_all_so_typing_replaces() {
    // Flameshot widget() selectAll parity on the re-edit path.
    let mut session = TextSession::with_text("", POINT_16, None, "old");
    assert_eq!(session.text(), "old");
    assert!(!session.selection_spans().is_empty(), "fully selected");
    session.insert_str("new");
    assert_eq!(session.text(), "new");
}

#[test]
fn wrap_width_bakes_visual_lines() {
    let mut session = TextSession::new("", POINT_16, Some(60.0));
    session.insert_str("hello world hello world");
    let logical = session.text();
    let baked = session.baked_text();
    assert!(!logical.contains('\n'), "logical text stays one line");
    assert!(baked.contains('\n'), "60px wrap must break the line");
    let strip = |text: &str| -> String {
        text.chars()
            .filter(|character| *character != '\n' && *character != ' ')
            .collect()
    };
    assert_eq!(strip(&baked), strip(&logical), "baking loses no characters");
    // Widening the box reflows back to a single line.
    session.set_wrap_width(None);
    assert!(!session.baked_text().contains('\n'));
}

#[test]
fn click_repositions_the_cursor() {
    let mut session = session();
    session.insert_str("hello");
    session.click(0.0, 0.0);
    session.insert_str("X");
    assert_eq!(session.text(), "Xhello");
}

#[test]
fn caret_and_layout_sanity() {
    let mut session = session();
    let caret = session.caret();
    assert_eq!((caret.x, caret.y, caret.height), (0.0, 0.0, LINE_16));
    let (width, height) = session.layout_size();
    assert_eq!(width, 0.0);
    assert_eq!(height, LINE_16, "an empty session owns one line");
    session.insert_str("hello");
    let (width, _) = session.layout_size();
    assert!(width > 0.0, "laid-out text has extent");
    assert!(session.caret().x > 0.0, "caret sits after the text");
    session.set_point_size(32.0);
    assert_eq!(session.caret().height, 32.0 * 1.2);
}

#[test]
fn cjk_glyphs_resolve_through_fontconfig_fallback() {
    // Plan acceptance: CJK font-fallback fixture - glyph coverage assert
    // via swash (a notdef/tofu glyph has id 0 and/or rasterizes empty).
    with_font_system(
        |fonts| {
            let mut cache = SwashCache::new();
            let mut buffer = Buffer::new(fonts, Metrics::new(POINT_16, LINE_16));
            buffer.set_text("日本語", &family_attrs(""), Shaping::Advanced, None);
            buffer.shape_until_scroll(fonts, false);
            let mut glyphs = 0usize;
            let mut inked = 0usize;
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    glyphs += 1;
                    assert_ne!(glyph.glyph_id, 0, "notdef (tofu) for a CJK char");
                    let physical = glyph.physical((0.0, run.line_y), 1.0);
                    if let Some(image) = cache.get_image(fonts, physical.cache_key).as_ref()
                        && image.placement.width > 0
                        && image.placement.height > 0
                    {
                        inked += 1;
                    }
                }
            }
            assert_eq!(glyphs, 3, "one glyph per CJK char");
            assert_eq!(inked, 3, "every CJK glyph rasterized with ink");
        },
        (),
    );
}

// ---------------------------------------------------------------------------
// B. The tool lifecycle (direct TextTool surface)
// ---------------------------------------------------------------------------

fn ctx_parts() -> EditorTools {
    EditorTools::default()
}

fn ctx(config: &EditorTools, tool_size: u32) -> EditorContext<'_> {
    EditorContext {
        frame: None,
        selection: None,
        color: DRAW_RED,
        tool_size,
        mouse: LogicalPoint::zero(),
        modifiers: ModifiersState::empty(),
        circle_count: 1,
        config,
    }
}

fn typing_tool() -> (EditorTools, TextTool) {
    let config = ctx_parts();
    let mut tool = TextTool::default();
    tool.on_color_changed(DRAW_RED);
    tool.on_size_changed(12);
    (config, tool)
}

fn tool_type(tool: &mut TextTool, ctx: &EditorContext<'_>, text: &str) {
    assert!(tool.edit_key(ctx, key(code_for_first(text), Some(text))));
}

#[test]
fn commit_produces_a_styled_text_object() {
    let (config, mut tool) = typing_tool();
    let ctx = ctx(&config, 12);
    tool.draw_start(&ctx, LogicalPoint::from_raw(100.0, 50.0));
    assert!(!tool.is_valid(), "empty session is not committable");
    tool_type(&mut tool, &ctx, "hi");
    assert!(tool.is_valid());
    let object = tool.commit_edit(&ctx).expect("committed");
    match object.to_data() {
        ToolObjectData::Text(text) => {
            assert_eq!(text.text, "hi");
            // F27: point size = tool_size + BASE_POINT_SIZE (12 + 8).
            assert_eq!(text.font_size, 20.0);
            assert_eq!(text.color, DRAW_RED);
            assert_eq!(text.position, ScenePoint::new(100.0, 50.0));
        }
        other => panic!("text object, got {other:?}"),
    }
    assert!(tool.commit_edit(&ctx).is_none(), "session consumed");
    assert!(tool.edit_rect().is_none());
}

#[test]
fn empty_commit_produces_no_object() {
    // Plan failure QA: empty commit -> no object.
    let (config, mut tool) = typing_tool();
    let ctx = ctx(&config, 12);
    tool.draw_start(&ctx, LogicalPoint::from_raw(100.0, 50.0));
    assert!(tool.commit_edit(&ctx).is_none());
}

#[test]
fn edit_rect_carries_the_five_px_padding() {
    let (config, mut tool) = typing_tool();
    let ctx = ctx(&config, 12);
    tool.draw_start(&ctx, LogicalPoint::from_raw(100.0, 50.0));
    let empty = tool.edit_rect().expect("editing");
    assert_eq!(empty.x.0, 100.0 - f64::from(TEXT_PADDING));
    assert_eq!(empty.y.0, 50.0 - f64::from(TEXT_PADDING));
    // One line at point size 20 (12 + BASE 8), padded both sides.
    assert_eq!(empty.height.0, 20.0 * 1.2 + f64::from(TEXT_PADDING * 2.0));
    tool_type(&mut tool, &ctx, "hello");
    let grown = tool.edit_rect().expect("editing");
    assert!(grown.width.0 > empty.width.0, "the box grows with the text");
    assert_eq!(grown.x.0, empty.x.0, "anchored at the press");
}

#[derive(Debug, Default)]
struct Recorder {
    texts: Vec<(ScenePoint, String, f32)>,
    fills: Vec<SceneRect>,
}

impl PaintSink for Recorder {
    fn fill_rect(&mut self, rect: SceneRect, _color: SceneColor) {
        self.fills.push(rect);
    }
    fn stroke_rect(&mut self, _r: SceneRect, _c: SceneColor, _w: f32) {}
    fn stroke_rounded_rect(&mut self, _r: SceneRect, _rad: f32, _c: SceneColor, _w: f32) {}
    fn fill_ellipse(&mut self, _r: SceneRect, _c: SceneColor) {}
    fn stroke_ellipse(&mut self, _r: SceneRect, _c: SceneColor, _w: f32) {}
    fn draw_line(&mut self, _a: ScenePoint, _b: ScenePoint, _c: SceneColor, _w: f32) {}
    fn stroke_polyline(&mut self, _p: &[ScenePoint], _c: SceneColor, _w: f32) {}
    fn fill_polygon(&mut self, _p: &[ScenePoint], _c: SceneColor) {}
    fn invert_region(&mut self, _r: SceneRect) {}
    fn draw_text(&mut self, position: ScenePoint, text: &str, font_size: f32, _c: SceneColor) {
        self.texts.push((position, text.to_owned(), font_size));
    }
}

#[test]
fn paint_emits_text_caret_and_the_underlined_preedit_overlay() {
    let (config, mut tool) = typing_tool();
    let ctx = ctx(&config, 12);
    tool.draw_start(&ctx, LogicalPoint::from_raw(100.0, 50.0));
    tool_type(&mut tool, &ctx, "ab");
    let mut sink = Recorder::default();
    tool.paint(&ctx, &mut sink);
    assert_eq!(sink.texts.len(), 1, "buffer text only");
    assert_eq!(sink.texts[0].1, "ab");
    assert_eq!(sink.texts[0].0, ScenePoint::new(100.0, 50.0));
    assert_eq!(sink.texts[0].2, 20.0);
    assert_eq!(sink.fills.len(), 1, "the caret bar");

    // With a pending composition: preedit text + underline + caret.
    assert!(tool.ime(&ctx, &Ime::Preedit("ni".to_owned(), Some((2, 2)))));
    let mut sink = Recorder::default();
    tool.paint(&ctx, &mut sink);
    assert_eq!(sink.texts.len(), 2, "buffer + composition overlay");
    assert_eq!(sink.texts[1].1, "ni");
    assert_eq!(sink.fills.len(), 2, "underline + caret");
    let underline = sink.fills[0];
    assert!(underline.width > 0.0, "underline spans the composition");
    assert!(underline.height <= 2.0, "underline is a thin bar");
}

#[test]
fn reedit_entry_preserves_text_and_position() {
    let (config, mut tool) = typing_tool();
    let ctx = ctx(&config, 12);
    let old = TextObject::new(
        ScenePoint::new(10.0, 20.0),
        "old".to_owned(),
        POINT_16,
        DRAW_RED,
    );
    assert!(tool.edit_object_data(&ctx, &ToolObjectData::Text(old)));
    let object = tool.commit_edit(&ctx).expect("re-edit commits");
    match object.to_data() {
        ToolObjectData::Text(text) => {
            assert_eq!(text.text, "old", "text preserved");
            assert_eq!(text.position, ScenePoint::new(10.0, 20.0));
            assert_eq!(text.font_size, 20.0, "current tool size wins (parity)");
        }
        other => panic!("text object, got {other:?}"),
    }
    // Non-text data is rejected (the probe falls through to a fresh edit).
    let mut tool = TextTool::default();
    let pencil = ToolObjectData::Pencil(flowshot_core::scene::PencilPath::new(
        vec![ScenePoint::new(0.0, 0.0), ScenePoint::new(1.0, 1.0)],
        DRAW_RED,
        3.0,
    ));
    assert!(!tool.edit_object_data(&ctx, &pencil));
}

#[test]
fn ime_events_route_into_the_session() {
    let (config, mut tool) = typing_tool();
    let ctx = ctx(&config, 12);
    tool.draw_start(&ctx, LogicalPoint::from_raw(100.0, 50.0));
    assert!(tool.ime(&ctx, &Ime::Enabled));
    assert!(tool.ime(&ctx, &Ime::Preedit("ka".to_owned(), None)));
    assert!(tool.ime(&ctx, &Ime::Commit("火".to_owned())));
    assert!(tool.ime(&ctx, &Ime::Disabled));
    let object = tool.commit_edit(&ctx).expect("committed");
    match object.to_data() {
        ToolObjectData::Text(text) => assert_eq!(text.text, "火"),
        other => panic!("text object, got {other:?}"),
    }
    // Without a session, IME events are not consumed.
    let mut idle = TextTool::default();
    assert!(!idle.ime(&ctx, &Ime::Commit("x".to_owned())));
}

#[test]
fn drag_defines_the_wrap_box() {
    let (config, mut tool) = typing_tool();
    let ctx = ctx(&config, 12);
    tool.draw_start(&ctx, LogicalPoint::from_raw(100.0, 100.0));
    tool.draw_move(&ctx, LogicalPoint::from_raw(180.0, 100.0));
    tool_type(&mut tool, &ctx, "hello world hello world");
    let object = tool.commit_edit(&ctx).expect("committed");
    match object.to_data() {
        ToolObjectData::Text(text) => {
            assert!(text.text.contains('\n'), "the 80px box wrapped the text");
        }
        other => panic!("text object, got {other:?}"),
    }
}

#[test]
fn edit_keys_reject_modified_and_control_text() {
    let (config, mut tool) = typing_tool();
    let ctx = ctx(&config, 12);
    tool.draw_start(&ctx, LogicalPoint::from_raw(100.0, 50.0));
    // Ctrl+C: rejected (the funnel passes it to the selection engine).
    let ctrl_ctx = EditorContext {
        modifiers: ModifiersState::CONTROL,
        ..ctx
    };
    assert!(!tool.edit_key(&ctrl_ctx, key(KeyCode::KeyC, Some("\u{3}"))));
    // Escape and Tab belong to the cascade / focus traversal.
    assert!(!tool.edit_key(&ctx, key(KeyCode::Escape, None)));
    assert!(!tool.edit_key(&ctx, key(KeyCode::Tab, None)));
    // Ctrl+Enter is the editor funnel's commit, not a newline.
    assert!(!tool.edit_key(&ctrl_ctx, key(KeyCode::Enter, None)));
    // Plain Enter IS a newline.
    assert!(tool.edit_key(&ctx, key(KeyCode::Enter, None)));
    // No session: nothing consumed.
    let mut idle = TextTool::default();
    assert!(!idle.edit_key(&ctx, key(KeyCode::KeyX, Some("x"))));
}

// ---------------------------------------------------------------------------
// C. Funnel integration (the production routing path)
// ---------------------------------------------------------------------------

const SLOT: WindowSlot = WindowSlot::new(0);

fn text_core() -> OverlayCore {
    let output = OutputInfo::new(
        "DP-1",
        "DP-1",
        LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
        PhysicalSize::from_raw(1920, 1080),
        1.0,
        Transform::Normal,
    )
    .expect("valid fixture output");
    let layout = OutputLayout::new(vec![output]);
    let mut core = OverlayCore::new(InputRouter::new(layout, vec![0]));
    register_text_tool(core.editor_mut().registry_mut());
    core
}

fn start_edit(core: &mut OverlayCore, x: f64, y: f64) {
    core.inject_event(SyntheticInput::key_press(SLOT, KeyCode::KeyT));
    assert_eq!(core.editor().active_tool(), Some(ToolKind::Text));
    core.inject_event(SyntheticInput::pointer_moved(SLOT, x, y));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        true,
    ));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        false,
    ));
    assert!(core.editor().editing(), "click opened the edit session");
}

fn type_text(core: &mut OverlayCore, text: &str) {
    for character in text.chars() {
        core.inject_event(SyntheticInput::key_text(
            SLOT,
            code_for(character),
            &character.to_string(),
        ));
    }
}

fn ctrl_enter(core: &mut OverlayCore) {
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::CONTROL));
    core.inject_event(SyntheticInput::key_press(SLOT, KeyCode::Enter));
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::empty()));
}

fn committed_text(core: &OverlayCore, id: usize) -> TextObject {
    match core
        .editor()
        .scene()
        .get_object(id)
        .expect("object")
        .to_data()
    {
        ToolObjectData::Text(text) => text,
        other => panic!("text object, got {other:?}"),
    }
}

#[test]
fn funnel_click_type_ctrl_enter_commits_one_undo_unit() {
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    // The caret rect feeds the shell's IME cursor-area mirror while editing.
    let area = core
        .editor()
        .ime_cursor_area()
        .expect("caret while editing");
    assert_eq!((area.x.0, area.y.0), (400.0, 300.0));
    type_text(&mut core, "FlowShot");
    ctrl_enter(&mut core);
    assert_eq!(core.editor().scene().object_count(), 1);
    assert_eq!(core.editor().undo_stack().undo_depth(), 1);
    let text = committed_text(&core, 0);
    assert_eq!(text.text, "FlowShot");
    assert_eq!(
        text.font_size,
        (BASE_POINT_SIZE + 8) as f32,
        "slot 8 + base"
    );
    assert_eq!(text.position, ScenePoint::new(400.0, 300.0));
    assert!(!core.editor().editing());
    assert!(core.editor().ime_cursor_area().is_none(), "caret gone");
}

#[test]
fn funnel_digits_type_while_editing_and_never_resize() {
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    core.inject_event(SyntheticInput::key_text(SLOT, KeyCode::Digit5, "5"));
    assert_eq!(core.editor().tool_size(), 8, "the font slot never moved");
    ctrl_enter(&mut core);
    assert_eq!(committed_text(&core, 0).text, "5");
}

#[test]
fn funnel_click_outside_commits() {
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    type_text(&mut core, "hi");
    core.inject_event(SyntheticInput::pointer_moved(SLOT, 900.0, 700.0));
    let report = core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        true,
    ));
    assert!(
        report
            .actions
            .iter()
            .any(|a| matches!(a, crate::input::Action::Redraw(_))),
        "the commit redraws"
    );
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        false,
    ));
    assert_eq!(core.editor().scene().object_count(), 1);
    assert_eq!(committed_text(&core, 0).text, "hi");
    assert!(!core.editor().editing());
}

#[test]
fn funnel_esc_mid_edit_cancels_without_object() {
    // Plan failure QA: Esc mid-edit cancels without an object.
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    type_text(&mut core, "doomed");
    core.inject_event(SyntheticInput::key_press(SLOT, KeyCode::Escape));
    assert_eq!(core.editor().scene().object_count(), 0);
    assert!(!core.editor().editing());
    assert_eq!(
        core.editor().active_tool(),
        None,
        "cascade stage 1 deselected the tool"
    );
}

#[test]
fn funnel_ime_commit_inserts_cjk() {
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    core.inject_event(SyntheticInput::ime(SLOT, Ime::Enabled));
    core.inject_event(SyntheticInput::ime(
        SLOT,
        Ime::Preedit("ni".to_owned(), None),
    ));
    core.inject_event(SyntheticInput::ime(
        SLOT,
        Ime::Preedit("nih".to_owned(), Some((1, 3))),
    ));
    core.inject_event(SyntheticInput::ime(SLOT, Ime::Commit("日".to_owned())));
    ctrl_enter(&mut core);
    assert_eq!(committed_text(&core, 0).text, "日");
    // The shared status tracking (todo 13) survives the editor consumption.
    assert_eq!(*core.ime(), crate::input::ImeStatus::Active);
}

#[test]
fn funnel_reedit_replaces_as_one_undo_unit() {
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    type_text(&mut core, "old");
    ctrl_enter(&mut core);
    assert_eq!(core.editor().undo_stack().undo_depth(), 1);

    // A click on the committed object re-enters edit (the tool is active).
    core.inject_event(SyntheticInput::pointer_moved(SLOT, 410.0, 310.0));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        true,
    ));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        false,
    ));
    assert!(core.editor().editing(), "re-edit session open");
    assert_eq!(
        core.editor().scene().object_count(),
        0,
        "the old object is provisionally removed"
    );

    // The session opened select-all (Flameshot parity): typing replaces.
    type_text(&mut core, "new");
    ctrl_enter(&mut core);
    assert_eq!(
        core.editor().scene().object_count(),
        1,
        "replaced, not added"
    );
    assert_eq!(core.editor().undo_stack().undo_depth(), 2, "ONE new unit");
    assert_eq!(committed_text(&core, 0).text, "new");

    // Undo restores the old text exactly.
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::CONTROL));
    core.inject_event(SyntheticInput::key_press(SLOT, KeyCode::KeyZ));
    core.inject_event(SyntheticInput::modifiers(SLOT, ModifiersState::empty()));
    assert_eq!(committed_text(&core, 0).text, "old");
}

#[test]
fn funnel_empty_reedit_commit_restores_the_old_object() {
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    type_text(&mut core, "old");
    ctrl_enter(&mut core);
    core.inject_event(SyntheticInput::pointer_moved(SLOT, 410.0, 310.0));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        true,
    ));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        false,
    ));
    assert!(core.editor().editing());
    for _ in 0..3 {
        core.inject_event(SyntheticInput::key_press(SLOT, KeyCode::Backspace));
    }
    ctrl_enter(&mut core);
    assert_eq!(
        core.editor().scene().object_count(),
        1,
        "old object restored"
    );
    assert_eq!(committed_text(&core, 0).text, "old");
    assert_eq!(
        core.editor().undo_stack().undo_depth(),
        1,
        "no new undo unit for the empty commit"
    );
    assert!(!core.editor().editing());
}

#[test]
fn funnel_esc_mid_reedit_restores_the_old_object() {
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    type_text(&mut core, "old");
    ctrl_enter(&mut core);
    core.inject_event(SyntheticInput::pointer_moved(SLOT, 410.0, 310.0));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        true,
    ));
    core.inject_event(SyntheticInput::pointer_button(
        SLOT,
        MouseButton::Left,
        false,
    ));
    assert_eq!(
        core.editor().scene().object_count(),
        0,
        "provisional removal"
    );
    core.inject_event(SyntheticInput::key_press(SLOT, KeyCode::Escape));
    assert_eq!(
        core.editor().scene().object_count(),
        1,
        "cancel restored the re-edited object"
    );
    assert_eq!(committed_text(&core, 0).text, "old");
}

#[test]
fn funnel_edit_session_paints_into_the_display_list() {
    // The scene paint of a committed text object + the live edit overlay
    // both reach the renderer's text layer (single shaping path).
    let mut core = text_core();
    start_edit(&mut core, 400.0, 300.0);
    type_text(&mut core, "FlowShot");
    let output = OutputInfo::new(
        "DP-1",
        "DP-1",
        LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
        PhysicalSize::from_raw(1920, 1080),
        1.0,
        Transform::Normal,
    )
    .expect("valid fixture output");
    let mut list = crate::render::DisplayList::new();
    core.editor().paint_into(
        &mut list,
        &output,
        EditorView {
            mouse: Some(LogicalPoint::from_raw(400.0, 300.0)),
            selection: None,
            modifiers: ModifiersState::empty(),
        },
    );
    let texts = list
        .iter()
        .filter(|command| matches!(command, crate::render::Command::Text(_)))
        .count();
    assert_eq!(texts, 1, "the edit session paints its buffer text");
    ctrl_enter(&mut core);
    let mut list = crate::render::DisplayList::new();
    core.editor().paint_into(
        &mut list,
        &output,
        EditorView {
            mouse: None,
            selection: None,
            modifiers: ModifiersState::empty(),
        },
    );
    let texts = list
        .iter()
        .filter(|command| matches!(command, crate::render::Command::Text(_)))
        .count();
    assert_eq!(texts, 1, "the committed object paints identically");
}

#[test]
fn funnel_editing_survives_without_a_cursor_track() {
    // paint_into must not hide the edit session when no motion was tracked
    // (headless paints with mouse: None still render the text).
    let mut ed = {
        let mut registry = ToolRegistry::new();
        register_text_tool(&mut registry);
        EditorState::new(EditorTools::default(), registry)
    };
    let env = EditorEnv {
        selection: None,
        modifiers: ModifiersState::empty(),
        now: Instant::now(),
        picker_visible: false,
        mouse: Some(LogicalPoint::from_raw(100.0, 100.0)),
    };
    ed.activate_tool(ToolKind::Text);
    ed.pointer_press(
        &env,
        MouseButton::Left,
        LogicalPoint::from_raw(100.0, 100.0),
    );
    ed.pointer_release(
        &env,
        MouseButton::Left,
        LogicalPoint::from_raw(100.0, 100.0),
    );
    ed.key_press(&env, KeyCode::KeyX, false, Some("x"));
    let output = OutputInfo::new(
        "DP-1",
        "DP-1",
        LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
        PhysicalSize::from_raw(1920, 1080),
        1.0,
        Transform::Normal,
    )
    .expect("valid fixture output");
    let mut list = crate::render::DisplayList::new();
    ed.paint_into(
        &mut list,
        &output,
        EditorView {
            mouse: None,
            selection: None,
            modifiers: ModifiersState::empty(),
        },
    );
    assert!(
        list.iter()
            .any(|command| matches!(command, crate::render::Command::Text(_))),
        "the edit session paints without a cursor track"
    );
}

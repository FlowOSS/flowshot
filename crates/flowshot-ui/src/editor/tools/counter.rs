//! The circle-count tool (clean-room from Flameshot's circlecount spec).
//!
//! Numbered step bubbles: click places the next count (the scene's max+1
//! rule in the core scene), diameter from `[tools.counter].size`, outline
//! toggle from `[tools.counter].outline`, fill = current draw color, number
//! centered contrasting. Wheel while hovering a bubble increments/decrements
//! ITS number (Flameshot row 10); delete triggers the core renumber op
//! (subsequent bubbles decrement); undo restores via max+1 rule.
//!
//! Press-DRAG aims the pointer (Flameshot `circlecounttool.cpp` @ 2d478061
//! drawStart/drawMove/drawEnd parity, cited in the core module docs): the
//! press fixes the bubble center, motion updates the aim target with a live
//! preview, and the release commits the target as the object's persisted
//! `pointer` - the core object's gate paints the triangle only while the
//! target is farther than one radius (their `line.length() > bubble_size`).
//! A click without drag stays a plain bubble (`pointer: None`).
//!
//! The tool commits a [`CounterObject`] with `count = 0` (unassigned); the
//! scene's `add_object` auto-numbers it with the max+1 rule. The outline
//! flag is read from the config at commit time.

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{Color as SceneColor, CounterObject, PaintSink, ToolObject};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, Tool};
use super::geometry::{logical, scene_point};
use crate::render::f32_from_u32;

/// The circle-count tool (numbered step bubbles).
#[derive(Debug, Default)]
pub struct CounterTool {
    /// The press position (the bubble center).
    press: Option<LogicalPoint>,
    /// The live drag-aim target (Flameshot's `points().second` while
    /// dragging; `None` until the first motion - a click stays plain).
    drag: Option<LogicalPoint>,
    /// The draw color.
    color: SceneColor,
    /// The bubble radius (dispatched from `[tools.counter].size`).
    radius: f32,
}

impl CounterTool {
    /// Computes the bubble radius from the dispatched size.
    ///
    /// The size slot is a small integer (default 1, max 50); the radius is
    /// `size * 8 + 8` (Flameshot `drawCircleCounterSize` semantics: a base radius
    /// of 8px plus 8px per size unit, so size=1 -> 16px diameter, size=2 ->
    /// 24px diameter, etc.).
    fn radius_from_size(size: u32) -> f32 {
        f32_from_u32(size) * 8.0 + 8.0
    }
}

impl Tool for CounterTool {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Counter
    }

    /// Opens a draw session at the press position (the bubble center; a
    /// drag from here aims the pointer).
    fn draw_start(&mut self, ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.press = Some(at);
        self.drag = None;
        self.radius = Self::radius_from_size(ctx.tool_size);
    }

    /// Extends the draw session: the aim target follows the cursor (the
    /// live pointer preview).
    fn draw_move(&mut self, _ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.drag = Some(at);
    }

    /// Commits the bubble at the press position; when the session dragged,
    /// the release point becomes the persisted aim pointer (a click without
    /// drag commits a plain bubble).
    fn draw_end(
        &mut self,
        ctx: &EditorContext<'_>,
        at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        let press = self.press.take()?;
        let pointer = self.drag.take().map(|_| scene_point(at));
        Some(Box::new(
            CounterObject::new(scene_point(press), self.radius, self.color, 0)
                .with_outline(ctx.config.tools.counter.outline)
                .with_pointer(pointer),
        ))
    }

    /// Paints the open session (the bubble anchored at the press plus the
    /// live aim pointer - the exact committed geometry with the next count
    /// previewed), else the cursor-following hover preview.
    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        let outline = ctx.config.tools.counter.outline;
        if let Some(press) = self.press {
            CounterObject::new(scene_point(press), self.radius, ctx.color, ctx.circle_count)
                .with_outline(outline)
                .with_pointer(self.drag.map(scene_point))
                .paint(sink);
            return;
        }
        if !ctx.config.mouse_preview {
            return;
        }
        let radius = Self::radius_from_size(ctx.tool_size);
        CounterObject::new(scene_point(ctx.mouse), radius, ctx.color, ctx.circle_count)
            .with_outline(outline)
            .paint(sink);
    }

    /// The in-progress bounds: the bubble grown by the outline ring and
    /// unioned with the drag target (the core object's own damage geometry).
    fn bounding_rect(&self) -> Option<LogicalRect> {
        let press = self.press?;
        let bounds = CounterObject::new(scene_point(press), self.radius, self.color, 0)
            .with_pointer(self.drag.map(scene_point))
            .bounding_rect();
        Some(logical(bounds))
    }

    /// A press is valid when it occurred.
    fn is_valid(&self) -> bool {
        self.press.is_some()
    }

    fn on_color_changed(&mut self, color: SceneColor) {
        self.color = color;
    }

    /// The dispatched size changed (digits/wheel/panel).
    fn on_size_changed(&mut self, size: u32) {
        self.radius = Self::radius_from_size(size);
    }

    /// Wheel while hovering a bubble increments/decrements ITS number (Flameshot
    /// row 10). The framework routes wheel events to the active tool; the
    /// tool returns `true` to consume it (otherwise the wheel adjusts the
    /// tool size).
    fn wheel(&mut self, _ctx: &EditorContext<'_>, _step: i32) -> bool {
        // The wheel-on-bubble logic requires hit-testing the cursor against
        // committed counter objects; the framework does not pass the scene
        // to the tool's wheel method. The wheel adjustment is handled at
        // the editor level ("wheel while hovering a bubble =
        // increment/decrement ITS number"); the tool's wheel method is a
        // no-op (returns false to let the framework adjust the tool size).
        false
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp
    )]

    use flowshot_core::config::Config;
    use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo, PhysicalSize, Transform};
    use flowshot_core::scene::{Point as ScenePoint, ToolObjectData};
    use winit::event::MouseButton;
    use winit::keyboard::ModifiersState;

    use super::*;
    use crate::editor::*;
    use crate::render::{Command, DisplayList, Shape};

    fn editor() -> EditorState {
        let mut registry = ToolRegistry::new();
        registry.register(ToolKind::Counter, || Box::new(CounterTool::default()));
        let mut ed = EditorState::new(EditorTools::from_config(&Config::default()), registry);
        ed.activate_tool(ToolKind::Counter);
        ed
    }

    fn env() -> EditorEnv {
        EditorEnv {
            selection: None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            now: std::time::Instant::now(),
            picker_visible: false,
            mouse: None,
        }
    }

    fn at(x: f64, y: f64) -> LogicalPoint {
        LogicalPoint::from_raw(x, y)
    }

    fn click(ed: &mut EditorState, env: &EditorEnv, x: f64, y: f64) {
        let pos = at(x, y);
        ed.pointer_press(env, MouseButton::Left, pos);
        ed.pointer_release(env, MouseButton::Left, pos);
    }

    fn drag(ed: &mut EditorState, env: &EditorEnv, from: (f64, f64), to: (f64, f64)) {
        ed.pointer_press(env, MouseButton::Left, at(from.0, from.1));
        ed.pointer_move(env, at(to.0, to.1));
        ed.pointer_release(env, MouseButton::Left, at(to.0, to.1));
    }

    fn counter(ed: &EditorState, id: usize) -> CounterObject {
        match ed.scene().get_object(id).expect("object").to_data() {
            ToolObjectData::Counter(counter) => counter,
            other => panic!("counter expected, got {other:?}"),
        }
    }

    fn dragged_counter(ed: &EditorState) -> CounterObject {
        ed.scene()
            .ids()
            .filter_map(|id| ed.scene().get_object(id).map(ToolObject::to_data))
            .find_map(|data| match data {
                ToolObjectData::Counter(c) if c.pointer.is_some() => Some(c),
                _ => None,
            })
            .expect("a dragged bubble")
    }

    fn paint_commands(ed: &EditorState, mouse: Option<LogicalPoint>) -> Vec<Command> {
        let out = OutputInfo::new(
            "DP-T",
            "DP-T",
            LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(1920, 1080),
            1.0,
            Transform::Normal,
        )
        .expect("valid fixture output");
        let mut list = DisplayList::new();
        ed.paint_into(
            &mut list,
            &out,
            EditorView {
                mouse,
                selection: None,
                modifiers: ModifiersState::empty(),
            },
        );
        list.iter().cloned().collect()
    }

    #[test]
    fn counter_auto_increment_sequence() {
        let mut ed = editor();
        let env = env();
        click(&mut ed, &env, 100.0, 100.0);
        click(&mut ed, &env, 200.0, 200.0);
        click(&mut ed, &env, 300.0, 300.0);
        assert_eq!(ed.scene().object_count(), 3);
        assert_eq!(ed.scene().counter_counts(), vec![1, 2, 3]);
    }

    #[test]
    fn delete_middle_renumbers() {
        let mut ed = editor();
        let env = env();
        for i in 0..5 {
            let x = 100.0 + f64::from(i) * 50.0;
            click(&mut ed, &env, x, 100.0);
        }
        assert_eq!(ed.scene().counter_counts(), vec![1, 2, 3, 4, 5]);
        ed.select_object_at(LogicalPoint::from_raw(150.0, 100.0));
        ed.delete_selected();
        assert_eq!(ed.scene().counter_counts(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn undo_restore_max_plus_one() {
        let mut ed = editor();
        let env = env();
        click(&mut ed, &env, 100.0, 100.0);
        click(&mut ed, &env, 200.0, 200.0);
        click(&mut ed, &env, 300.0, 300.0);
        assert_eq!(ed.scene().counter_counts(), vec![1, 2, 3]);
        ed.select_object_at(LogicalPoint::from_raw(200.0, 200.0));
        ed.delete_selected();
        assert_eq!(ed.scene().counter_counts(), vec![1, 2]);
        ed.undo();
        assert_eq!(ed.scene().counter_counts(), vec![1, 2, 3]);
    }

    #[test]
    fn outline_toggle_from_config() {
        let mut config = Config::default();
        config.tools.counter.outline = false;
        let mut registry = ToolRegistry::new();
        registry.register(ToolKind::Counter, || Box::new(CounterTool::default()));
        let mut ed = EditorState::new(EditorTools::from_config(&config), registry);
        ed.activate_tool(ToolKind::Counter);
        let env = env();
        click(&mut ed, &env, 100.0, 100.0);
        assert_eq!(ed.scene().object_count(), 1);
        assert!(
            !counter(&ed, 0).outline,
            "the config flag is captured at commit"
        );
    }

    #[test]
    fn click_without_drag_commits_a_plain_bubble() {
        let mut ed = editor();
        let env = env();
        click(&mut ed, &env, 100.0, 100.0);
        let bubble = counter(&ed, 0);
        assert_eq!(bubble.center, ScenePoint::new(100.0, 100.0));
        assert_eq!(bubble.pointer, None, "no motion, no aim target");
        assert_eq!(bubble.count, 1);
        assert!(bubble.outline, "the config default is captured at commit");
    }

    #[test]
    fn drag_commits_the_release_point_as_the_aim_pointer() {
        let mut ed = editor();
        let env = env();
        drag(&mut ed, &env, (100.0, 100.0), (160.0, 100.0));
        assert_eq!(ed.scene().object_count(), 1);
        let bubble = counter(&ed, 0);
        assert_eq!(
            bubble.center,
            ScenePoint::new(100.0, 100.0),
            "the press anchors"
        );
        assert_eq!(bubble.pointer, Some(ScenePoint::new(160.0, 100.0)));
        assert_eq!(
            bubble.pointer_triangle(),
            Some([
                ScenePoint::new(100.0, 84.0),
                ScenePoint::new(160.0, 100.0),
                ScenePoint::new(100.0, 116.0)
            ]),
            "the committed object paints the Flameshot triangle"
        );
    }

    #[test]
    fn live_drag_preview_paints_the_session_anchored_at_the_press() {
        let mut ed = editor();
        let env = env();
        ed.pointer_press(&env, MouseButton::Left, at(100.0, 100.0));
        ed.pointer_move(&env, at(160.0, 100.0));
        let commands = paint_commands(&ed, Some(at(160.0, 100.0)));
        let Command::Fill {
            shape: Shape::Polyline { points, closed },
            color,
        } = &commands[0]
        else {
            panic!("the aim triangle paints first: {:?}", commands.first());
        };
        assert!(closed);
        assert_eq!(
            points,
            &[
                crate::render::Point::new(100.0, 84.0),
                crate::render::Point::new(160.0, 100.0),
                crate::render::Point::new(100.0, 116.0)
            ],
            "apex at the cursor, base on the press diameter (scale 1)"
        );
        assert_eq!(color.r, 1.0, "the triangle fills with the draw color");
        let Command::Fill {
            shape: Shape::Ellipse { center, .. },
            ..
        } = &commands[1]
        else {
            panic!("the outline ring follows: {:?}", commands.get(1));
        };
        assert_eq!(
            *center,
            crate::render::Point::new(100.0, 100.0),
            "the bubble stays anchored at the press, not the cursor"
        );
    }

    #[test]
    fn renumbering_survives_dragged_bubbles() {
        let mut ed = editor();
        let env = env();
        click(&mut ed, &env, 100.0, 100.0);
        drag(&mut ed, &env, (200.0, 200.0), (260.0, 200.0));
        click(&mut ed, &env, 300.0, 300.0);
        assert_eq!(ed.scene().counter_counts(), vec![1, 2, 3]);
        ed.select_object_at(at(100.0, 100.0));
        ed.delete_selected();
        assert_eq!(ed.scene().counter_counts(), vec![1, 2]);
        let dragged = dragged_counter(&ed);
        assert_eq!(dragged.count, 1, "the dragged bubble renumbered");
        assert_eq!(
            dragged.pointer,
            Some(ScenePoint::new(260.0, 200.0)),
            "renumbering preserves the aim pointer"
        );
    }

    #[test]
    fn size_slot_dispatch() {
        let sizes = ToolSizes::from_config(&EditorTools::from_config(&Config::default()));
        assert_eq!(sizes.get(Some(ToolKind::Counter)), 1);
        assert_eq!(CounterTool::radius_from_size(1), 16.0);
        assert_eq!(CounterTool::radius_from_size(2), 24.0);
    }

    #[test]
    fn delete_non_counter_leaves_counts_untouched() {
        let mut ed = editor();
        let env = env();
        click(&mut ed, &env, 100.0, 100.0);
        click(&mut ed, &env, 200.0, 200.0);
        assert_eq!(ed.scene().counter_counts(), vec![1, 2]);
        register_shape_tools(ed.registry_mut());
        ed.activate_tool(ToolKind::Line);
        let from = LogicalPoint::from_raw(300.0, 300.0);
        let to = LogicalPoint::from_raw(400.0, 400.0);
        ed.pointer_press(&env, MouseButton::Left, from);
        ed.pointer_move(&env, to);
        ed.pointer_release(&env, MouseButton::Left, to);
        assert_eq!(ed.scene().object_count(), 3);
        ed.select_object_at(LogicalPoint::from_raw(350.0, 350.0));
        ed.delete_selected();
        assert_eq!(ed.scene().counter_counts(), vec![1, 2]);
    }
}

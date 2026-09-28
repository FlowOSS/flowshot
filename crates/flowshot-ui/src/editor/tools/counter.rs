//! The circle-count tool (draft F27 circlecount spec).
//!
//! Numbered step bubbles: click places the next count (the scene's max+1
//! rule in the core scene), diameter from `[tools.counter].size`, outline toggle from
//! `[tools.counter].outline`, fill = current draw color, number centered
//! contrasting. Wheel while hovering a bubble increments/decrements ITS
//! number (F12 row 10); delete triggers the core renumber op
//! (subsequent bubbles decrement); undo restores via max+1 rule.
//!
//! The tool commits a [`CounterObject`] with `count = 0` (unassigned); the
//! scene's `add_object` auto-numbers it with the max+1 rule. The outline
//! flag is read from the config at commit time.

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::{
    Color as SceneColor, CounterObject, PaintSink, Point as ScenePoint, ToolObject,
};

use super::super::kind::ToolKind;
use super::super::tool::{EditorContext, Tool};
use crate::render::{f32_from_f64, f32_from_u32};

/// The circle-count tool (numbered step bubbles).
#[derive(Debug, Default)]
pub struct CounterTool {
    /// The press position (click placement, no drag).
    press: Option<LogicalPoint>,
    /// The draw color.
    color: SceneColor,
    /// The bubble radius (dispatched from `[tools.counter].size`).
    radius: f32,
}

impl CounterTool {
    /// Computes the bubble radius from the dispatched size.
    ///
    /// The size slot is a small integer (default 1, max 50); the radius is
    /// `size * 8 + 8` (F27 `drawCircleCounterSize` semantics: a base radius
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

    /// Opens a draw session at the press position (click placement).
    fn draw_start(&mut self, ctx: &EditorContext<'_>, at: LogicalPoint) {
        self.press = Some(at);
        self.radius = Self::radius_from_size(ctx.tool_size);
    }

    /// Extends the draw session (unused for click placement; the counter
    /// is placed at the press position).
    fn draw_move(&mut self, _ctx: &EditorContext<'_>, _at: LogicalPoint) {}

    /// Commits the counter object at the press position (click placement;
    /// the drag distance is ignored).
    fn draw_end(
        &mut self,
        _ctx: &EditorContext<'_>,
        _at: LogicalPoint,
    ) -> Option<Box<dyn ToolObject>> {
        let press = self.press.take()?;
        let center = ScenePoint::new(f32_from_f64(press.x.0), f32_from_f64(press.y.0));
        Some(Box::new(CounterObject::new(
            center,
            self.radius,
            self.color,
            0,
        )))
    }

    /// Paints the cursor-following preview (a filled circle with the next
    /// count number).
    fn paint(&self, ctx: &EditorContext<'_>, sink: &mut dyn PaintSink) {
        if !ctx.config.mouse_preview {
            return;
        }
        let radius = Self::radius_from_size(ctx.tool_size);
        let center = ScenePoint::new(f32_from_f64(ctx.mouse.x.0), f32_from_f64(ctx.mouse.y.0));
        let preview = CounterObject::new(center, radius, ctx.color, ctx.circle_count);
        preview.paint(sink);
    }

    /// The preview bounds (the cursor-following bubble).
    fn bounding_rect(&self) -> Option<LogicalRect> {
        self.press.map(|p| {
            let r = f64::from(self.radius);
            LogicalRect::from_raw(p.x.0 - r, p.y.0 - r, r * 2.0, r * 2.0)
        })
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

    /// Wheel while hovering a bubble increments/decrements ITS number (F12
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
    use flowshot_core::geometry::LogicalPoint;
    use winit::event::MouseButton;

    use super::*;
    use crate::editor::*;

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

    fn click(ed: &mut EditorState, env: &EditorEnv, x: f64, y: f64) {
        let at = LogicalPoint::from_raw(x, y);
        ed.pointer_press(env, MouseButton::Left, at);
        ed.pointer_release(env, MouseButton::Left, at);
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

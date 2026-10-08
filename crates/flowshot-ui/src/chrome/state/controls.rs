//! The chrome control writes (split from [`super::input`] at the 250-LOC
//! ceiling): the color pick with its persistence sink and the side-panel
//! control press - both through the editor seams ONLY (`set_tool_size` /
//! `resize_selected` / `recolor_selected` / `configure` / `activate_tool` /
//! the z-order ops), and the layer drag-reorder arm
//! ([`EditorState::move_layer`], ONE undo unit).

use flowshot_core::config::ArrowStyle;

use crate::chrome::side_panel;
use crate::editor::{EditorState, EditorTools, MAX_TOOL_SIZE, MIN_TOOL_SIZE, ToolKind};
use crate::render::Point;

use super::ChromeState;

impl ChromeState {
    /// The color pick: the draw color (an editor seam), the selected object's
    /// color (the property funnel, ONE undo unit, invert excluded),
    /// the persistence sink (Flameshot TOML write - the binary layer's seam), and
    /// the wheel hides.
    pub(super) fn pick_color(&mut self, hex: &str, editor: &mut EditorState) {
        let Some(color) = crate::editor::scene_color_from_hex(hex) else {
            tracing::warn!(
                target: "flowshot_ui::chrome",
                "palette entry malformed; pick ignored"
            );
            self.hide_color_wheel();
            return;
        };
        editor.set_color(color);
        editor.recolor_selected(color);
        if let Some(sink) = &self.draw_color_sink {
            sink(hex);
        }
        self.hide_color_wheel();
    }

    /// One panel control press (the hit order is the layout's paint order;
    /// row index == paint z, so the drop target maps straight onto
    /// [`EditorState::move_layer`]).
    pub(super) fn panel_press(
        &mut self,
        panel: &side_panel::SidePanelLayout,
        at: Point,
        editor: &mut EditorState,
    ) {
        if let Some(rect) = panel.size_slider
            && rect.contains(at)
        {
            let fraction = (at.x - rect.origin.x) / rect.size.width;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "the fraction is clamped into [MIN_TOOL_SIZE, MAX_TOOL_SIZE] before the cast"
            )]
            let size = (fraction * MAX_TOOL_SIZE as f32)
                .clamp(MIN_TOOL_SIZE as f32, MAX_TOOL_SIZE as f32)
                .round() as u32;
            editor.set_tool_size(size);
            editor.resize_selected(size);
        } else if let Some(rect) = panel.arrow_style
            && rect.contains(at)
        {
            edit_tool_config(editor, |config| {
                config.tools.arrow.style = match config.tools.arrow.style {
                    ArrowStyle::Straight => ArrowStyle::Curved,
                    ArrowStyle::Curved => ArrowStyle::Straight,
                };
            });
        } else if let Some(rect) = panel.arrow_reverse
            && rect.contains(at)
        {
            edit_tool_config(editor, |config| {
                config.tools.arrow.reverse = !config.tools.arrow.reverse;
            });
        } else if let Some(rect) = panel.counter_outline
            && rect.contains(at)
        {
            edit_tool_config(editor, |config| {
                config.tools.counter.outline = !config.tools.counter.outline;
            });
        } else if let Some(rect) = panel.pixelate_mode
            && rect.contains(at)
        {
            let next = if editor.active_tool() == Some(ToolKind::Blur) {
                ToolKind::Pixelate
            } else {
                ToolKind::Blur
            };
            editor.activate_tool(next);
        } else if let Some(rect) = panel.raise_button
            && rect.contains(at)
        {
            editor.raise_selected();
        } else if let Some(rect) = panel.lower_button
            && rect.contains(at)
        {
            editor.lower_selected();
        } else {
            for (z, (id, rect)) in panel.layer_rows.iter().enumerate() {
                if rect.contains(at) {
                    editor.select_layer(*id);
                    self.layer_drag = Some(super::LayerDrag { from_z: z });
                    break;
                }
            }
        }
    }
}

/// Writes a tool-config change through the editor settings seam
/// ([`EditorState::configure`]), preserving what the panel must not stomp:
/// the active tool's runtime size slot (wheel/digit adjustments) and the
/// wheel-picked draw color (the config string still carries the old hex).
fn edit_tool_config(editor: &mut EditorState, edit: impl FnOnce(&mut EditorTools)) {
    let (kind, size) = (editor.active_tool(), editor.tool_size());
    let color = editor.color();
    let mut config = editor.config().clone();
    edit(&mut config);
    config.editor.draw_color = format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b);
    editor.configure(config);
    if editor.active_tool() == kind {
        editor.set_tool_size(size);
    }
}

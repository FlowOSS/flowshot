//! The chrome input half (split from [`super`] at the 250-LOC ceiling): the
//! widget hit-tests behind the funnel's chrome-first ordering, the color
//! pick with its persistence sink, the panel control writes (through the
//! todo-20/25 editor seams ONLY - `set_tool_size` / `resize_selected` /
//! `recolor_selected` / `configure` / `activate_tool` / the z-order ops),
//! and the layer drag-reorder ([`EditorState::move_layer`], ONE undo unit).

use flowshot_core::config::ArrowStyle;
use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo};
use winit::event::MouseButton;

use crate::chrome::side_panel;
use crate::chrome::toolbar::ToolbarButton;
use crate::editor::paint::{local_x, local_y};
use crate::editor::{EditorState, EditorTools, MAX_TOOL_SIZE, MIN_TOOL_SIZE, ToolKind};
use crate::render::{Point, f32_from_f64};
use crate::router::WindowSlot;
use crate::state::OverlayCore;

use super::{ChromeState, LayerDrag};

impl ChromeState {
    /// A pointer press in global logical coordinates. `true` when a chrome
    /// widget consumed it (the funnel then skips the F27 editor chain and
    /// the selection engine, and the release belongs to the chrome too).
    pub fn press(
        &mut self,
        button: MouseButton,
        at: LogicalPoint,
        editor: &mut EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) -> bool {
        let scale = f32_from_f64(output.scale);
        let local_pt = Point::new(local_x(output, at.x.0), local_y(output, at.y.0));
        let left = button == MouseButton::Left;

        // F27 P1: a visible wheel consumes EVERY press - a swatch picks,
        // the rainbow slot is the todo-27 eyedropper seam, anywhere else
        // hides (and the press dies with it).
        if self.color_wheel.visible {
            if left {
                let wheel = self.color_wheel.layout(editor, &self.tokens, scale, output);
                let palette = editor.config().editor.color_palette.clone();
                for (hex, rect) in palette.iter().zip(&wheel.swatches) {
                    if rect.contains(local_pt) {
                        self.pick_color(hex, editor);
                        return true;
                    }
                }
                if wheel.rainbow.contains(local_pt) {
                    tracing::debug!(
                        target: "flowshot_ui::chrome",
                        "rainbow slot pressed; the custom-pick eyedropper flow lands with todo 27"
                    );
                    return true;
                }
            }
            self.hide_color_wheel();
            return true;
        }

        let Some(selection) = selection else {
            return false;
        };

        let (toolbar_rect, buttons) = self.toolbar.layout(selection, &self.tokens, scale, output);
        if !buttons.is_empty() && toolbar_rect.contains(local_pt) {
            if left {
                for (rect, button) in buttons.iter().zip(&self.toolbar.buttons) {
                    if rect.contains(local_pt) {
                        toolbar_action(button, editor);
                        break;
                    }
                }
            }
            self.grabbed = true;
            return true;
        }

        if self.panel_shown(editor) {
            let panel = side_panel::layout(editor, selection, &self.tokens, scale, output);
            if panel.rect.contains(local_pt) {
                if left {
                    self.panel_press(&panel, local_pt, editor);
                }
                self.grabbed = true;
                return true;
            }
        }
        false
    }

    /// A pointer release: consumed while a chrome press holds the grab; a
    /// pending layer drag lands on the row under the release (press-drag-
    /// release reorder, ONE undo unit) and cancels on any other drop point.
    pub fn release(
        &mut self,
        at: LogicalPoint,
        editor: &mut EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) -> bool {
        if !self.grabbed {
            return false;
        }
        self.grabbed = false;
        if let Some(drag) = self.layer_drag.take()
            && let Some(selection) = selection
        {
            let scale = f32_from_f64(output.scale);
            let local_pt = Point::new(local_x(output, at.x.0), local_y(output, at.y.0));
            let panel = side_panel::layout(editor, selection, &self.tokens, scale, output);
            for (to_z, (_, rect)) in panel.layer_rows.iter().enumerate() {
                if rect.contains(local_pt) {
                    if to_z != drag.from_z {
                        editor.move_layer(drag.from_z, to_z);
                    }
                    break;
                }
            }
        }
        true
    }

    /// The color pick: the draw color (todo-20 seam), the selected object's
    /// color (the todo-25 property funnel, ONE undo unit, invert excluded),
    /// the persistence sink (F27 TOML write - the binary layer's seam), and
    /// the wheel hides.
    fn pick_color(&mut self, hex: &str, editor: &mut EditorState) {
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
    fn panel_press(
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
                    self.layer_drag = Some(LayerDrag { from_z: z });
                    break;
                }
            }
        }
    }
}

impl OverlayCore {
    /// The funnel's chrome-first press (widget parity: the chrome sees the
    /// press before the F27 editor chain).
    pub(crate) fn chrome_press(
        &mut self,
        slot: WindowSlot,
        button: MouseButton,
        at: LogicalPoint,
    ) -> bool {
        let Some(output) = self.router.output_for(slot) else {
            return false;
        };
        let selection = self.selection.rect();
        self.chrome
            .press(button, at, &mut self.editor, selection, output)
    }

    /// The funnel's chrome release (a chrome-consumed press grabbed it).
    pub(crate) fn chrome_release(&mut self, slot: WindowSlot, at: LogicalPoint) -> bool {
        let Some(output) = self.router.output_for(slot) else {
            return false;
        };
        let selection = self.selection.rect();
        self.chrome.release(at, &mut self.editor, selection, output)
    }

    /// The funnel's Space seam: the side-panel toggle (plan todo 26);
    /// `false` when the config gate is off and the key falls through.
    pub(crate) fn chrome_space(&mut self) -> bool {
        self.chrome.toggle_panel(&self.editor)
    }
}

/// One toolbar button press: tools activate; undo/redo drive the todo-25
/// journal; the W5 action ids (copy/save/upload/pin/open-app/exit) stay
/// no-op seams per the plan ("no action implementations behind buttons -
/// wired in W5 todos via callback traits").
fn toolbar_action(button: &ToolbarButton, editor: &mut EditorState) {
    match button {
        ToolbarButton::Tool(kind) => editor.activate_tool(*kind),
        ToolbarButton::Action(id) => match id.as_str() {
            "undo" => {
                let _ = editor.undo();
            }
            "redo" => {
                let _ = editor.redo();
            }
            _ => tracing::debug!(
                target: "flowshot_ui::chrome",
                action = id.as_str(),
                "toolbar action awaits its W5 callback trait"
            ),
        },
    }
}

/// Writes a tool-config change through the todo-20 settings seam
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

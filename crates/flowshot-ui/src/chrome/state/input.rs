//! The chrome input half (split from [`super`] at the 250-LOC ceiling): the
//! widget hit-tests behind the funnel's chrome-first ordering and the layer
//! drag-reorder release ([`EditorState::move_layer`], ONE undo unit). The
//! control writes (color pick, panel controls) live in [`super::controls`].

use std::time::Instant;

use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo};
use winit::event::MouseButton;

use crate::chrome::side_panel;
use crate::chrome::toolbar::ToolbarButton;
use crate::editor::EditorState;
use crate::editor::paint::{local_x, local_y};
use crate::input::Action;
use crate::render::{Point, f32_from_f64};
use crate::router::WindowSlot;
use crate::state::OverlayCore;

use super::ChromeState;

impl ChromeState {
    /// A pointer press in global logical coordinates. `true` when a chrome
    /// widget consumed it (the funnel then skips the Flameshot editor chain and
    /// the selection engine, and the release belongs to the chrome too).
    pub fn press(
        &mut self,
        button: MouseButton,
        at: LogicalPoint,
        editor: &mut EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
        now: Instant,
    ) -> bool {
        let scale = f32_from_f64(output.scale);
        let local_pt = Point::new(local_x(output, at.x.0), local_y(output, at.y.0));
        let left = button == MouseButton::Left;

        // Flameshot P1: a visible wheel consumes EVERY press - a swatch picks,
        // the rainbow slot is the eyedropper seam, anywhere else
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
                        "rainbow slot pressed; the custom-pick flow is owned by the eyedropper tool"
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
                let pressed = buttons.iter().position(|rect| rect.contains(local_pt));
                self.motion.set_press(pressed, now);
                if let Some(index) = pressed
                    && let Some(action) = toolbar_action(&self.toolbar.buttons[index], editor)
                {
                    self.pending_actions.push(action);
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
        now: Instant,
    ) -> bool {
        if !self.grabbed {
            return false;
        }
        self.grabbed = false;
        self.motion.set_press(None, now);
        self.aids.press = None;
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
}

impl OverlayCore {
    /// The funnel's chrome-first press (widget parity: the chrome sees the
    /// press before the Flameshot editor chain).
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
        // The aid chips ride the cursor-owning output only (the paint
        // rule); a press routed to any other slot must not phantom-consume.
        if self.cursor().is_some_and(|cursor| cursor.slot == slot)
            && self
                .chrome
                .aids_press(button, at, &mut self.editor, selection, output)
        {
            return true;
        }
        self.chrome.press(
            button,
            at,
            &mut self.editor,
            selection,
            output,
            Instant::now(),
        )
    }

    /// The funnel's chrome release (a chrome-consumed press grabbed it).
    pub(crate) fn chrome_release(&mut self, slot: WindowSlot, at: LogicalPoint) -> bool {
        let Some(output) = self.router.output_for(slot) else {
            return false;
        };
        let selection = self.selection.rect();
        self.chrome
            .release(at, &mut self.editor, selection, output, Instant::now())
    }

    /// The funnel's Space seam: the side-panel toggle;
    /// `false` when the config gate is off and the key falls through.
    pub(crate) fn chrome_space(&mut self) -> bool {
        self.chrome.toggle_panel(&self.editor)
    }
}

/// One toolbar button press: tools activate; undo/redo drive the
/// journal; the W5 action ids (copy/save/upload/pin/open-app/exit) become
/// shell actions (the chrome queues, the funnel drains,
/// the shell/binary layer executes).
fn toolbar_action(button: &ToolbarButton, editor: &mut EditorState) -> Option<Action> {
    match button {
        ToolbarButton::Tool(kind) => {
            editor.activate_tool(*kind);
            None
        }
        ToolbarButton::Action(id) => match id.as_str() {
            "undo" => {
                let _ = editor.undo();
                None
            }
            "redo" => {
                let _ = editor.redo();
                None
            }
            "copy" => Some(Action::Copy),
            "save" => Some(Action::Save),
            "pin" => Some(Action::Pin),
            "upload" => Some(Action::Upload),
            "open-app" => Some(Action::OpenWith),
            "exit" => Some(Action::Exit),
            _ => {
                tracing::warn!(
                    target: "flowshot_ui::chrome",
                    action = id.as_str(),
                    "unknown toolbar action id; press ignored"
                );
                None
            }
        },
    }
}

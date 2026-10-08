//! The per-window editor paint bridge.
//!
//! [`EditorView`] is the shell-supplied snapshot one window's paint needs;
//! [`EditorState::paint_into`] appends the baked pixel-effect overlay
//! (image quads above the backdrop, BELOW the scene - Flameshot
//! bakes redactions into the pixmap under all annotations), the scene
//! (paint order), the selected object's outline, and the active tool's live
//! visuals - the in-progress stroke, the mouse preview, and the
//! open edit session with its caret, selection, and IME composition
//! overlay - into the window's physical-px [`DisplayList`] with the
//! output's own scale.

use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo};
use winit::keyboard::ModifiersState;

use super::EditorState;
use super::outline;
use super::paint;
use super::tool::EditorContext;
use crate::render::DisplayList;

/// Everything one window's editor paint needs beyond the list and output.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorView {
    /// The live cursor (global logical) for the mouse preview; `None`
    /// before the first motion.
    pub mouse: Option<LogicalPoint>,
    /// The current selection (tool clamping context at paint time).
    pub selection: Option<LogicalRect>,
    /// The modifier snapshot (the painted preview honors the same Flameshot
    /// constrain conventions as the committed shape).
    pub modifiers: ModifiersState,
}

impl EditorState {
    /// Appends this window's editor visuals to `list`: the scene in paint
    /// order, the selected object's outline, and the active tool's
    /// in-progress shape / mouse preview / edit session.
    pub fn paint_into(&self, list: &mut DisplayList, output: &OutputInfo, view: EditorView) {
        self.paint_export_into(list, output);
        if let Some(object) = self.selected.and_then(|id| self.scene.get_object(id)) {
            outline::append_object_outline(list, output, object.bounding_rect());
        }
        let Some(tool) = self.tool.as_ref() else {
            return;
        };
        // The edit session paints from its own anchor - a missing cursor
        // track must not hide the text being edited.
        let editing = tool.edit_rect().is_some();
        let preview = self.config.mouse_preview && tool.show_mouse_preview();
        if !self.drawing && !preview && !editing {
            return;
        }
        let mouse = match view.mouse {
            Some(mouse) => mouse,
            None if editing => LogicalPoint::zero(),
            None => return,
        };
        let ctx = EditorContext {
            frame: self.frame.as_ref(),
            selection: view.selection,
            color: self.color,
            tool_size: self.sizes.get(self.active_kind),
            mouse,
            modifiers: view.modifiers,
            circle_count: self.scene.next_counter_value(),
            config: &self.config,
        };
        let family = Some(self.config.editor.font_family.as_str());
        let mut sink = paint::ListSink::new(list, output, family);
        tool.paint(&ctx, &mut sink);
    }

    /// Appends ONLY the committed export layer: the baked
    /// pixel-effect overlay (image quads above the backdrop, below
    /// the scene) plus the scene in paint order - no selected-object
    /// outline, no live tool visuals. This is exactly what the completion
    /// export renders, so the saved image never carries editing chrome.
    pub fn paint_export_into(&self, list: &mut DisplayList, output: &OutputInfo) {
        let family = Some(self.config.editor.font_family.as_str());
        for effect in &self.effects {
            let dst = paint::local_rect(output, effect.rect());
            list.image(effect.texture_id(), dst, None);
        }
        let mut sink = paint::ListSink::new(list, output, family);
        self.scene.paint(&mut sink);
    }
}

//! Tool activation lifecycle and edit-session event routing (plan todo
//! 20/22; split from the facade at the LOC ceiling).
//!
//! Activation/deactivation cancel the tool's own edit widget AND any
//! pending re-edit (the checkable-button parity: unchecking never leaves a
//! half-committed edit behind).
//!
//! While a tool edit widget is open, keys and IME events belong to the
//! session (Flameshot's child-widget focus): [`EditorState::editing_key_press`]
//! intercepts every non-Escape key BEFORE the normal key map (so typing `t`
//! inserts text instead of toggling the tool, and digits never resize), and
//! [`EditorState::ime_event`] feeds winit `Ime` events to the tool. Esc
//! stays with the selection engine's cascade (stage 1 deselects the tool,
//! which cancels the edit - the plan's "Esc mid-edit cancels" QA path).
//!
//! Re-editing an existing object (a press on committed text with the text
//! tool active) removes the object from the scene PROVISIONALLY - the
//! [`Reedit`] snapshot restores it on cancel/empty-commit, and the commit
//! pushes (snapshot, scene+new object) as ONE undo unit.

use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_core::scene::ToolObject;
use winit::event::Ime;
use winit::keyboard::KeyCode;

use super::EditorState;
use super::kind::ToolKind;
use super::tool::EditKey;
use super::types::{EditorEnv, EditorUpdate};
use super::undo::Snapshot;

/// The provisional re-edit state: the full (scene, effects) snapshot taken
/// when the tool took over an existing object (exact restore on cancel -
/// nothing else mutates the editor state while an edit widget is open).
#[derive(Debug)]
pub(super) struct Reedit {
    before: Snapshot,
}

impl EditorState {
    /// Checks a tool (creating it from the registry; an unregistered kind
    /// warn-logs and stays unchecked).
    pub fn activate_tool(&mut self, kind: ToolKind) {
        let Some(mut tool) = self.registry.create(kind) else {
            tracing::warn!(
                target: "flowshot_ui::editor",
                tool = kind.id(),
                "tool not registered; activation ignored"
            );
            return;
        };
        tool.on_color_changed(self.color);
        tool.on_size_changed(self.sizes.get(Some(kind)));
        if let Some(previous) = self.tool.as_mut() {
            previous.cancel_edit();
        }
        self.cancel_reedit();
        self.tool = Some(tool);
        self.active_kind = Some(kind);
        self.drawing = false;
        self.digits.reset();
        tracing::info!(target: "flowshot_ui::editor", tool = kind.id(), "tool activated");
    }

    /// Unchecks the active tool (Esc cascade stage 1 reaction). The tool's
    /// own edit session is cancelled, but a detached edit-widget flag
    /// survives until stage 4 (Flameshot parity: unchecking the tool button
    /// does NOT delete `m_toolWidget` - `deleteToolWidgetOrClose` walks the
    /// stages independently).
    pub fn deactivate_tool(&mut self) {
        let Some(kind) = self.active_kind else {
            return;
        };
        if let Some(tool) = self.tool.as_mut() {
            tool.cancel_edit();
        }
        self.cancel_reedit();
        self.cancel_move_selection();
        self.tool = None;
        self.active_kind = None;
        self.drawing = false;
        self.digits.reset();
        tracing::info!(target: "flowshot_ui::editor", tool = kind.id(), "tool deactivated");
    }

    /// Activation-key semantics: re-pressing the active tool's key unchecks
    /// it (the Flameshot checkable-button toggle).
    pub fn toggle_tool(&mut self, kind: ToolKind) {
        if self.active_kind == Some(kind) {
            self.deactivate_tool();
        } else {
            self.activate_tool(kind);
        }
    }

    /// Key routing while an edit widget is active; `None` when not editing
    /// (the caller runs the normal key map). Ctrl+Return commits (the F27
    /// text lifecycle); everything else goes to the tool's session; keys the
    /// session rejects (Ctrl+C, ...) pass through to the funnel - the
    /// selection engine's own map still applies to those.
    pub(super) fn editing_key_press(
        &mut self,
        env: &EditorEnv,
        code: KeyCode,
        repeat: bool,
        text: Option<&str>,
    ) -> Option<EditorUpdate> {
        if !self.editing() || code == KeyCode::Escape {
            return None;
        }
        let at = env.mouse.unwrap_or_default();
        let ctrl = env.modifiers.control_key();
        if ctrl
            && !env.modifiers.alt_key()
            && !env.modifiers.super_key()
            && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter)
        {
            self.commit_edit(env, at);
            return Some(EditorUpdate::eaten(true));
        }
        let key = EditKey { code, text, repeat };
        let handled = self
            .with_ctx(env, at, |ctx, tool| tool.edit_key(ctx, key))
            .unwrap_or(false);
        Some(if handled {
            EditorUpdate::eaten(true)
        } else {
            EditorUpdate::pass()
        })
    }

    /// An IME event: routed to the active edit session (the always-on model
    /// of draft D7 - the shell plumbs every `WindowEvent::Ime` here).
    pub fn ime_event(&mut self, env: &EditorEnv, ime: &Ime) -> EditorUpdate {
        if !self.editing() {
            return EditorUpdate::pass();
        }
        let at = env.mouse.unwrap_or_default();
        let handled = self
            .with_ctx(env, at, |ctx, tool| tool.ime(ctx, ime))
            .unwrap_or(false);
        if handled {
            EditorUpdate::eaten(true)
        } else {
            EditorUpdate::pass()
        }
    }

    /// The caret rect for the shell's `set_ime_cursor_area` mirror (the
    /// IME popup anchors at the caret; `None` while not editing).
    #[must_use]
    pub fn ime_cursor_area(&self) -> Option<LogicalRect> {
        self.tool.as_ref()?.caret_rect()
    }

    /// Removes the re-edited object from the scene and snapshots for the
    /// undo pair / cancel restore (called by the funnel's re-edit probe).
    pub(super) fn begin_reedit(&mut self, id: usize) {
        let before = self.snapshot();
        if self.scene.remove_object(id).is_none() {
            return;
        }
        self.reedit = Some(Reedit { before });
        // The removal shifts ids above it - the selection is invalidated
        // (the editor's undo/redo discipline).
        self.selected = None;
        tracing::debug!(target: "flowshot_ui::editor", "re-edit started");
    }

    /// Restores the provisionally removed object (cancel / empty commit).
    pub(super) fn cancel_reedit(&mut self) {
        let Some(reedit) = self.reedit.take() else {
            return;
        };
        self.restore(reedit.before);
        tracing::debug!(target: "flowshot_ui::editor", "re-edit cancelled; object restored");
    }

    /// Commits the active edit widget (click-outside / Ctrl+Return); the
    /// produced object becomes one undo unit (re-edit replacement
    /// bookkeeping lives in [`EditorState::commit_edit_object`]).
    pub(super) fn commit_edit(&mut self, env: &EditorEnv, at: LogicalPoint) {
        let object = self
            .with_ctx(env, at, |ctx, tool| tool.commit_edit(ctx))
            .flatten();
        let committed = object.is_some();
        self.commit_edit_object(object);
        tracing::debug!(
            target: "flowshot_ui::editor",
            committed,
            "edit committed"
        );
    }

    /// The edit-commit bookkeeping: a produced object becomes ONE undo unit
    /// (paired with the re-edit snapshot when one is pending); no object
    /// with a pending re-edit restores the old one (an empty commit never
    /// destroys the re-edited text).
    pub(super) fn commit_edit_object(&mut self, object: Option<Box<dyn ToolObject>>) {
        match (object, self.reedit.take()) {
            (Some(object), Some(reedit)) => {
                let id = self.scene.add_object(object);
                self.undo.push(reedit.before, self.snapshot());
                tracing::info!(
                    target: "flowshot_ui::editor",
                    object = self.scene.get_object(id).map_or("?", ToolObject::type_id),
                    objects = self.scene.object_count(),
                    undo_depth = self.undo.undo_depth(),
                    reedit = true,
                    "object committed"
                );
            }
            (Some(object), None) => {
                self.commit_object(object);
            }
            (None, Some(reedit)) => {
                self.restore(reedit.before);
                tracing::debug!(
                    target: "flowshot_ui::editor",
                    "empty re-edit commit; old object restored"
                );
            }
            (None, None) => {}
        }
    }
}

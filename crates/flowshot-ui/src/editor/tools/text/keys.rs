//! The text tool's input handlers (plan todo 22): the edit-key map and the
//! winit `Ime` event routing into the session state machine. Split from
//! [`super::TextTool`] at the 250-LOC ceiling; the lifecycle lives there.

use cosmic_text::Motion;
use winit::event::Ime;
use winit::keyboard::KeyCode;

use super::super::super::tool::{EditKey, EditorContext};
use super::TextTool;

impl TextTool {
    /// Handles one key routed to the edit session; `true` consumes it.
    /// Ctrl+Return is rejected (the editor funnel owns the commit), Esc
    /// belongs to the cascade, and only unmodified non-control text
    /// inserts (Ctrl+C must reach the selection engine's copy).
    pub(super) fn handle_edit_key(&mut self, ctx: &EditorContext<'_>, key: EditKey<'_>) -> bool {
        let Some(session) = &mut self.session else {
            return false;
        };
        let shift = ctx.modifiers.shift_key();
        let ctrl = ctx.modifiers.control_key();
        let plain = !ctrl && !ctx.modifiers.alt_key() && !ctx.modifiers.super_key();
        match key.code {
            KeyCode::Backspace => session.backspace(),
            KeyCode::Delete => session.delete(),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if ctrl {
                    return false;
                }
                session.enter();
            }
            KeyCode::ArrowLeft => session.move_cursor(
                if ctrl {
                    Motion::PreviousWord
                } else {
                    Motion::Left
                },
                shift,
            ),
            KeyCode::ArrowRight => session.move_cursor(
                if ctrl {
                    Motion::NextWord
                } else {
                    Motion::Right
                },
                shift,
            ),
            KeyCode::ArrowUp => session.move_cursor(Motion::Up, shift),
            KeyCode::ArrowDown => session.move_cursor(Motion::Down, shift),
            KeyCode::Home => session.move_cursor(Motion::Home, shift),
            KeyCode::End => session.move_cursor(Motion::End, shift),
            KeyCode::PageUp => session.move_cursor(Motion::PageUp, shift),
            KeyCode::PageDown => session.move_cursor(Motion::PageDown, shift),
            KeyCode::KeyA if ctrl => session.select_all(),
            KeyCode::Escape | KeyCode::Tab => return false,
            _ => {
                let Some(text) = key.text else {
                    return false;
                };
                if !plain || text.is_empty() || text.chars().any(char::is_control) {
                    return false;
                }
                session.insert_str(text);
            }
        }
        true
    }

    /// Handles one winit IME event (the always-on model, draft D7); `true`
    /// consumes it. The composition lives outside the buffer, so only
    /// `Commit` mutates text (the iced 0.14 reference semantics).
    pub(super) fn handle_ime(&mut self, ime: &Ime) -> bool {
        let Some(session) = &mut self.session else {
            return false;
        };
        match ime {
            Ime::Enabled => {
                tracing::trace!(target: "flowshot_ui::editor", "ime enabled in text session");
            }
            Ime::Preedit(text, cursor) => {
                tracing::trace!(target: "flowshot_ui::editor", %text, "ime preedit");
                session.set_preedit(text, *cursor);
            }
            Ime::Commit(text) => {
                tracing::debug!(
                    target: "flowshot_ui::editor",
                    chars = text.chars().count(),
                    "ime commit"
                );
                session.commit_ime(text);
            }
            Ime::Disabled => {
                tracing::trace!(target: "flowshot_ui::editor", "ime disabled; composition discarded");
                session.cancel_ime();
            }
        }
        true
    }
}

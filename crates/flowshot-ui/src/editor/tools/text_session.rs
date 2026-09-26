//! The text-editing session state machine (plan todo 22).
//!
//! The editing buffer the plan mandates: a cosmic-text [`Editor`] (cursor,
//! selection, backspace/delete, arrows, home/end, word wrap within the
//! drag-defined box) plus the IME preedit state. The preedit follows the
//! iced 0.14 reference model (plan F26 cite, `iced/winit/src/window.rs`
//! L189-303): the composition string is held OUTSIDE the buffer and drawn
//! as an underlined overlay at the caret - only [`TextSession::commit_ime`]
//! writes to the buffer, so a cancelled composition (winit `Ime::Disabled`
//! without a `Commit`) discards cleanly and the acceptance sequence
//! `Preedit("ni") -> Preedit("nih") -> Commit("日")` yields exactly `"日"`.
//!
//! Layout queries (caret rect, selection highlights, wrapped-line baking)
//! live in [`super::text_measure`] - this file owns the mutations. Every
//! mutation ends with the buffer shaped, so the `&self` paint path only
//! reads layout data.

use cosmic_text::{
    Action, Attrs, AttrsList, Buffer, Edit, Editor, Family, Metrics, Motion, Selection, Shaping,
};

use super::super::paint::LINE_HEIGHT_RATIO;
use super::text_font::with_font_system;

/// The pending IME composition (winit `Ime::Preedit` payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Preedit {
    /// The composition string (displayed underlined at the caret).
    pub text: String,
    /// Byte-offset `(start, end)` cursor range within `text`, when the IME
    /// sent one (winit validates the char boundaries).
    pub cursor: Option<(usize, usize)>,
}

/// One live text-editing session: the cosmic-text editor plus the IME
/// composition state. Buffer-local coordinates are LOGICAL px at the
/// session's point size - the tool converts to scene space.
#[derive(Debug)]
pub(super) struct TextSession {
    editor: Editor<'static>,
    attrs: AttrsList,
    family: String,
    preedit: Option<Preedit>,
    point_size: f32,
    wrap_width: Option<f32>,
}

/// The family attribute for a config family name (empty = sans-serif
/// default with fontconfig fallback - the CJK coverage path).
pub(super) fn family_attrs(family: &str) -> Attrs<'_> {
    if family.is_empty() {
        Attrs::new()
    } else {
        Attrs::new().family(Family::Name(family))
    }
}

impl TextSession {
    /// A fresh empty session.
    pub(super) fn new(family: &str, point_size: f32, wrap_width: Option<f32>) -> Self {
        Self::build(family, point_size, wrap_width, "")
    }

    /// A session pre-loaded with `text` (the re-edit path), fully selected
    /// (Flameshot `TextTool::widget()` calls `selectAll()` on the preserved
    /// old text - typing immediately replaces it).
    pub(super) fn with_text(
        family: &str,
        point_size: f32,
        wrap_width: Option<f32>,
        text: &str,
    ) -> Self {
        let mut session = Self::build(family, point_size, wrap_width, text);
        session.select_all();
        session
    }

    fn build(family: &str, point_size: f32, wrap_width: Option<f32>, text: &str) -> Self {
        let metrics = Metrics::new(point_size, point_size * LINE_HEIGHT_RATIO);
        let buffer = with_font_system(
            |fonts| {
                let mut buffer = Buffer::new(fonts, metrics);
                buffer.set_size(wrap_width, None);
                buffer.set_text(text, &family_attrs(family), Shaping::Advanced, None);
                buffer
            },
            {
                let mut buffer = Buffer::new_empty(metrics);
                buffer.set_size(wrap_width, None);
                buffer
            },
        );
        let mut session = Self {
            editor: Editor::new(buffer),
            attrs: AttrsList::new(&family_attrs(family)),
            family: family.to_owned(),
            preedit: None,
            point_size,
            wrap_width,
        };
        session.reshape();
        session
    }

    /// Inserts `text` at the cursor, replacing any selection (the IME
    /// commit and direct keyboard text paths).
    pub(super) fn insert_str(&mut self, text: &str) {
        self.editor.insert_string(text, Some(self.attrs.clone()));
        self.reshape();
    }

    /// Inserts a line break (plain Enter; Ctrl+Return commits instead -
    /// the editor funnel intercepts it before the session sees the key).
    pub(super) fn enter(&mut self) {
        self.action(Action::Enter);
    }

    /// Deletes behind the cursor.
    pub(super) fn backspace(&mut self) {
        self.action(Action::Backspace);
    }

    /// Deletes in front of the cursor.
    pub(super) fn delete(&mut self) {
        self.action(Action::Delete);
    }

    /// Moves the cursor; `extend` grows the selection from its anchor
    /// (Shift+motion - the cosmic-edit selection convention).
    pub(super) fn move_cursor(&mut self, motion: Motion, extend: bool) {
        if extend {
            if self.editor.selection() == Selection::None {
                self.editor
                    .set_selection(Selection::Normal(self.editor.cursor()));
            }
        } else {
            self.editor.set_selection(Selection::None);
        }
        self.action(Action::Motion(motion));
    }

    /// Selects the whole buffer (Ctrl+A; the re-edit entry state).
    pub(super) fn select_all(&mut self) {
        with_font_system(
            |fonts| {
                self.editor
                    .action(fonts, Action::Motion(Motion::BufferStart));
                self.editor
                    .set_selection(Selection::Normal(self.editor.cursor()));
                self.editor.action(fonts, Action::Motion(Motion::BufferEnd));
                self.editor.shape_as_needed(fonts, false);
            },
            (),
        );
    }

    /// Places the cursor at the buffer-local point (a click inside the edit
    /// box; clears the selection).
    pub(super) fn click(&mut self, x: f32, y: f32) {
        // The i32 range as exact f32 bounds (both are powers of two).
        const LOW: f32 = -2_147_483_648.0;
        const HIGH: f32 = 2_147_483_647.0;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "buffer-local px are clamped into the i32 range before the cast"
        )]
        let (x, y) = (x.clamp(LOW, HIGH) as i32, y.clamp(LOW, HIGH) as i32);
        self.action(Action::Click { x, y });
    }

    /// Records a composition update (winit `Ime::Preedit`); an empty string
    /// clears the pending composition (winit sends `Preedit("")` before a
    /// commit or cancel - the wayland backend's clear path).
    pub(super) fn set_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        self.preedit = (!text.is_empty()).then(|| Preedit {
            text: text.to_owned(),
            cursor,
        });
    }

    /// Writes an IME commit into the buffer, dropping any pending
    /// composition (winit `Ime::Commit`).
    pub(super) fn commit_ime(&mut self, text: &str) {
        self.preedit = None;
        self.insert_str(text);
    }

    /// Discards a pending composition without committing (winit
    /// `Ime::Disabled` - the acceptance cancel path).
    pub(super) fn cancel_ime(&mut self) {
        self.preedit = None;
    }

    /// The pending composition, when any.
    pub(super) const fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    /// The buffer text (logical lines joined with `\n`; the composition is
    /// NOT included - it only enters through [`Self::commit_ime`]).
    pub(super) fn text(&self) -> String {
        self.editor.with_buffer(|buffer| {
            let mut text = String::new();
            for (index, line) in buffer.lines.iter().enumerate() {
                if index > 0 {
                    text.push('\n');
                }
                text.push_str(line.text());
            }
            text
        })
    }

    /// Whether the buffer holds no text (the empty-commit rule: Flameshot
    /// `TextTool::isValid` is `!m_text.isEmpty()`).
    pub(super) fn is_empty(&self) -> bool {
        self.editor
            .with_buffer(|buffer| buffer.lines.iter().all(|line| line.text().is_empty()))
    }

    /// Resizes the text (digits/wheel while editing): metrics update and
    /// the layout reflows.
    pub(super) fn set_point_size(&mut self, point_size: f32) {
        self.point_size = point_size;
        with_font_system(
            |fonts| {
                self.editor.with_buffer_mut(|buffer| {
                    buffer.set_metrics(Metrics::new(point_size, point_size * LINE_HEIGHT_RATIO));
                });
                self.editor.shape_as_needed(fonts, false);
            },
            (),
        );
    }

    /// Sets (or clears with `None`) the wrap width - the drag-defined box.
    pub(super) fn set_wrap_width(&mut self, wrap_width: Option<f32>) {
        self.wrap_width = wrap_width;
        with_font_system(
            |fonts| {
                self.editor
                    .with_buffer_mut(|buffer| buffer.set_size(wrap_width, None));
                self.editor.shape_as_needed(fonts, false);
            },
            (),
        );
    }

    /// The point size in effect.
    pub(super) const fn point_size(&self) -> f32 {
        self.point_size
    }

    /// The configured family name (empty = platform sans-serif).
    pub(super) fn family(&self) -> &str {
        &self.family
    }

    /// The cosmic-text editor (read access for the measurement half).
    pub(super) const fn editor(&self) -> &Editor<'static> {
        &self.editor
    }

    fn action(&mut self, action: Action) {
        with_font_system(
            |fonts| {
                self.editor.action(fonts, action);
                self.editor.shape_as_needed(fonts, false);
            },
            (),
        );
    }

    fn reshape(&mut self) {
        with_font_system(|fonts| self.editor.shape_as_needed(fonts, false), ());
    }
}

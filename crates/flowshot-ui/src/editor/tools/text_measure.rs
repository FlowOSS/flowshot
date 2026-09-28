//! Layout measurement for the text session.
//!
//! The read-only half of [`TextSession`]: caret geometry (also the IME
//! cursor-area source), the laid-out extent (the edit box), selection
//! highlight spans, the wrapped-line bake for the committed object, and
//! string measurement for the preedit overlay. All coordinates are
//! buffer-local LOGICAL px - the tool offsets them by the scene anchor.

use cosmic_text::{Buffer, Edit, Metrics, Shaping};

use super::super::paint::LINE_HEIGHT_RATIO;
use super::text_font::with_font_system;
use super::text_session::{TextSession, family_attrs};

/// The caret geometry in buffer-local logical px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Caret {
    /// Horizontal caret position (left edge).
    pub x: f32,
    /// Top of the caret's line.
    pub y: f32,
    /// Caret height (the line height).
    pub height: f32,
}

/// One selection highlight span, `[x, y, width, height]` buffer-local.
pub(super) type HighlightSpan = [f32; 4];

impl TextSession {
    /// The caret geometry (falls back to the buffer origin when the line
    /// is not laid out - an empty fresh session).
    pub(super) fn caret(&self) -> Caret {
        let height = self.point_size() * LINE_HEIGHT_RATIO;
        let cursor = self.editor().cursor();
        let (x, y) = self
            .editor()
            .with_buffer(|buffer| buffer.cursor_position(&cursor))
            .unwrap_or((0.0, 0.0));
        Caret { x, y, height }
    }

    /// The laid-out text extent `(width, height)`, at least one line tall
    /// (the empty session still owns a clickable line).
    pub(super) fn layout_size(&self) -> (f32, f32) {
        let min_height = self.point_size() * LINE_HEIGHT_RATIO;
        self.editor().with_buffer(|buffer| {
            let mut width = 0.0_f32;
            let mut height = 0.0_f32;
            for run in buffer.layout_runs() {
                width = width.max(run.line_w);
                height = height.max(run.line_top + run.line_height);
            }
            (width, height.max(min_height))
        })
    }

    /// The selection highlight spans (empty without a selection).
    pub(super) fn selection_spans(&self) -> Vec<HighlightSpan> {
        let Some((start, end)) = self.editor().selection_bounds() else {
            return Vec::new();
        };
        self.editor().with_buffer(|buffer| {
            let mut spans = Vec::new();
            for run in buffer.layout_runs() {
                if run.line_i < start.line || run.line_i > end.line {
                    continue;
                }
                for (x, width) in run.highlight(start, end) {
                    spans.push([x, run.line_top, width, run.line_height]);
                }
            }
            spans
        })
    }

    /// The commit text: visual (wrapped) lines baked with explicit `\n`
    /// breaks. The committed scene object paints through
    /// `PaintSink::draw_text`, which carries no wrap width - baking keeps
    /// the committed render identical to the edit view (the plan's single
    /// shaping path). Hard breaks the user typed survive as-is.
    pub(super) fn baked_text(&self) -> String {
        self.editor().with_buffer(|buffer| {
            let mut lines: Vec<&str> = Vec::new();
            for run in buffer.layout_runs() {
                let slice = match (run.glyphs.first(), run.glyphs.last()) {
                    (Some(first), Some(last)) => {
                        run.text.get(first.start..last.end).unwrap_or(run.text)
                    }
                    // An empty visual line (a bare hard break) bakes as
                    // an empty string, preserving the break.
                    _ => "",
                };
                lines.push(slice);
            }
            lines.join("\n")
        })
    }

    /// Measures `text` at the session metrics (the preedit overlay extent);
    /// `0` when the font system is unavailable.
    pub(super) fn measure(&self, text: &str) -> f32 {
        with_font_system(
            |fonts| {
                let mut buffer = Buffer::new(
                    fonts,
                    Metrics::new(self.point_size(), self.point_size() * LINE_HEIGHT_RATIO),
                );
                buffer.set_text(text, &family_attrs(self.family()), Shaping::Advanced, None);
                buffer.shape_until_scroll(fonts, false);
                buffer
                    .layout_runs()
                    .map(|run| run.line_w)
                    .fold(0.0_f32, f32::max)
            },
            0.0,
        )
    }
}

//! The text tool's edit-overlay paint (plan todo 22): the buffer text, the
//! selection highlights, the caret bar, and the underlined IME composition
//! overlay at the caret. Split from [`super::TextTool`] at the 250-LOC
//! ceiling; everything paints through the SAME [`PaintSink`] vocabulary the
//! committed scene objects use (the plan's single shaping path - the
//! committed `TextObject` and this overlay both land in `draw_text`).

use flowshot_core::scene::{
    Color as SceneColor, PaintSink, Point as ScenePoint, Rect as SceneRect,
};

use super::TextTool;

impl TextTool {
    /// Paints the live edit session (text + selection + caret + preedit).
    pub(super) fn paint_session(&self, sink: &mut dyn PaintSink) {
        let Some(session) = &self.session else {
            return;
        };
        for [x, y, width, height] in session.selection_spans() {
            sink.fill_rect(
                SceneRect::new(self.anchor.x + x, self.anchor.y + y, width, height),
                self.selection_color(),
            );
        }
        let text = session.text();
        if !text.is_empty() {
            sink.draw_text(self.anchor, &text, self.point_size(), self.color);
        }
        let caret = session.caret();
        let caret_y = self.anchor.y + caret.y;
        let mut caret_x = self.anchor.x + caret.x;
        if let Some(preedit) = session.preedit() {
            // The composition overlay: drawn at the caret, underlined, with
            // the caret moved to the IME's cursor offset within it.
            sink.draw_text(
                ScenePoint::new(caret_x, caret_y),
                &preedit.text,
                self.point_size(),
                self.color,
            );
            let width = session.measure(&preedit.text).max(1.0);
            sink.fill_rect(
                SceneRect::new(
                    caret_x,
                    caret_y + caret.height - super::UNDERLINE_THICKNESS,
                    width,
                    super::UNDERLINE_THICKNESS,
                ),
                self.color,
            );
            caret_x += match preedit.cursor {
                Some((start, _)) => {
                    session.measure(preedit.text.get(..start).unwrap_or(&preedit.text))
                }
                None => width,
            };
        }
        sink.fill_rect(
            SceneRect::new(caret_x, caret_y, super::CARET_WIDTH, caret.height),
            self.color,
        );
    }

    /// The selection highlight color (the text color at quarter strength).
    pub(super) fn selection_color(&self) -> SceneColor {
        SceneColor::new(
            self.color.r,
            self.color.g,
            self.color.b,
            super::SELECTION_ALPHA,
        )
    }
}

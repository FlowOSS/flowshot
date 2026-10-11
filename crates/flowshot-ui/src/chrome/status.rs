//! The session-status pill row: one label-only chip stating which rasterizer
//! presents the window — `GPU` quiet when all is well, accent-tinted
//! `CPU fallback` when the session lost the GPU frame target and the CPU
//! fallback took over (adr-007).
//!
//! Placement rides the SELECTION, like the quick-aids cluster and the
//! toolbar plate: the pill hugs one of the four slots just outside the
//! selection rect on a medium margin, preferring the slot diagonally
//! opposite the aid cluster's above-leading default and walking through the
//! rest, skipping any slot the toolbar plate or the aid chips already own.
//! Unlike those two, the pill never clamps into the output: when no slot
//! fits clear — no selection yet, or a selection that leaves no free edge —
//! the pill is simply GONE. It is decoration, never a control: the chrome
//! funnel never hit-tests it, so it cannot steal a click (unlike the aid
//! chips, which are clickable toggles).

use flowshot_core::geometry::{LogicalRect, OutputInfo};

use super::state::ChromeState;
use crate::editor::EditorState;
use crate::editor::paint::local_rect;
use crate::render::{DisplayList, Rect, Size, f32_from_f64, f32_from_i32};
use crate::widgets::StatusChip;
use flowshot_core::tokens::DesignTokens;

/// The healthy-path pill's label: which rasterizer presents the window.
/// Bare on purpose — the healthy path needs no ceremony, only a read.
pub(crate) const GPU_LABEL: &str = "GPU";

/// The CPU-fallback pill's label: the fact stated, kept short so the pill
/// fits beside a selection at 1x.
pub(crate) const CPU_LABEL: &str = "CPU fallback";

impl ChromeState {
    /// Paints the raster-path pill beside one output's selection. The CALLER
    /// owns the path read
    /// ([`ChromeState::cpu_path_for`](crate::chrome::state::ChromeState::cpu_path_for)
    /// reads the slots the shell recorded at renderer init) and paints AFTER
    /// the aid chips, so the pill never competes with them for z-order.
    pub fn paint_render_status(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
        cpu_path: bool,
    ) {
        let Some(selection) = selection else {
            return;
        };
        let tokens = self.tokens();
        let scale = f32_from_f64(output.scale);
        let label = if cpu_path { CPU_LABEL } else { GPU_LABEL };
        let size = StatusChip::measured_size(label, tokens, scale);
        let blocked = self.obstacles(editor, selection, tokens, scale, output);
        let Some(rect) = status_rect(
            size,
            output,
            tokens,
            &blocked,
            local_rect(output, selection),
        ) else {
            return;
        };
        StatusChip {
            rect,
            label,
            active: cpu_path,
        }
        .draw(list, tokens, scale);
    }

    /// The rects the pill must not land on: the toolbar plate and the aid
    /// chips — the two chrome elements that already hug the selection.
    fn obstacles(
        &self,
        editor: &EditorState,
        selection: LogicalRect,
        tokens: &DesignTokens,
        scale: f32,
        output: &OutputInfo,
    ) -> [Rect; 3] {
        let (plate, _) = self.toolbar.layout(selection, tokens, scale, output);
        let chips = self.aids_layout(editor, Some(selection), output);
        [plate, chips[0], chips[1]]
    }
}

/// The pill's slot: the first of the four selection-adjacent slots that sits
/// fully inside the output's margin band and misses every blocked rect, or
/// `None` when none fits — the pill is gone rather than clamped onto the
/// scene.
#[must_use]
pub(crate) fn status_rect(
    size: Size,
    output: &OutputInfo,
    tokens: &DesignTokens,
    blocked: &[Rect],
    selection: Rect,
) -> Option<Rect> {
    let scale = f32_from_f64(output.scale);
    let margin = tokens.spacing.medium as f32 * scale;
    let width = f32_from_i32(output.physical_size.width.0);
    let height = f32_from_i32(output.physical_size.height.0);
    let below = selection.bottom() + margin;
    let above = selection.origin.y - margin - size.height;
    let leading = selection.origin.x;
    let trailing = selection.right() - size.width;
    let candidates = [
        Rect::from_parts(trailing, below, size.width, size.height),
        Rect::from_parts(leading, below, size.width, size.height),
        Rect::from_parts(trailing, above, size.width, size.height),
        Rect::from_parts(leading, above, size.width, size.height),
    ];
    let fits = |rect: &Rect| {
        rect.origin.x >= margin
            && rect.origin.y >= margin
            && rect.right() <= width - margin
            && rect.bottom() <= height - margin
    };
    candidates
        .iter()
        .filter(|rect| fits(rect))
        .find(|candidate| !blocked.iter().any(|other| other.intersects(candidate)))
        .copied()
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "fixture construction validates a known-good fixture"
    )]

    use super::*;
    use crate::render::{Command, Point};
    use flowshot_core::geometry::{LogicalRect, PhysicalSize, Transform};

    fn output() -> OutputInfo {
        OutputInfo::new(
            "DP-1",
            "DP-1",
            LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(1920, 1080),
            1.0,
            Transform::Normal,
        )
        .expect("valid fixture output")
    }

    fn tokens() -> DesignTokens {
        DesignTokens::default()
    }

    const MARGIN: f32 = 8.0;
    const SIZE: Size = Size {
        width: 90.0,
        height: 20.0,
    };
    /// A mid-screen selection: every adjacent slot fits on-screen.
    const MID_SEL: Rect = Rect {
        origin: Point { x: 400.0, y: 300.0 },
        size: Size {
            width: 800.0,
            height: 450.0,
        },
    };

    #[test]
    fn the_pill_hugs_the_selection_below_trailing() {
        // Given a mid-screen selection and nothing blocking, when the pill
        // is placed, then it sits below the selection, aligned to its
        // trailing edge, one margin clear.
        let rect = status_rect(SIZE, &output(), &tokens(), &[], MID_SEL)
            .expect("a mid-screen selection leaves every slot free");
        assert_eq!(rect.origin.x, MID_SEL.right() - SIZE.width);
        assert_eq!(rect.origin.y, MID_SEL.bottom() + MARGIN);
    }

    #[test]
    fn the_pill_walks_to_the_next_free_slot() {
        // Given the below-trailing slot owned by another chrome element,
        // when the pill is placed, then it takes the below-leading slot.
        let owner = Rect::from_parts(
            MID_SEL.right() - SIZE.width,
            MID_SEL.bottom() + MARGIN,
            SIZE.width,
            SIZE.height,
        );
        let rect = status_rect(SIZE, &output(), &tokens(), &[owner], MID_SEL)
            .expect("the walk finds the below-leading slot");
        assert_eq!(rect.origin.x, MID_SEL.origin.x);
        assert_eq!(rect.origin.y, MID_SEL.bottom() + MARGIN);
        assert!(!owner.intersects(&rect), "the pill left the owned slot");
    }

    #[test]
    fn the_pill_is_gone_when_no_slot_fits() {
        // Given a selection that fills the output, every adjacent slot runs
        // off the margin band - the pill is gone, never clamped onto the
        // scene.
        let full = Rect::from_parts(0.0, 0.0, 1920.0, 1080.0);
        assert!(status_rect(SIZE, &output(), &tokens(), &[], full).is_none());
    }

    #[test]
    fn the_pill_is_gone_without_a_selection() {
        let mut list = DisplayList::new();
        ChromeState::new().paint_render_status(
            &mut list,
            &EditorState::default(),
            None,
            &output(),
            true,
        );
        assert!(
            list.iter().next().is_none(),
            "nothing painted before a selection exists"
        );
    }

    #[test]
    fn the_pill_names_the_path_in_both_states() {
        let selection = LogicalRect::from_raw(400.0, 300.0, 800.0, 450.0);
        let labels = |cpu_path: bool| {
            let mut list = DisplayList::new();
            ChromeState::new().paint_render_status(
                &mut list,
                &EditorState::default(),
                Some(selection),
                &output(),
                cpu_path,
            );
            list.iter()
                .filter_map(|command| match command {
                    Command::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(false), vec![GPU_LABEL], "the healthy read");
        assert_eq!(labels(true), vec![CPU_LABEL], "the degraded read");
    }

    #[test]
    fn the_pill_is_one_body_and_one_label() {
        let selection = LogicalRect::from_raw(400.0, 300.0, 800.0, 450.0);
        let mut list = DisplayList::new();
        ChromeState::new().paint_render_status(
            &mut list,
            &EditorState::default(),
            Some(selection),
            &output(),
            true,
        );
        let fills = list
            .iter()
            .filter(|command| matches!(*command, Command::Fill { .. }))
            .count();
        let texts = list
            .iter()
            .filter(|command| matches!(*command, Command::Text { .. }))
            .count();
        assert_eq!(fills, 1, "the pill body only — no key plate, no wash");
        assert_eq!(texts, 1, "one label, no key glyph");
    }

    #[test]
    fn the_pill_scales_with_the_output() {
        let small = StatusChip::measured_size(CPU_LABEL, &tokens(), 1.0);
        let doubled = StatusChip::measured_size(CPU_LABEL, &tokens(), 2.0);
        assert_eq!(doubled.width, small.width * 2.0);
        assert_eq!(doubled.height, small.height * 2.0);
    }
}

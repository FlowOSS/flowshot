//! The session-status pill row: one label-only chip stating a fact the user
//! should SEE but never click — today its only member is the CPU raster
//! fallback (adr-007), so a session that lost the GPU frame target says so
//! quietly instead of silently rendering differently.
//!
//! Placement is the output's TRAILING-BOTTOM corner on a medium margin:
//! diagonally opposite the quick-aids cluster's default above-leading slot,
//! so the two never compete for the same corner. When the aid chips DO own
//! that corner the pill walks to trailing-top, then leading-bottom — the pill
//! is the less important element, so it moves and the chips never do. The
//! chrome funnel never hit-tests the pill, so it stays pointer-transparent
//! decoration (unlike the aid chips, which are clickable toggles).

use flowshot_core::geometry::{LogicalRect, OutputInfo};

use super::state::ChromeState;
use crate::editor::EditorState;
use crate::render::{DisplayList, Rect, Size, f32_from_f64, f32_from_i32};
use crate::widgets::StatusChip;
use flowshot_core::tokens::DesignTokens;

/// The CPU-fallback pill's label: the fact stated, kept short so the pill
/// fits an output corner at 1x.
pub(crate) const CPU_LABEL: &str = "CPU raster";

impl ChromeState {
    /// Paints the CPU-fallback pill for one output. The CALLER owns the gate
    /// ([`ChromeState::cpu_path_for`](crate::chrome::state::ChromeState::cpu_path_for)
    /// reads the slots the shell recorded at renderer init) and paints AFTER
    /// the aid chips, so the pill never competes with them for z-order.
    pub fn paint_cpu_status(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) {
        let tokens = self.tokens();
        let scale = f32_from_f64(output.scale);
        let size = StatusChip::measured_size(CPU_LABEL, tokens, scale);
        let rect = status_rect(
            size,
            output,
            tokens,
            self.aids_layout(editor, selection, output),
        );
        StatusChip {
            rect,
            label: CPU_LABEL,
            active: true,
        }
        .draw(list, tokens, scale);
    }
}

/// The pill's rect: the output's trailing-bottom corner on a medium margin,
/// falling back to trailing-top then leading-bottom when that corner
/// collides with the aid chips.
#[must_use]
pub(crate) fn status_rect(
    size: Size,
    output: &OutputInfo,
    tokens: &DesignTokens,
    chips: [Rect; 2],
) -> Rect {
    let scale = f32_from_f64(output.scale);
    let margin = tokens.spacing.medium as f32 * scale;
    let width = f32_from_i32(output.physical_size.width.0);
    let height = f32_from_i32(output.physical_size.height.0);
    let primary = Rect::from_parts(
        width - margin - size.width,
        height - margin - size.height,
        size.width,
        size.height,
    );
    if !chips.iter().any(|chip| chip.intersects(&primary)) {
        return primary;
    }
    let fallbacks = [
        Rect::from_parts(width - margin - size.width, margin, size.width, size.height),
        Rect::from_parts(
            margin,
            height - margin - size.height,
            size.width,
            size.height,
        ),
    ];
    *fallbacks
        .iter()
        .find(|candidate| !chips.iter().any(|chip| chip.intersects(candidate)))
        .unwrap_or(&primary)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "fixture construction validates a known-good fixture"
    )]

    use super::*;
    use crate::editor::EditorState;
    use crate::render::Command;
    use flowshot_core::geometry::{PhysicalSize, Transform};

    fn no_chips() -> [Rect; 2] {
        let clear = Rect::from_parts(0.0, 0.0, 1.0, 1.0);
        [clear, clear]
    }

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

    #[test]
    fn the_pill_docks_at_the_trailing_bottom_corner() {
        // Given an output and no selection, when the pill is placed, then it
        // sits in the trailing-bottom corner on a medium margin — diagonally
        // opposite the aid cluster's above-leading default.
        let tokens = DesignTokens::default();
        let size = StatusChip::measured_size(CPU_LABEL, &tokens, 1.0);
        let rect = status_rect(size, &output(), &tokens, no_chips());
        let margin = tokens.spacing.medium as f32;
        assert_eq!(rect.right(), 1920.0 - margin);
        assert_eq!(rect.bottom(), 1080.0 - margin);
    }

    #[test]
    fn the_pill_walks_to_a_free_corner_when_the_chips_own_it() {
        // Given the aid chips own the trailing-bottom corner, when the pill
        // is placed, then it moves to the trailing-top corner and touches no
        // chip.
        let tokens = DesignTokens::default();
        let size = StatusChip::measured_size(CPU_LABEL, &tokens, 1.0);
        let margin = tokens.spacing.medium as f32;
        let owner = Rect::from_parts(
            1920.0 - margin - size.width,
            1080.0 - margin - size.height,
            size.width,
            size.height,
        );
        let rect = status_rect(size, &output(), &tokens, [owner, owner]);
        assert_eq!(rect.origin.y, margin, "trailing-top corner");
        assert_eq!(rect.right(), 1920.0 - margin);
        assert!(!owner.intersects(&rect), "the pill left the owned corner");
    }

    #[test]
    fn the_pill_scales_with_the_output() {
        let tokens = DesignTokens::default();
        let small = StatusChip::measured_size(CPU_LABEL, &tokens, 1.0);
        let doubled = StatusChip::measured_size(CPU_LABEL, &tokens, 2.0);
        assert_eq!(doubled.width, small.width * 2.0);
        assert_eq!(doubled.height, small.height * 2.0);
    }

    #[test]
    fn the_pill_states_the_fact_in_one_label() {
        let editor = EditorState::default();
        let mut list = DisplayList::new();
        ChromeState::new().paint_cpu_status(&mut list, &editor, None, &output());
        let texts: Vec<String> = list
            .iter()
            .filter_map(|command| match command {
                Command::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec![CPU_LABEL], "one label, no key glyph");
        let fills = list
            .iter()
            .filter(|command| matches!(*command, Command::Fill { .. }))
            .count();
        assert_eq!(fills, 1, "the pill body only — no key plate, no wash");
    }
}

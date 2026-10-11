//! The session-status pill row: one label-only chip stating which rasterizer
//! presents the window — `GPU raster` quiet in the corner when all is well,
//! accent-tinted `CPU raster` when the session lost the GPU frame target and
//! the CPU fallback took over (adr-007). The read is always present, so the
//! degraded state is visible instead of silently rendering differently.
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

/// The healthy-path pill's label: which rasterizer presents the window.
/// Kept short so the pill fits an output corner at 1x.
pub(crate) const GPU_LABEL: &str = "GPU raster";

/// The CPU-fallback pill's label: the fact stated, kept short so the pill
/// fits an output corner at 1x.
pub(crate) const CPU_LABEL: &str = "CPU raster";

impl ChromeState {
    /// Paints the raster-path pill for one output. The CALLER owns the gate
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
        let tokens = self.tokens();
        let scale = f32_from_f64(output.scale);
        let label = if cpu_path { CPU_LABEL } else { GPU_LABEL };
        let size = StatusChip::measured_size(label, tokens, scale);
        let rect = status_rect(
            size,
            output,
            tokens,
            self.aids_layout(editor, selection, output),
        );
        StatusChip {
            rect,
            label,
            active: cpu_path,
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
    fn the_pill_names_the_path_in_both_states() {
        // Given a window on either raster path, when its frame is painted,
        // then the pill states exactly that path — `CPU raster` for the
        // fallback, `GPU raster` for the healthy path.
        let editor = EditorState::default();
        let labels = |cpu_path: bool| {
            let mut list = DisplayList::new();
            ChromeState::new().paint_render_status(&mut list, &editor, None, &output(), cpu_path);
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
        let editor = EditorState::default();
        let mut list = DisplayList::new();
        ChromeState::new().paint_render_status(&mut list, &editor, None, &output(), true);
        let fills = list
            .iter()
            .filter(|command| matches!(*command, Command::Fill { .. }))
            .count();
        assert_eq!(fills, 1, "the pill body only — no key plate, no wash");
        let texts = list
            .iter()
            .filter(|command| matches!(*command, Command::Text { .. }))
            .count();
        assert_eq!(texts, 1, "one label, no key glyph");
    }
}

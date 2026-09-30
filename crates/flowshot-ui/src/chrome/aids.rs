//! The quick-aids indicator: a chip cluster (magnifier `L`, grid `F`) at a
//! fixed output corner showing each in-session aid's state + key.
//!
//! Discoverability contract (the reported gap: the magnifier was only
//! findable by being told `L` exists):
//!
//! - SEPARATED: the cluster anchors to an output corner, never inside the
//!   selection; the placement math walks the corner candidates
//!   (bottom-left first - opposite the geometry HUD's default bottom-right)
//!   and picks the first that intersects NONE of the avoid rects
//!   (selection, toolbar, side panel). The geometry HUD lives inside the
//!   selection, so avoiding the selection rect avoids the HUD.
//! - QUIET, NOT ANNOYING: off-state chips are dimmed contrast ink, on-state
//!   chips are accent-tinted ([`AidChip`]). NO idle fade-out: a fade timer
//!   would need periodic wakes (breaking the shell's idle zero-CPU
//!   contract), and the dimmed off-state already IS the low-prominence
//!   read - documented choice per the task's "your call".
//! - POINTER-TRANSPARENT: the cluster registers no hit-test anywhere (the
//!   chrome press funnel never consults it), so clicks fall through to the
//!   selection engine untouched.
//! - LIVE: the chip states are read from the [`EditorState`] every frame,
//!   and an `L`/`F` toggle emits a redraw - the cluster follows
//!   immediately, including rebinds (the keys come from the SAME
//!   [`ToolShortcuts`](crate::editor::ToolShortcuts) slots the dispatch
//!   reads).

use flowshot_core::geometry::{LogicalRect, OutputInfo};
use flowshot_core::tokens::DesignTokens;

use crate::editor::EditorState;
use crate::editor::paint::local_rect;
use crate::render::{DisplayList, Rect, f32_from_f64};
use crate::widgets::AidChip;

use super::side_panel;
use super::state::ChromeState;
use super::tooltips::chord;

/// One aid chip's content, resolved from the live editor state.
struct AidSpec {
    key: String,
    label: &'static str,
    active: bool,
}

impl ChromeState {
    /// Paints the quick-aids cluster for one output (the frame builder
    /// calls this on the cursor-owning slot only - one cluster where the
    /// user is looking, the magnifier/crosshair slot rule).
    pub fn paint_aids(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) {
        let scale = f32_from_f64(output.scale);
        let tokens = self.tokens();
        let mut avoid = Vec::new();
        if let Some(selection) = selection {
            avoid.push(local_rect(output, selection));
            let (toolbar_rect, buttons) = self.toolbar.layout(selection, tokens, scale, output);
            if !buttons.is_empty() {
                avoid.push(toolbar_rect);
            }
            if self.panel_shown(editor) {
                avoid.push(side_panel::layout(editor, selection, tokens, scale, output).rect);
            }
        }
        draw(list, editor, tokens, scale, output, &avoid);
    }
}

/// The corner candidate order: bottom-left first (opposite the geometry
/// HUD's default bottom-right anchor), then the remaining corners.
fn cluster_rect(width: f32, height: f32, output: &OutputInfo, avoid: &[Rect], margin: f32) -> Rect {
    let bounds_w = output.physical_size.width.0 as f32;
    let bounds_h = output.physical_size.height.0 as f32;
    let candidates = [
        Rect::from_parts(margin, bounds_h - height - margin, width, height),
        Rect::from_parts(margin, margin, width, height),
        Rect::from_parts(
            bounds_w - width - margin,
            bounds_h - height - margin,
            width,
            height,
        ),
        Rect::from_parts(bounds_w - width - margin, margin, width, height),
    ];
    candidates
        .into_iter()
        .find(|candidate| !avoid.iter().any(|rect| rect.intersects(candidate)))
        .unwrap_or(candidates[0])
}

/// Lays out and draws the chip cluster (stateless: every frame re-reads
/// the editor's toggle states and key bindings).
fn draw(
    list: &mut DisplayList,
    editor: &EditorState,
    tokens: &DesignTokens,
    scale: f32,
    output: &OutputInfo,
    avoid: &[Rect],
) {
    let shortcuts = editor.shortcuts();
    let specs = [
        AidSpec {
            key: chord(shortcuts.magnifier_key()),
            label: "Magnifier",
            active: editor.magnifier_visible(),
        },
        AidSpec {
            key: chord(shortcuts.grid_key()),
            label: "Grid",
            active: editor.grid_visible(),
        },
    ];

    let gap = tokens.spacing.small as f32 * scale;
    let margin = tokens.spacing.medium as f32 * scale;
    let sizes: Vec<_> = specs
        .iter()
        .map(|spec| AidChip::measured_size(spec.label, tokens, scale))
        .collect();
    let width = sizes.iter().map(|size| size.width).sum::<f32>()
        + gap * (specs.len().saturating_sub(1)) as f32;
    let height = sizes.iter().map(|size| size.height).fold(0.0_f32, f32::max);

    let cluster = cluster_rect(width, height, output, avoid, margin);
    let mut x = cluster.origin.x;
    for (spec, size) in specs.iter().zip(sizes) {
        AidChip {
            rect: Rect::from_parts(x, cluster.origin.y, size.width, size.height),
            key: &spec.key,
            label: spec.label,
            active: spec.active,
        }
        .draw(list, tokens, scale);
        x += size.width + gap;
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp
    )]

    use flowshot_core::geometry::{LogicalRect, PhysicalSize, Transform};

    use super::*;

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

    const MARGIN: f32 = 8.0;

    #[test]
    fn default_corner_is_bottom_left() {
        let rect = cluster_rect(200.0, 25.0, &output(), &[], MARGIN);
        assert_eq!(rect.origin.x, MARGIN);
        assert_eq!(rect.bottom(), 1080.0 - MARGIN);
    }

    #[test]
    fn cluster_steps_through_corners_away_from_avoid_rects() {
        let out = output();
        // Given the bottom-left corner is blocked, the cluster rises to
        // top-left.
        let bl_blocker = Rect::from_parts(0.0, 900.0, 400.0, 180.0);
        let rect = cluster_rect(200.0, 25.0, &out, &[bl_blocker], MARGIN);
        assert_eq!((rect.origin.x, rect.origin.y), (MARGIN, MARGIN), "top-left");
        // Given both left corners are blocked, bottom-right comes next.
        let left_band = Rect::from_parts(0.0, 0.0, 300.0, 1080.0);
        let rect = cluster_rect(200.0, 25.0, &out, &[bl_blocker, left_band], MARGIN);
        assert_eq!(rect.right(), 1920.0 - MARGIN, "bottom-right");
        assert_eq!(rect.bottom(), 1080.0 - MARGIN);
        // Given every corner is blocked, the first candidate is the
        // documented fallback (the cluster never disappears).
        let everything = Rect::from_parts(0.0, 0.0, 1920.0, 1080.0);
        let rect = cluster_rect(200.0, 25.0, &out, &[everything], MARGIN);
        assert_eq!(rect.origin.x, MARGIN);
        assert_eq!(rect.bottom(), 1080.0 - MARGIN);
    }

    #[test]
    fn edge_touching_avoid_rect_does_not_displace_the_cluster() {
        // A toolbar whose left edge starts exactly at the cluster's right
        // edge abuts it - abutment is not overlap.
        let abutting = Rect::from_parts(MARGIN + 200.0, 1000.0, 100.0, 40.0);
        let rect = cluster_rect(200.0, 25.0, &output(), &[abutting], MARGIN);
        assert_eq!(rect.origin.x, MARGIN);
    }

    #[test]
    fn drawn_cluster_never_intersects_the_avoid_rects() {
        let editor = EditorState::default();
        let tokens = DesignTokens::default();
        let avoid = [
            Rect::from_parts(400.0, 900.0, 1200.0, 172.0),
            Rect::from_parts(0.0, 1040.0, 150.0, 40.0),
        ];
        let mut list = DisplayList::new();
        draw(&mut list, &editor, &tokens, 1.0, &output(), &avoid);
        // The chip fills are the cluster's geometry: every fill rect must
        // clear both avoid rects.
        let fills: Vec<Rect> = list
            .iter()
            .filter_map(|command| match command {
                crate::render::Command::Fill {
                    shape: crate::render::Shape::Rect { rect, .. },
                    ..
                } => Some(*rect),
                _ => None,
            })
            .collect();
        assert!(!fills.is_empty(), "the cluster paints chips");
        for fill in &fills {
            for blocked in &avoid {
                assert!(!fill.intersects(blocked), "chip {fill:?} hits {blocked:?}");
            }
        }
    }

    #[test]
    fn chip_keys_come_from_the_live_bindings() {
        let mut editor = EditorState::default();
        editor.shortcuts_mut().rebind_aids(
            winit::keyboard::KeyCode::KeyV,
            winit::keyboard::KeyCode::KeyH,
        );
        let mut list = DisplayList::new();
        draw(
            &mut list,
            &editor,
            &DesignTokens::default(),
            1.0,
            &output(),
            &[],
        );
        let texts: Vec<String> = list
            .iter()
            .filter_map(|command| match command {
                crate::render::Command::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"V".to_owned()), "{texts:?}");
        assert!(texts.contains(&"H".to_owned()), "{texts:?}");
        assert!(!texts.contains(&"L".to_owned()), "{texts:?}");
    }
}

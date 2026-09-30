//! The quick-aids cluster: a chip row (magnifier `L`, grid `F`) anchored
//! ADJACENT TO THE SELECTION, showing each in-session aid's state + key and
//! toggling it on click.
//!
//! Discoverability contract (the reported gaps: the aids were only findable
//! by being told the key exists, and a corner-placed pointer-transparent
//! indicator read as unnoticeable decoration):
//!
//! - ADJACENT: the cluster follows the toolbar's anchoring model - edge of
//!   the selection, flipping at the screen edges. Candidate order:
//!   above-leading first (diagonally opposite the toolbar's default
//!   below-trailing slot, so chips + toolbar + geometry HUD bracket the
//!   focus area as ONE affordance cluster), then below-leading,
//!   above-trailing, below-trailing; every candidate is clamped on-screen,
//!   so an edge flip falls out of the collision walk (a clamped slot
//!   overlaps the selection and loses). The first candidate intersecting
//!   NONE of the avoid rects (selection, toolbar, side panel - the geometry
//!   HUD lives INSIDE the selection, so the selection rect covers it) wins;
//!   a fully blocked walk (a fullscreen selection) keeps the first
//!   candidate - the cluster never disappears. With NO selection yet the
//!   cluster docks at the cursor-owning output's bottom-center edge:
//!   stable under cursor motion (a cursor-following offset would flee the
//!   click it advertises), clear of the aim area, and promoted to
//!   above-the-selection by the first drag.
//! - CLICKABLE: the chips hit-test through the chrome press funnel
//!   (`ChromeState::aids_press`, gated to the cursor-owning output - the
//!   paint rule). A left click toggles the aid through the SAME editor seam
//!   the key dispatch uses, and the press is consumed + grabbed like a
//!   toolbar press - a chip click never starts a selection drag.
//! - QUIET, NOT ANNOYING: off-state chips are dimmed contrast ink, on-state
//!   chips are accent-tinted ([`AidChip`]). Hover/press feedback is the
//!   design system's wash ramp at DISCRETE levels (1 hover, 2 press): the
//!   quiet affordance snaps instead of tweening, so a hovered chip
//!   schedules no motion wakes. NO idle fade-out: a fade timer would need
//!   periodic wakes (breaking the shell's idle zero-CPU contract), and the
//!   dimmed off-state already IS the low-prominence read.
//! - LIVE: the chip states are read from the [`EditorState`] every frame,
//!   and a toggle (key OR click) emits a redraw - the cluster follows
//!   immediately, including rebinds (the keys come from the SAME
//!   [`ToolShortcuts`](crate::editor::ToolShortcuts) slots the dispatch
//!   reads).

use flowshot_core::geometry::{LogicalPoint, LogicalRect, OutputInfo};
use winit::event::MouseButton;

use crate::editor::paint::{local_rect, local_x, local_y};
use crate::editor::{AidToggle, EditorState};
use crate::render::{DisplayList, Point, Rect, Size, f32_from_f64};
use crate::widgets::AidChip;

use super::side_panel;
use super::state::ChromeState;
use super::tooltips::chord;

/// The chip table: the toggle each chip carries and its label - ONE source
/// for the paint order, the hit-test order, and the click dispatch (the
/// chip index IS the table index).
const AIDS: [(AidToggle, &str); 2] = [
    (AidToggle::Magnifier, "Magnifier"),
    (AidToggle::Grid, "Grid"),
];

/// The cluster's discrete wash state (the widget-convention hover/press
/// ramp, snapped - no tween, no motion-timeline participation, so a
/// hovered chip schedules zero wakes).
#[derive(Debug, Default)]
pub(crate) struct AidsInput {
    /// The hovered chip index (wash level 1).
    pub(crate) hover: Option<usize>,
    /// The pressed chip index (wash level 2).
    pub(crate) press: Option<usize>,
}

impl AidsInput {
    /// Chip `index`'s wash level (2 press, 1 hover, 0 idle - the shared
    /// [`wash_alpha`](crate::widgets::wash_alpha) ramp).
    #[must_use]
    pub(crate) fn wash(&self, index: usize) -> f64 {
        if self.press == Some(index) {
            2.0
        } else if self.hover == Some(index) {
            1.0
        } else {
            0.0
        }
    }
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
        let shortcuts = editor.shortcuts();
        let chips = self.aids_layout(editor, selection, output);
        for (index, ((aid, label), rect)) in AIDS.iter().zip(chips).enumerate() {
            let (key_code, active) = match aid {
                AidToggle::Magnifier => (shortcuts.magnifier_key(), editor.magnifier_visible()),
                AidToggle::Grid => (shortcuts.grid_key(), editor.grid_visible()),
            };
            let key = chord(key_code);
            AidChip {
                rect,
                key: &key,
                label,
                active,
                wash: self.aids.wash(index),
            }
            .draw(list, tokens, scale);
        }
    }

    /// The chip rects in [`AIDS`] order (output-local physical px). The
    /// paint path and the input path share this, so hit geometry can never
    /// disagree with the drawn chips (the toolbar's `layout`/`button_at`
    /// contract).
    #[must_use]
    pub(crate) fn aids_layout(
        &self,
        editor: &EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) -> [Rect; 2] {
        let tokens = self.tokens();
        let scale = f32_from_f64(output.scale);
        let sizes = AIDS.map(|(_, label)| AidChip::measured_size(label, tokens, scale));
        let gap = tokens.spacing.small as f32 * scale;
        let margin = tokens.spacing.medium as f32 * scale;
        let cluster = Size::new(
            sizes.iter().map(|size| size.width).sum::<f32>()
                + gap * AIDS.len().saturating_sub(1) as f32,
            sizes.iter().map(|size| size.height).fold(0.0_f32, f32::max),
        );
        let avoid = self.aids_avoid(editor, selection, output);
        let local_sel = selection.map(|selection| local_rect(output, selection));
        let placement = cluster_rect(cluster, output, &avoid, margin, local_sel);
        let mut x = placement.origin.x;
        sizes.map(|size| {
            let rect = Rect::from_parts(x, placement.origin.y, size.width, size.height);
            x += size.width + gap;
            rect
        })
    }

    /// The placement avoid set: the selection (the geometry HUD lives
    /// inside it), the toolbar plate, and the side panel when shown.
    #[must_use]
    fn aids_avoid(
        &self,
        editor: &EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) -> Vec<Rect> {
        let Some(selection) = selection else {
            return Vec::new();
        };
        let tokens = self.tokens();
        let scale = f32_from_f64(output.scale);
        let mut avoid = vec![local_rect(output, selection)];
        let (toolbar_rect, buttons) = self.toolbar.layout(selection, tokens, scale, output);
        if !buttons.is_empty() {
            avoid.push(toolbar_rect);
        }
        if self.panel_shown(editor) {
            avoid.push(side_panel::layout(editor, selection, tokens, scale, output).rect);
        }
        avoid
    }

    /// A press on the chip cluster: `true` when a chip consumed it - the
    /// funnel then skips the F27 editor chain and the selection engine (a
    /// chip click never starts a selection drag) and the release belongs to
    /// the chrome (the grab). A left click toggles the aid through the same
    /// editor seam the key dispatch uses. A visible color wheel wins: this
    /// defers to [`ChromeState::press`], whose F27 P1 branch consumes
    /// EVERY press while the picker is open.
    pub(crate) fn aids_press(
        &mut self,
        button: MouseButton,
        at: LogicalPoint,
        editor: &mut EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) -> bool {
        if self.color_wheel.visible {
            return false;
        }
        let Some(index) = self.chip_index_at(at, editor, selection, output) else {
            return false;
        };
        if button == MouseButton::Left {
            match AIDS[index].0 {
                AidToggle::Magnifier => editor.toggle_magnifier(),
                AidToggle::Grid => editor.toggle_grid(),
            }
            self.aids.press = Some(index);
        }
        self.grabbed = true;
        true
    }

    /// The funnel's motion seam: retargets the chip hover wash from the
    /// pointer position (every motion event, editor-consumed or not - the
    /// toolbar hover rule).
    pub(crate) fn aids_hover(
        &mut self,
        at: LogicalPoint,
        editor: &EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) {
        let index = self.chip_index_at(at, editor, selection, output);
        self.aids.hover = index;
    }

    /// The chip under a global logical point (`None` off every chip): the
    /// hover wash and the press funnel share it with the paint path's
    /// layout.
    #[must_use]
    fn chip_index_at(
        &self,
        at: LogicalPoint,
        editor: &EditorState,
        selection: Option<LogicalRect>,
        output: &OutputInfo,
    ) -> Option<usize> {
        let local = Point::new(local_x(output, at.x.0), local_y(output, at.y.0));
        self.aids_layout(editor, selection, output)
            .iter()
            .position(|rect| rect.contains(local))
    }
}

/// The cluster placement (the module header's ADJACENT contract): the
/// selection-adjacent candidate walk, or the bottom-center dock when no
/// selection exists yet.
fn cluster_rect(
    size: Size,
    output: &OutputInfo,
    avoid: &[Rect],
    margin: f32,
    selection: Option<Rect>,
) -> Rect {
    let bounds = Size::new(
        output.physical_size.width.0 as f32,
        output.physical_size.height.0 as f32,
    );
    let x_max = (bounds.width - size.width - margin).max(margin);
    let y_max = (bounds.height - size.height - margin).max(margin);
    let Some(sel) = selection else {
        return Rect::from_parts(
            ((bounds.width - size.width) / 2.0).clamp(margin, x_max),
            y_max,
            size.width,
            size.height,
        );
    };
    let leading = sel.origin.x.clamp(margin, x_max);
    let trailing = (sel.right() - size.width).clamp(margin, x_max);
    let above = (sel.origin.y - size.height - margin).clamp(margin, y_max);
    let below = (sel.bottom() + margin).clamp(margin, y_max);
    let candidates = [
        Rect::from_parts(leading, above, size.width, size.height),
        Rect::from_parts(leading, below, size.width, size.height),
        Rect::from_parts(trailing, above, size.width, size.height),
        Rect::from_parts(trailing, below, size.width, size.height),
    ];
    candidates
        .into_iter()
        .find(|candidate| !avoid.iter().any(|rect| rect.intersects(candidate)))
        .unwrap_or(candidates[0])
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp
    )]

    use flowshot_core::geometry::{PhysicalSize, Transform};
    use winit::keyboard::KeyCode;

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
    const SIZE: Size = Size {
        width: 200.0,
        height: 25.0,
    };
    /// A mid-screen selection: every adjacent slot fits on-screen.
    const MID_SEL: Rect = Rect {
        origin: Point { x: 400.0, y: 300.0 },
        size: Size {
            width: 800.0,
            height: 450.0,
        },
    };

    fn place(avoid: &[Rect], selection: Option<Rect>) -> Rect {
        cluster_rect(SIZE, &output(), avoid, MARGIN, selection)
    }

    #[test]
    fn default_slot_is_above_the_selection_leading_edge() {
        // Given a mid-screen selection and nothing to avoid, when the
        // cluster is placed, then it sits above the selection, aligned to
        // its leading edge, one margin clear.
        let rect = place(&[], Some(MID_SEL));
        assert_eq!(rect.origin.x, 400.0);
        assert_eq!(rect.bottom(), 300.0 - MARGIN);
    }

    #[test]
    fn flips_below_when_above_runs_off_screen() {
        // Given the selection touches the top edge, the clamped above slot
        // overlaps it and loses the walk - the cluster flips below.
        let sel = Rect::from_parts(400.0, 0.0, 800.0, 450.0);
        let rect = place(&[sel], Some(sel));
        assert_eq!(rect.origin.y, 450.0 + MARGIN);
        assert_eq!(rect.origin.x, 400.0);
    }

    #[test]
    fn bottom_edge_selection_keeps_the_cluster_above() {
        // Given the selection touches the bottom edge, the clamped below
        // slot overlaps it and loses - above (the first candidate) wins.
        let sel = Rect::from_parts(400.0, 980.0, 800.0, 100.0);
        let rect = place(&[sel], Some(sel));
        assert_eq!(rect.bottom(), 980.0 - MARGIN);
    }

    #[test]
    fn walk_steps_past_blocked_slots_and_never_disappears() {
        let sel = MID_SEL;
        // Given the above-leading slot is blocked, below-leading wins.
        let above_blocker = Rect::from_parts(380.0, 250.0, 240.0, 40.0);
        let rect = place(&[above_blocker], Some(sel));
        assert_eq!(rect.origin.y, sel.bottom() + MARGIN, "below-leading");
        // Given both leading slots are blocked, above-trailing comes next.
        let below_blocker = Rect::from_parts(380.0, 750.0, 240.0, 40.0);
        let rect = place(&[above_blocker, below_blocker], Some(sel));
        assert_eq!(rect.right(), sel.right(), "above-trailing");
        assert_eq!(rect.bottom(), sel.origin.y - MARGIN);
        // Given every candidate is blocked (a fullscreen selection), the
        // first candidate is the documented fallback - never disappears.
        let everything = Rect::from_parts(0.0, 0.0, 1920.0, 1080.0);
        let rect = place(&[everything], Some(sel));
        assert_eq!(rect.origin.x, sel.origin.x);
        assert_eq!(rect.bottom(), sel.origin.y - MARGIN);
    }

    #[test]
    fn no_selection_docks_bottom_center() {
        // Given no selection yet, the cluster docks at the output's
        // bottom-center edge (the documented fallback).
        let rect = place(&[], None);
        assert_eq!(rect.origin.x, (1920.0 - SIZE.width) / 2.0);
        assert_eq!(rect.bottom(), 1080.0 - MARGIN);
    }

    #[test]
    fn edge_touching_avoid_rect_does_not_displace_the_cluster() {
        // A rect whose left edge starts exactly at the cluster's right edge
        // abuts it - abutment is not overlap.
        let abutting = Rect::from_parts(400.0 + SIZE.width, 100.0, 100.0, 40.0);
        let rect = place(&[abutting], Some(MID_SEL));
        assert_eq!(rect.origin.x, 400.0);
        assert_eq!(rect.bottom(), 300.0 - MARGIN);
    }

    #[test]
    fn layout_clears_selection_toolbar_and_panel() {
        // Given a selection with the toolbar below and the panel shown,
        // when the cluster is laid out, then no chip intersects any avoid
        // rect (the selection covers the geometry HUD - it lives inside).
        let mut chrome = ChromeState::new();
        let editor = EditorState::default();
        assert!(chrome.toggle_panel(&editor), "panel gate is on by default");
        let selection = LogicalRect::from_raw(500.0, 300.0, 700.0, 400.0);
        let out = output();
        let avoid = chrome.aids_avoid(&editor, Some(selection), &out);
        assert_eq!(avoid.len(), 3, "selection + toolbar + panel");
        for chip in &chrome.aids_layout(&editor, Some(selection), &out) {
            for blocked in &avoid {
                assert!(!chip.intersects(blocked), "chip {chip:?} hits {blocked:?}");
            }
        }
    }

    #[test]
    fn near_top_selection_flips_the_layout_below() {
        // Given a wide selection touching the top edge (above clamps into
        // it), the laid-out chips land below the selection, clear of the
        // toolbar's below-trailing plate.
        let chrome = ChromeState::new();
        let editor = EditorState::default();
        let selection = LogicalRect::from_raw(300.0, 10.0, 1000.0, 400.0);
        let out = output();
        let chips = chrome.aids_layout(&editor, Some(selection), &out);
        for chip in &chips {
            assert!(chip.origin.y > 410.0, "chip {chip:?} flipped below");
            for blocked in &chrome.aids_avoid(&editor, Some(selection), &out) {
                assert!(!chip.intersects(blocked), "chip {chip:?} hits {blocked:?}");
            }
        }
    }

    #[test]
    fn painted_chips_are_the_hit_test_geometry() {
        // Given a painted cluster, the chip backgrounds appear verbatim in
        // the display list AND each chip's center hit-tests back to its own
        // index - paint geometry == hit geometry.
        let chrome = ChromeState::new();
        let editor = EditorState::default();
        let selection = Some(LogicalRect::from_raw(500.0, 300.0, 700.0, 400.0));
        let out = output();
        let chips = chrome.aids_layout(&editor, selection, &out);
        let mut list = DisplayList::new();
        chrome.paint_aids(&mut list, &editor, selection, &out);
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
        for (index, chip) in chips.iter().enumerate() {
            assert!(fills.contains(chip), "chip bg {chip:?} painted verbatim");
            let center = chip.center();
            let at = LogicalPoint::from_raw(f64::from(center.x), f64::from(center.y));
            assert_eq!(
                chrome.chip_index_at(at, &editor, selection, &out),
                Some(index)
            );
        }
    }

    #[test]
    fn hover_tracks_the_pointer_on_and_off_the_chips() {
        let mut chrome = ChromeState::new();
        let editor = EditorState::default();
        let selection = Some(LogicalRect::from_raw(500.0, 300.0, 700.0, 400.0));
        let out = output();
        let center = chrome.aids_layout(&editor, selection, &out)[0].center();
        let over = LogicalPoint::from_raw(f64::from(center.x), f64::from(center.y));
        chrome.aids_hover(over, &editor, selection, &out);
        assert_eq!(chrome.aids.hover, Some(0));
        assert_eq!(chrome.aids.wash(0), 1.0);
        assert_eq!(chrome.aids.wash(1), 0.0);
        chrome.aids_hover(LogicalPoint::from_raw(5.0, 5.0), &editor, selection, &out);
        assert_eq!(chrome.aids.hover, None);
    }

    #[test]
    fn chip_keys_come_from_the_live_bindings() {
        let chrome = ChromeState::new();
        let mut editor = EditorState::default();
        editor
            .shortcuts_mut()
            .rebind_aids(KeyCode::KeyV, KeyCode::KeyH);
        let mut list = DisplayList::new();
        chrome.paint_aids(&mut list, &editor, None, &output());
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

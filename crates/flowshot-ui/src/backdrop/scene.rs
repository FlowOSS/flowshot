//! Per-window backdrop scene math (plan todo 15).
//!
//! The unified backdrop is LOGICAL: one [`OutputLayout`] positions every
//! output's frozen frame in global space, and each monitor window renders
//! ITS OWN physical crop 1:1 - the physical-first rule (#4871 fix: never
//! rescale, never blend an averaged scale across outputs). A single stitched
//! GPU texture would either resample `HiDPI` content or exceed texture limits
//! on wide multi-monitor spans, so stitching lives in the placement algebra
//! here, not in one atlas.
//!
//! All math is pure (no GPU): global logical selection rects become
//! window-local physical dim cutouts by converting EDGES with THAT output's
//! scale (the [`OutputInfo::physical_crop`] discipline), and the cursor
//! sprite lands at `position * scale - hotspot` on the window whose output
//! contains it.

use flowshot_core::geometry::{
    LogicalPoint, LogicalRect, OutputInfo, OutputLayout, PhysicalPoint, ToPhysical,
};

use crate::render::{Color, DisplayList, Rect, TextureId, f32_from_i32, f32_from_u32};

/// The 1:1 crop of an output texture visible in one window.
///
/// `src` (texture pixels) and `dst` (window pixels) always have the SAME
/// extent - the frozen frame is never rescaled. When the surface is larger
/// than the buffer, `needs_placeholder` marks the uncovered margin for the
/// letterbox fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WindowCrop {
    pub src: Rect,
    pub dst: Rect,
    pub needs_placeholder: bool,
}

/// Computes the 1:1 top-left-aligned crop of `buffer` visible in `surface`.
#[must_use]
pub(super) fn window_crop(buffer: (u32, u32), surface: (u32, u32)) -> WindowCrop {
    let visible_width = f32_from_u32(buffer.0.min(surface.0));
    let visible_height = f32_from_u32(buffer.1.min(surface.1));
    let rect = Rect::from_parts(0.0, 0.0, visible_width, visible_height);
    WindowCrop {
        src: rect,
        dst: rect,
        needs_placeholder: surface.0 > buffer.0 || surface.1 > buffer.1,
    }
}

/// Converts a global logical rect into window-local physical pixels for
/// `output`, clamped to the output's bounds (the dim-cutout algebra).
///
/// Edges convert with THIS output's scale only; `None` when the rect does
/// not intersect the output or rounds to zero pixels.
#[must_use]
pub(super) fn local_physical_rect(output: &OutputInfo, global: LogicalRect) -> Option<Rect> {
    let intersection = output.logical_rect.intersection(&global)?;
    let origin = output.logical_rect.origin();
    let scale = output.scale;
    let x0 = (intersection.x - origin.x).to_physical(scale).0;
    let y0 = (intersection.y - origin.y).to_physical(scale).0;
    let x1 = (intersection.right() - origin.x).to_physical(scale).0;
    let y1 = (intersection.bottom() - origin.y).to_physical(scale).0;
    (x1 > x0 && y1 > y0).then(|| {
        Rect::from_parts(
            f32_from_i32(x0),
            f32_from_i32(y0),
            f32_from_i32(x1 - x0),
            f32_from_i32(y1 - y0),
        )
    })
}

/// The cursor sprite's resolved placement: which output owns it and where
/// its top-left corner lands in that window's local physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResolvedCursor {
    pub output_index: usize,
    pub top_left: PhysicalPoint,
}

/// Resolves a global logical cursor position (todo 12's
/// `resolve_cursor_pos` contract) into the owning output and the sprite's
/// top-left corner after the hotspot offset. `None` when the position falls
/// outside every output.
#[must_use]
pub(super) fn resolve_cursor(
    layout: &OutputLayout,
    position: LogicalPoint,
    hotspot: PhysicalPoint,
) -> Option<ResolvedCursor> {
    let output_index = layout
        .outputs
        .iter()
        .position(|output| output.logical_rect.contains_point(position))?;
    let output = layout.outputs.get(output_index)?;
    let origin = output.logical_rect.origin();
    let local = LogicalPoint::from_raw(position.x.0 - origin.x.0, position.y.0 - origin.y.0)
        .to_physical(output.scale);
    Some(ResolvedCursor {
        output_index,
        top_left: PhysicalPoint::from_raw(local.x.0 - hotspot.x.0, local.y.0 - hotspot.y.0),
    })
}

/// Everything one window's backdrop frame needs, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WindowScene {
    pub surface: (u32, u32),
    pub texture: TextureId,
    pub crop: Option<WindowCrop>,
    pub cursor: Option<Rect>,
    pub dim: Option<Color>,
    pub cutout: Option<Rect>,
    pub placeholder: Color,
}

/// Appends one window's backdrop commands to `list`, in paint order:
/// letterbox placeholder (uncovered margins only), frozen frame 1:1, cursor
/// sprite (part of the frozen scene, so BELOW the dim), then the dim layer
/// with the selection cutout (even-odd, todo 14(d)).
pub(super) fn push_window_scene(list: &mut DisplayList, scene: &WindowScene) {
    let window = Rect::from_parts(
        0.0,
        0.0,
        f32_from_u32(scene.surface.0),
        f32_from_u32(scene.surface.1),
    );
    if scene.crop.is_none_or(|crop| crop.needs_placeholder) {
        list.fill(
            crate::render::Shape::Rect {
                rect: window,
                radius: 0.0,
            },
            scene.placeholder,
        );
    }
    if let Some(crop) = scene.crop {
        list.image(scene.texture, crop.dst, Some(crop.src));
    }
    if let Some(dst) = scene.cursor {
        list.image(super::cursor_texture_id(), dst, None);
    }
    if let Some(color) = scene.dim {
        list.dim(window, scene.cutout.into_iter().collect(), color);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use flowshot_core::geometry::{Logical, PhysicalSize, Transform};

    use super::*;

    fn output(
        connector: &str,
        logical: LogicalRect,
        physical: (i32, i32),
        scale: f64,
    ) -> OutputInfo {
        OutputInfo::new(
            connector,
            connector,
            logical,
            PhysicalSize::from_raw(physical.0, physical.1),
            scale,
            Transform::Normal,
        )
        .expect("valid fixture output")
    }

    /// HDMI-A-1 1920x1080 @ 1x at (0,0); DP-3 2560x1440 logical 1280x720
    /// @ 2x at (1920,0) - mixed scales, never averaged.
    fn dual_layout() -> OutputLayout {
        OutputLayout::new(vec![
            output(
                "HDMI-A-1",
                LogicalRect::from_raw(0.0, 0.0, 1920.0, 1080.0),
                (1920, 1080),
                1.0,
            ),
            output(
                "DP-3",
                LogicalRect::from_raw(1920.0, 0.0, 1280.0, 720.0),
                (2560, 1440),
                2.0,
            ),
        ])
    }

    #[test]
    fn equal_surface_and_buffer_cover_the_window_exactly() {
        let crop = window_crop((2560, 1440), (2560, 1440));
        assert_eq!(crop.src, Rect::from_parts(0.0, 0.0, 2560.0, 1440.0));
        assert_eq!(crop.dst, crop.src);
        assert!(!crop.needs_placeholder);
    }

    #[test]
    fn smaller_surface_crops_one_to_one_without_rescale() {
        // Surface 1920x1080 over a 2560x1440 buffer: src extent == dst
        // extent (1:1), never a rescaled full-buffer squeeze (#4871).
        let crop = window_crop((2560, 1440), (1920, 1080));
        assert_eq!(crop.src.size.width, crop.dst.size.width);
        assert_eq!(crop.src.size.height, crop.dst.size.height);
        assert_eq!(crop.dst, Rect::from_parts(0.0, 0.0, 1920.0, 1080.0));
        assert!(!crop.needs_placeholder);
    }

    #[test]
    fn larger_surface_flags_the_letterbox_placeholder() {
        let crop = window_crop((800, 600), (1024, 768));
        assert!(crop.needs_placeholder);
        assert_eq!(crop.dst, Rect::from_parts(0.0, 0.0, 800.0, 600.0));
    }

    #[test]
    fn cutout_converts_edges_with_the_owning_outputs_scale() {
        let layout = dual_layout();
        let dp3 = layout.outputs.get(1).expect("DP-3");
        // Selection inside DP-3: logical (1970, 50) size 100x50 -> physical
        // offset (100, 100) size 200x100 at scale 2.
        let selection = LogicalRect::new(
            Logical(1970.0),
            Logical(50.0),
            Logical(100.0),
            Logical(50.0),
        );
        let cutout = local_physical_rect(dp3, selection).expect("intersecting");
        assert_eq!(cutout, Rect::from_parts(100.0, 100.0, 200.0, 100.0));
    }

    #[test]
    fn spanning_selection_clamps_per_window() {
        let layout = dual_layout();
        // Selection spanning the boundary at logical x=1920.
        let selection = LogicalRect::from_raw(1900.0, 100.0, 100.0, 100.0);
        let hdmi = local_physical_rect(layout.outputs.first().expect("HDMI"), selection)
            .expect("left part");
        assert_eq!(hdmi, Rect::from_parts(1900.0, 100.0, 20.0, 100.0));
        let dp3 = local_physical_rect(layout.outputs.get(1).expect("DP-3"), selection)
            .expect("right part");
        // The 80 logical px inside DP-3 become 160 physical px at scale 2.
        assert_eq!(dp3, Rect::from_parts(0.0, 200.0, 160.0, 200.0));
    }

    #[test]
    fn selection_outside_an_output_yields_no_cutout() {
        let layout = dual_layout();
        let selection = LogicalRect::from_raw(10.0, 10.0, 50.0, 50.0);
        assert!(local_physical_rect(layout.outputs.get(1).expect("DP-3"), selection).is_none());
    }

    #[test]
    fn cursor_resolves_to_the_owning_output_with_hotspot_offset() {
        let layout = dual_layout();
        // Global logical (2020, 200) is inside DP-3 (origin 1920,0 scale 2):
        // local physical (200, 400); hotspot (3, 1) -> top-left (197, 399).
        let resolved = resolve_cursor(
            &layout,
            LogicalPoint::from_raw(2020.0, 200.0),
            PhysicalPoint::from_raw(3, 1),
        )
        .expect("inside layout");
        assert_eq!(resolved.output_index, 1);
        assert_eq!(resolved.top_left, PhysicalPoint::from_raw(197, 399));
    }

    #[test]
    fn cursor_on_the_first_output_uses_its_scale() {
        let layout = dual_layout();
        let resolved = resolve_cursor(
            &layout,
            LogicalPoint::from_raw(100.0, 100.0),
            PhysicalPoint::zero(),
        )
        .expect("inside layout");
        assert_eq!(resolved.output_index, 0);
        assert_eq!(resolved.top_left, PhysicalPoint::from_raw(100, 100));
    }

    #[test]
    fn cursor_outside_the_layout_is_unresolved() {
        let layout = dual_layout();
        assert!(
            resolve_cursor(
                &layout,
                LogicalPoint::from_raw(9000.0, 9000.0),
                PhysicalPoint::zero(),
            )
            .is_none()
        );
    }

    #[test]
    fn scene_paint_order_is_placeholder_frame_cursor_dim() {
        use crate::render::{Command, Shape};

        let mut list = DisplayList::new();
        let scene = WindowScene {
            surface: (100, 50),
            texture: TextureId::new(7),
            crop: Some(WindowCrop {
                src: Rect::from_parts(0.0, 0.0, 100.0, 50.0),
                dst: Rect::from_parts(0.0, 0.0, 100.0, 50.0),
                needs_placeholder: true,
            }),
            cursor: Some(Rect::from_parts(10.0, 10.0, 24.0, 24.0)),
            dim: Some(Color::from_rgba8(0, 0, 0, 128)),
            cutout: Some(Rect::from_parts(5.0, 5.0, 20.0, 20.0)),
            placeholder: Color::from_rgba8(1, 1, 1, 255),
        };
        push_window_scene(&mut list, &scene);
        let commands: Vec<_> = list.iter().collect();
        assert!(matches!(
            commands[0],
            Command::Fill {
                shape: Shape::Rect { .. },
                ..
            }
        ));
        assert!(matches!(commands[1], Command::Image(_)));
        assert!(matches!(commands[2], Command::Image(_)));
        let Command::Dim { cutouts, .. } = commands[3] else {
            panic!("fourth command is the dim layer");
        };
        assert_eq!(cutouts.len(), 1);
    }

    #[test]
    fn scene_without_dim_or_cursor_emits_only_the_frame() {
        let mut list = DisplayList::new();
        let scene = WindowScene {
            surface: (100, 50),
            texture: TextureId::new(7),
            crop: Some(WindowCrop {
                src: Rect::from_parts(0.0, 0.0, 100.0, 50.0),
                dst: Rect::from_parts(0.0, 0.0, 100.0, 50.0),
                needs_placeholder: false,
            }),
            cursor: None,
            dim: None,
            cutout: None,
            placeholder: Color::from_rgba8(1, 1, 1, 255),
        };
        push_window_scene(&mut list, &scene);
        assert_eq!(list.len(), 1);
        assert!(matches!(
            list.iter().next(),
            Some(&crate::render::Command::Image(_))
        ));
    }

    #[test]
    fn missing_frame_window_is_a_full_placeholder() {
        let mut list = DisplayList::new();
        let scene = WindowScene {
            surface: (100, 50),
            texture: TextureId::new(7),
            crop: None,
            cursor: None,
            dim: None,
            cutout: None,
            placeholder: Color::from_rgba8(26, 26, 46, 255),
        };
        push_window_scene(&mut list, &scene);
        assert_eq!(list.len(), 1);
        assert!(matches!(
            list.iter().next(),
            Some(&crate::render::Command::Fill { .. })
        ));
    }
}

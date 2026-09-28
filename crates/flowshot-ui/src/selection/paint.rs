//! Per-window selection painting (plan todo 16): outline, grips, HUD.
//!
//! The selection lives in GLOBAL LOGICAL space; each monitor window derives
//! its own physical-px commands from the same geometry (the #4894 spanning
//! model - a selection crossing the monitor boundary is drawn by BOTH
//! windows, each rendering its visible portion; the GPU viewport clips).
//! Edge conversion rounds with the output's own scale (the
//! `local_physical_rect` discipline of the backdrop - never an averaged
//! factor). Unlike the dim cutout, the outline/grip/HUD geometry is NOT
//! intersected with the output bounds: strokes and glyphs clip naturally,
//! and intersecting would cut the outline at the boundary instead of
//! letting it run seamlessly across windows.
//!
//! All colors come from the design tokens (accent for outline+grips, accent
//! at the Flameshot HUD alpha for the box, black/white text by luminance -
//! `ColorUtils::colorIsDark` parity); all sizes from [`SelectionMetrics`].

use flowshot_core::geometry::{Logical, LogicalPoint, LogicalRect, OutputInfo, ToPhysical};

use crate::render::{
    Color, DisplayList, Point, Rect, Shape, Size, TextCommand, f32_from_f64, f32_from_i32,
};

use super::hit::Handle;
use super::hud::HudView;
use super::metrics::{HUD_BACKGROUND_ALPHA, OUTLINE_WIDTH, SelectionMetrics};

/// The token-derived colors of the selection visuals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SelectionColors {
    /// Outline + grips (palette accent).
    pub accent: Color,
    /// HUD box background (accent at the Flameshot HUD alpha).
    pub hud_background: Color,
    /// HUD text (black or white by accent luminance).
    pub hud_text: Color,
}

impl SelectionColors {
    /// Derives the HUD colors from the (already parsed) accent token color.
    pub(super) fn from_accent(accent: Color) -> Self {
        Self {
            accent,
            hud_background: accent.with_alpha8(HUD_BACKGROUND_ALPHA),
            hud_text: accent.readable_ink(),
        }
    }
}

/// Everything one window's selection paint needs.
#[derive(Debug, Clone, Copy)]
pub(super) struct SelectionPaint<'a> {
    /// The output this window covers (origin + scale for the conversion).
    pub output: &'a OutputInfo,
    /// The selection in global logical space.
    pub rect: LogicalRect,
    /// The HUD content, when visible.
    pub hud: Option<&'a HudView>,
    /// Token-derived sizes.
    pub metrics: &'a SelectionMetrics,
    /// Token-derived colors.
    pub colors: &'a SelectionColors,
    /// HUD text font family (typography token).
    pub font_family: Option<&'a str>,
    /// HUD box corner radius (radii token, logical px).
    pub hud_radius: f64,
    /// Per-grip radius multiplier at the paint instant (`Handle::index`
    /// order; the todo-41 hover-grow, 1.0 = resting).
    pub grip_scales: [f64; 8],
}

/// Appends the selection visuals to `list` in paint order: outline, grips,
/// HUD box, HUD text.
pub(super) fn append(list: &mut DisplayList, paint: &SelectionPaint<'_>) {
    let scale = paint.output.scale;
    list.stroke(
        Shape::Rect {
            rect: local_rect(paint.output, paint.rect),
            radius: 0.0,
        },
        local_len(OUTLINE_WIDTH, scale),
        paint.colors.accent,
    );
    let grip_radius = local_len(paint.metrics.grip / 2.0, scale);
    for handle in Handle::ALL {
        let grown = grip_radius * f32_from_f64(paint.grip_scales[handle.index()]);
        list.fill(
            Shape::Ellipse {
                center: local_point(paint.output, handle.anchor(paint.rect)),
                radii: Size::new(grown, grown),
            },
            paint.colors.accent,
        );
    }
    if let Some(hud) = paint.hud {
        append_hud(list, paint, hud);
    }
}

fn append_hud(list: &mut DisplayList, paint: &SelectionPaint<'_>, hud: &HudView) {
    let scale = paint.output.scale;
    let box_rect = local_rect(paint.output, hud.box_rect);
    list.fill(
        Shape::Rect {
            rect: box_rect,
            radius: local_len(paint.hud_radius, scale),
        },
        paint.colors.hud_background,
    );
    list.text(TextCommand {
        position: local_point(paint.output, hud.text_origin),
        text: hud.text.clone(),
        font_size: local_len(paint.metrics.font_size, scale),
        line_height: local_len(paint.metrics.font_line_spacing, scale),
        color: paint.colors.hud_text,
        family: paint.font_family.map(str::to_owned),
        max_width: None,
    });
}

/// Global logical rect -> window-local physical px (edges rounded with THIS
/// output's scale; deliberately unclamped - see the module header).
fn local_rect(output: &OutputInfo, global: LogicalRect) -> Rect {
    let x0 = local_x(output, global.x.0);
    let y0 = local_y(output, global.y.0);
    let x1 = local_x(output, global.right().0);
    let y1 = local_y(output, global.bottom().0);
    Rect::from_parts(x0, y0, x1 - x0, y1 - y0)
}

fn local_point(output: &OutputInfo, global: LogicalPoint) -> Point {
    Point::new(local_x(output, global.x.0), local_y(output, global.y.0))
}

fn local_x(output: &OutputInfo, global: f64) -> f32 {
    let offset = Logical(global - output.logical_rect.x.0);
    f32_from_i32(offset.to_physical(output.scale).0)
}

fn local_y(output: &OutputInfo, global: f64) -> f32 {
    let offset = Logical(global - output.logical_rect.y.0);
    f32_from_i32(offset.to_physical(output.scale).0)
}

/// A logical LENGTH (not a coordinate) as physical px; a non-finite scale
/// falls back to 1.0 (the geometry crate's totality rule).
fn local_len(logical: f64, scale: f64) -> f32 {
    let factor = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    f32_from_f64(logical * factor)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic
    )]

    use flowshot_core::geometry::{PhysicalSize, Transform};
    use flowshot_core::tokens::DesignTokens;

    use crate::render::Command;
    use crate::selection::hud::{HudPosition, hud_view};

    use super::*;

    fn output(origin_x: f64, scale: f64) -> OutputInfo {
        OutputInfo::new(
            "DP-X",
            "DP-X",
            LogicalRect::from_raw(origin_x, 0.0, 1920.0, 1080.0),
            PhysicalSize::from_raw(1920, 1080),
            scale,
            Transform::Normal,
        )
        .expect("valid fixture output")
    }

    fn paint<'a>(
        output: &'a OutputInfo,
        rect: LogicalRect,
        metrics: &'a SelectionMetrics,
        colors: &'a SelectionColors,
    ) -> SelectionPaint<'a> {
        SelectionPaint {
            output,
            rect,
            hud: None,
            metrics,
            colors,
            font_family: None,
            hud_radius: 2.0,
            grip_scales: [1.0; 8],
        }
    }

    #[test]
    fn outline_and_eight_grips_paint_in_order() {
        let tokens = DesignTokens::default();
        let metrics = SelectionMetrics::from_tokens(&tokens);
        let colors = SelectionColors::from_accent(Color::from_rgba8(42, 161, 152, 255));
        let out = output(0.0, 1.0);
        let mut list = DisplayList::new();
        append(
            &mut list,
            &paint(
                &out,
                LogicalRect::from_raw(100.0, 100.0, 200.0, 150.0),
                &metrics,
                &colors,
            ),
        );
        let commands: Vec<_> = list.iter().collect();
        assert_eq!(commands.len(), 9, "outline + 8 grips");
        assert!(matches!(commands[0], Command::Stroke { .. }));
        assert!(commands[1..].iter().all(|command| matches!(
            command,
            Command::Fill {
                shape: Shape::Ellipse { .. },
                ..
            }
        )));
    }

    #[test]
    fn geometry_converts_with_the_outputs_own_scale_and_origin() {
        let tokens = DesignTokens::default();
        let metrics = SelectionMetrics::from_tokens(&tokens);
        let colors = SelectionColors::from_accent(Color::from_rgba8(42, 161, 152, 255));
        // Second output at logical x=1920, scale 2: the selection's left
        // edge at global 2020 is local physical (2020-1920)*2 = 200.
        let out = output(1920.0, 2.0);
        let mut list = DisplayList::new();
        append(
            &mut list,
            &paint(
                &out,
                LogicalRect::from_raw(2020.0, 50.0, 100.0, 50.0),
                &metrics,
                &colors,
            ),
        );
        let Command::Stroke {
            shape: Shape::Rect { rect, .. },
            width,
            ..
        } = list.iter().next().expect("outline")
        else {
            panic!("outline stroke");
        };
        assert_eq!(rect.origin, Point::new(200.0, 100.0));
        assert_eq!(rect.size, Size::new(200.0, 100.0));
        assert_eq!(*width, 2.0, "1 logical px outline at scale 2");
    }

    #[test]
    fn spanning_selection_paints_unclamped_in_both_windows() {
        let tokens = DesignTokens::default();
        let metrics = SelectionMetrics::from_tokens(&tokens);
        let colors = SelectionColors::from_accent(Color::from_rgba8(42, 161, 152, 255));
        let selection = LogicalRect::from_raw(1820.0, 100.0, 200.0, 100.0);
        // Window A (origin 0): the outline runs past its right edge.
        let left = output(0.0, 1.0);
        let mut list = DisplayList::new();
        append(&mut list, &paint(&left, selection, &metrics, &colors));
        let Command::Stroke {
            shape: Shape::Rect { rect, .. },
            ..
        } = list.iter().next().expect("outline")
        else {
            panic!("outline stroke");
        };
        assert_eq!(*rect, Rect::from_parts(1820.0, 100.0, 200.0, 100.0));
        // Window B (origin 1920): the same rect starts at local -100.
        let right = output(1920.0, 1.0);
        let mut list = DisplayList::new();
        append(&mut list, &paint(&right, selection, &metrics, &colors));
        let Command::Stroke {
            shape: Shape::Rect { rect, .. },
            ..
        } = list.iter().next().expect("outline")
        else {
            panic!("outline stroke");
        };
        assert_eq!(*rect, Rect::from_parts(-100.0, 100.0, 200.0, 100.0));
    }

    #[test]
    fn hud_adds_box_fill_and_text() {
        let tokens = DesignTokens::default();
        let metrics = SelectionMetrics::from_tokens(&tokens);
        let colors = SelectionColors::from_accent(Color::from_rgba8(42, 161, 152, 255));
        let out = output(0.0, 1.0);
        let selection = LogicalRect::from_raw(100.0, 100.0, 300.0, 200.0);
        let view = hud_view(
            selection,
            HudPosition::BottomRight,
            &metrics,
            &tokens.spacing,
        )
        .expect("position 4 shows the HUD");
        let mut list = DisplayList::new();
        append(
            &mut list,
            &SelectionPaint {
                hud: Some(&view),
                ..paint(&out, selection, &metrics, &colors)
            },
        );
        let commands: Vec<_> = list.iter().collect();
        assert_eq!(commands.len(), 11, "outline + 8 grips + box + text");
        assert!(matches!(commands[9], Command::Fill { .. }));
        let Command::Text(text) = commands[10] else {
            panic!("hud text");
        };
        assert_eq!(text.text, "300x200+100+100");
        assert_eq!(text.font_size, f32_from_f64(metrics.font_size));
    }

    #[test]
    fn dark_accent_gets_white_hud_text() {
        let dark = SelectionColors::from_accent(Color::from_rgba8(26, 26, 46, 255));
        assert_eq!(dark.hud_text, Color::from_rgba8(255, 255, 255, 255));
        let light = SelectionColors::from_accent(Color::from_rgba8(255, 255, 255, 255));
        assert_eq!(light.hud_text, Color::from_rgba8(0, 0, 0, 255));
        assert_eq!(
            dark.hud_background.a,
            f32::from(HUD_BACKGROUND_ALPHA) / 255.0
        );
    }
}

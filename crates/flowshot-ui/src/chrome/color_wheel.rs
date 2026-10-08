//! The color wheel popover (clean-room from the Flameshot colorpicker spec).
//!
//! The `[editor].color_palette` swatches sit at EQUAL ANGLES on a circle of
//! `radius = 3 * count + ` [`BUTTON_BASE_SIZE`] (the Flameshot formula), the
//! swatch matching the active draw color wears the 6px accent ring
//! ([`SELECTED_RING`]), and a rainbow slot trails the palette for the custom
//! pick - its eyedropper flow belongs to the eyedropper tool, so the slot
//! is a rendered seam only (a press on it logs and keeps the wheel open). The wheel centers on
//! the opening cursor position, clamped fully on-screen (the toolbar's
//! extreme-corner discipline).

use std::f32::consts::{PI, TAU};
use std::time::Instant;

use super::motion::ChromeMotion;
use crate::editor::EditorState;
use crate::editor::paint::{local_x, local_y};
use crate::motion::WHEEL_SCALE_FROM;
use crate::render::{
    Color, DisplayList, Point, Rect, ShadowSpec, Shape, Size, TextureId, f32_from_f64,
};
use crate::widgets::{IconButton, icons::Icon};
use flowshot_core::geometry::{LogicalPoint, OutputInfo};
use flowshot_core::tokens::DesignTokens;

/// The `buttonBaseSize` of the Flameshot radius formula (the toolbar button
/// footprint, logical px).
pub const BUTTON_BASE_SIZE: f32 = 32.0;
/// The swatch diameter (logical px).
pub const SWATCH_SIZE: f32 = 24.0;
/// The selected-swatch ring thickness (Flameshot: "selected = 6px ring").
pub const SELECTED_RING: f32 = 6.0;

/// The color wheel UI.
#[derive(Debug, Default)]
pub struct ColorWheel {
    /// Whether the color wheel is visible.
    pub visible: bool,
    /// The position where the color wheel was opened (global logical; the
    /// wheel centers here).
    pub position: LogicalPoint,
}

/// The wheel geometry: the popover bounds, the palette swatch rects (in
/// config order, equal angles), and the trailing rainbow slot.
#[derive(Debug, Clone)]
pub struct WheelLayout {
    /// The bounding rectangle (the circular popover footprint).
    pub rect: Rect,
    /// The swatch rectangles, in `[editor].color_palette` order.
    pub swatches: Vec<Rect>,
    /// The rainbow (custom-pick) slot rectangle.
    pub rainbow: Rect,
}

impl ColorWheel {
    /// Computes the wheel geometry: `count + 1` slots (palette + rainbow) at
    /// equal angles starting at 12 o'clock, on the Flameshot radius.
    #[must_use]
    pub fn layout(
        &self,
        editor: &EditorState,
        tokens: &DesignTokens,
        scale: f32,
        output: &OutputInfo,
    ) -> WheelLayout {
        let palette = &editor.config().editor.color_palette;
        let slots = palette.len() + 1;
        let radius = (3.0 * palette.len() as f32 + BUTTON_BASE_SIZE) * scale;
        let swatch = SWATCH_SIZE * scale;
        let margin = tokens.spacing.medium as f32 * scale;
        let half = radius + swatch / 2.0 + SELECTED_RING * scale + margin;

        // Center on the opening cursor, kept fully on-screen (min/max pair,
        // never clamp: a degenerate output smaller than the wheel must not
        // panic on inverted bounds).
        let raw_x = local_x(output, self.position.x.0);
        let raw_y = local_y(output, self.position.y.0);
        let width = output.physical_size.width.0 as f32;
        let height = output.physical_size.height.0 as f32;
        let center = Point::new(
            raw_x.max(half).min(width - half),
            raw_y.max(half).min(height - half),
        );

        let mut swatches = Vec::with_capacity(palette.len());
        let mut rainbow = Rect::from_parts(center.x, center.y, swatch, swatch);
        for index in 0..slots {
            let angle = index as f32 * TAU / slots as f32 - PI / 2.0;
            let rect = Rect::from_parts(
                center.x + radius * angle.cos() - swatch / 2.0,
                center.y + radius * angle.sin() - swatch / 2.0,
                swatch,
                swatch,
            );
            if index == palette.len() {
                rainbow = rect;
            } else {
                swatches.push(rect);
            }
        }

        let rect = Rect::from_parts(center.x - half, center.y - half, half * 2.0, half * 2.0);
        WheelLayout {
            rect,
            swatches,
            rainbow,
        }
    }

    /// Draws the wheel (circular popover, swatch ring, selected ring,
    /// rainbow slot) with the scale-in motion evaluated at `now`: the
    /// popover grows from [`WHEEL_SCALE_FROM`] and fades in; hit-testing
    /// stays on the FINAL layout (the motion checklist's interaction-leads-
    /// visual rule).
    pub fn draw(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        tokens: &DesignTokens,
        scale: f32,
        atlas: TextureId,
        output: &OutputInfo,
        motion: &ChromeMotion,
        now: Instant,
    ) {
        if !self.visible {
            return;
        }
        let progress = f32_from_f64(motion.wheel_progress(now));
        if progress <= 0.0 {
            return;
        }

        let layout = self.layout(editor, tokens, scale, output);
        let center = layout.rect.center();
        let factor = WHEEL_SCALE_FROM + (1.0 - WHEEL_SCALE_FROM) * progress;
        let layout = layout.scaled(center, factor);
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let disc = Size::new(layout.rect.size.width / 2.0, layout.rect.size.height / 2.0);
        // The square popover bounds at radius = half-side round into the
        // disc's own silhouette, so the rect shadow primitive fits.
        if let Some(spec) = ShadowSpec::from_token(&tokens.shadows.medium, scale) {
            list.shadow(layout.rect, disc.width, spec);
        }
        list.fill(
            Shape::Ellipse {
                center,
                radii: disc,
            },
            contrast.with_alpha(progress),
        );

        let swatch_radius = SWATCH_SIZE * scale * factor / 2.0;
        for (hex, rect) in editor
            .config()
            .editor
            .color_palette
            .iter()
            .zip(&layout.swatches)
        {
            let Some(color) = crate::editor::scene_color_from_hex(hex) else {
                continue;
            };
            let swatch_center = rect.center();
            list.fill(
                Shape::Ellipse {
                    center: swatch_center,
                    radii: Size::new(swatch_radius, swatch_radius),
                },
                crate::editor::render_color(color).with_alpha(progress),
            );
            if editor.color() == color {
                // The 6px ring hugs the swatch edge: a SELECTED_RING-wide
                // stroke centered SELECTED_RING/2 outside the swatch.
                let ring = swatch_radius + SELECTED_RING * scale * factor / 2.0;
                list.stroke(
                    Shape::Ellipse {
                        center: swatch_center,
                        radii: Size::new(ring, ring),
                    },
                    SELECTED_RING * scale * factor,
                    accent.with_alpha(progress),
                );
            }
        }

        IconButton::new(layout.rainbow, Icon::Rainbow)
            .alpha(progress)
            .draw(list, tokens, scale, atlas);
    }
}

impl WheelLayout {
    /// Every rect scaled by `factor` around `center` (the popover scale-in;
    /// the hit-test path keeps the unscaled layout).
    #[must_use]
    pub fn scaled(&self, center: Point, factor: f32) -> Self {
        let around = |rect: Rect| {
            Rect::from_parts(
                center.x + (rect.origin.x - center.x) * factor,
                center.y + (rect.origin.y - center.y) * factor,
                rect.size.width * factor,
                rect.size.height * factor,
            )
        };
        Self {
            rect: around(self.rect),
            swatches: self.swatches.iter().map(|rect| around(*rect)).collect(),
            rainbow: around(self.rainbow),
        }
    }
}

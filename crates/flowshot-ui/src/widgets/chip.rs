//! The aid-indicator chip: a quiet pill showing a key glyph plus a tiny
//! label, dimmed when its toggle is off and accent-tinted when on.
//!
//! Pointer-transparent by construction (the chip layer registers no
//! hit-test), token-driven like every widget: one ink (contrast) at low
//! alpha for the off state, the accent token for the on state.

use crate::render::{
    Color, DisplayList, Point, Rect, Shape, Size, TextAnchor, TextCommand, f32_from_f64,
};
use crate::selection::AVG_GLYPH_ADVANCE;
use flowshot_core::tokens::DesignTokens;

/// Off-state chip background opacity (0-255): visible but quiet.
const OFF_BG_ALPHA: u8 = 110;
/// Off-state chip ink opacity (0-255).
const OFF_INK_ALPHA: u8 = 200;
/// Key-glyph plate opacity (0-255) inside the chip (both states).
const KEY_PLATE_ALPHA: u8 = 60;
/// Text line height ratio (the render stack's standard).
const LINE_HEIGHT_RATIO: f32 = 1.2;

/// One aid-indicator chip.
#[derive(Debug, Clone, PartialEq)]
pub struct AidChip<'a> {
    /// The bounding rectangle (physical px).
    pub rect: Rect,
    /// The key glyph (e.g. `L`).
    pub key: &'a str,
    /// The tiny label (e.g. `Magnifier`).
    pub label: &'a str,
    /// Whether the aid is toggled ON (accent-tinted) or off (dimmed).
    pub active: bool,
}

impl AidChip<'_> {
    /// Measures a chip's box (the width uses the HUD's estimated-advance
    /// model - the shaped text itself is renderer-measured, the box is an
    /// estimate).
    #[must_use]
    pub fn measured_size(label: &str, tokens: &DesignTokens, scale: f32) -> Size {
        let font_size = tokens.typography.base_size as f32 * scale;
        let line = font_size * LINE_HEIGHT_RATIO;
        let pad = tokens.spacing.small as f32 * scale;
        let gap = tokens.spacing.small as f32 * scale;
        Size::new(
            pad * 2.0 + line + gap + text_width(label, font_size),
            line + pad * 2.0,
        )
    }

    /// Draws the chip into the display list.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let (background, ink) = if self.active {
            (accent, accent.readable_ink())
        } else {
            (
                contrast.with_alpha8(OFF_BG_ALPHA),
                contrast.readable_ink().with_alpha8(OFF_INK_ALPHA),
            )
        };
        list.fill(
            Shape::Rect {
                rect: self.rect,
                radius: tokens.radii.medium as f32 * scale,
            },
            background,
        );

        let font_size = tokens.typography.base_size as f32 * scale;
        let line = font_size * LINE_HEIGHT_RATIO;
        let pad = tokens.spacing.small as f32 * scale;
        let gap = tokens.spacing.small as f32 * scale;

        let plate = Rect::from_parts(
            self.rect.origin.x + pad,
            self.rect.origin.y + (self.rect.size.height - line) / 2.0,
            line,
            line,
        );
        list.fill(
            Shape::Rect {
                rect: plate,
                radius: tokens.radii.small as f32 * scale,
            },
            ink.with_alpha8(KEY_PLATE_ALPHA),
        );
        list.text(text_command(
            Point::new(
                plate.center().x - text_width(self.key, font_size) / 2.0,
                plate.center().y - line / 2.0,
            ),
            self.key,
            ink,
            font_size,
            line,
            tokens,
            true,
        ));
        list.text(text_command(
            Point::new(
                plate.right() + gap,
                self.rect.origin.y + (self.rect.size.height - line) / 2.0,
            ),
            self.label,
            ink,
            font_size,
            line,
            tokens,
            false,
        ));
    }
}

fn text_width(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size * f32_from_f64(AVG_GLYPH_ADVANCE)
}

fn text_command(
    position: Point,
    text: &str,
    color: Color,
    font_size: f32,
    line_height: f32,
    tokens: &DesignTokens,
    bold: bool,
) -> TextCommand {
    TextCommand {
        position,
        text: text.to_owned(),
        font_size,
        line_height,
        color,
        family: Some(tokens.typography.family.clone()),
        max_width: None,
        anchor: TextAnchor::TopLeft,
        bold,
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

    use super::*;
    use crate::render::Command;

    fn tokens() -> DesignTokens {
        DesignTokens::default()
    }

    fn chip(active: bool) -> AidChip<'static> {
        AidChip {
            rect: Rect::from_parts(10.0, 10.0, 120.0, 25.0),
            key: "L",
            label: "Magnifier",
            active,
        }
    }

    #[test]
    fn size_grows_with_the_label_and_scales() {
        let t = tokens();
        let small = AidChip::measured_size("Grid", &t, 1.0);
        let large = AidChip::measured_size("Magnifier", &t, 1.0);
        assert!(large.width > small.width);
        assert_eq!(small.height, large.height);
        let doubled = AidChip::measured_size("Grid", &t, 2.0);
        assert_eq!(doubled.width, small.width * 2.0);
        assert_eq!(doubled.height, small.height * 2.0);
    }

    #[test]
    fn active_and_off_chips_paint_different_inks() {
        let t = tokens();
        let mut on = DisplayList::new();
        let mut off = DisplayList::new();
        chip(true).draw(&mut on, &t, 1.0);
        chip(false).draw(&mut off, &t, 1.0);
        assert_eq!(on.len(), off.len());
        let fills = |list: &DisplayList| -> Vec<Color> {
            list.iter()
                .filter_map(|command| match command {
                    Command::Fill { color, .. } => Some(*color),
                    _ => None,
                })
                .collect()
        };
        assert_ne!(fills(&on), fills(&off), "state is visible in the ink");
        let texts = |list: &DisplayList| -> Vec<String> {
            list.iter()
                .filter_map(|command| match command {
                    Command::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(texts(&on), vec!["L".to_owned(), "Magnifier".to_owned()]);
    }
}

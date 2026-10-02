//! The size-notifier HUD (the Flameshot `NotifierBox` parity): a box
//! flashing the dispatched tool size on every size write (digits/wheel),
//! auto-hidden after [`DIGIT_RESET_DELAY`] (`notifierbox.cpp`
//! `setInterval(600)` + `timeout -> hide`).
//!
//! BORROW-MODIFIED (placement): Flameshot parents its single notifier
//! widget near the primary screen's top-left; `FlowShot` paints per window,
//! so the box anchors at EVERY window's top-left on a spacing-token margin.

use std::time::Instant;

use flowshot_core::tokens::DesignTokens;

use crate::editor::{DIGIT_RESET_DELAY, EditorState};
use crate::render::{
    Color, DisplayList, Point, Rect, Shape, Size, TextAnchor, TextCommand, TextureId, f32_from_f64,
    f32_from_u32,
};
use crate::selection::AVG_GLYPH_ADVANCE;

/// The HUD box opacity (0-255): the selection HUD's alpha (F27
/// `capturewidget.cpp` paints the geometry box at 200).
const HUD_BOX_ALPHA: u8 = 200;
/// Text line height ratio (the render stack's standard, shared with the
/// selection metrics' `LINE_SPACING_RATIO`).
const LINE_HEIGHT_RATIO: f32 = 1.2;

/// The size-notifier HUD: the auto-hide deadline IS the visibility
/// (`Some` while the box flashes - the two can never disagree).
#[derive(Debug, Default)]
pub struct SizeHud {
    until: Option<Instant>,
}

impl SizeHud {
    /// Whether the box is currently flashing.
    #[must_use]
    pub const fn visible(&self) -> bool {
        self.until.is_some()
    }

    /// Flashes the HUD, (re)arming the auto-hide deadline (the
    /// `showMessage` + `m_timer->start()` parity).
    pub fn show(&mut self, now: Instant) {
        self.until = now.checked_add(DIGIT_RESET_DELAY);
    }

    /// Hides the HUD immediately.
    pub fn hide(&mut self) {
        self.until = None;
    }

    /// The deadline flip: hides the expired HUD; `true` when visibility
    /// changed (every window must redraw the frame without the box).
    pub fn tick(&mut self, now: Instant) -> bool {
        let expired = self.until.is_some_and(|until| now >= until);
        if expired {
            self.hide();
        }
        expired
    }

    /// The auto-hide deadline while visible (the event-loop wake).
    #[must_use]
    pub const fn wake(&self) -> Option<Instant> {
        self.until
    }

    /// Draws the size HUD (window-local physical px; the box metrics are
    /// the selection HUD's estimate - token typography plus the average
    /// glyph advance).
    pub fn draw(
        &self,
        list: &mut DisplayList,
        editor: &EditorState,
        tokens: &DesignTokens,
        scale: f32,
        _atlas: TextureId,
    ) {
        if !self.visible() {
            return;
        }

        let text = format!("Size: {}", editor.tool_size());
        let font_size = f32_from_u32(tokens.typography.base_size) * scale;
        let line_height = font_size * LINE_HEIGHT_RATIO;
        let pad_h = f32_from_u32(tokens.spacing.medium) * scale;
        let pad_v = f32_from_u32(tokens.spacing.small) * scale;
        let chars = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
        let width = f32_from_u32(chars) * font_size * f32_from_f64(AVG_GLYPH_ADVANCE) + pad_h * 2.0;
        let height = line_height + pad_v * 2.0;
        let margin = f32_from_u32(tokens.spacing.large) * scale;
        let rect = Rect::new(Point::new(margin, margin), Size::new(width, height));

        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let background = contrast.with_alpha8(HUD_BOX_ALPHA);

        list.fill(
            Shape::Rect {
                rect,
                radius: f32_from_u32(tokens.radii.medium) * scale,
            },
            background,
        );

        list.text(TextCommand {
            position: Point::new(rect.origin.x + pad_h, rect.origin.y + pad_v),
            text,
            font_size,
            line_height,
            color: background.readable_ink(),
            family: Some(tokens.typography.family.clone()),
            max_width: None,
            anchor: TextAnchor::TopLeft,
            bold: false,
        });
    }
}

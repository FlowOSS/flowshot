//! Context menu widget.
//!
//! A token-driven popup list: solid contrast background with a medium
//! elevation shadow, labeled items with a hover highlight, and separators.
//! Layout metrics derive from the typography/spacing tokens so the pin menu
//! and any later surface share one implementation; the
//! entry geometry functions are the single source of truth for both drawing
//! and hit-testing.

use crate::render::{Color, DisplayList, Point, Rect, ShadowSpec, Shape, TextAnchor, TextCommand};
use flowshot_core::tokens::DesignTokens;

/// One entry of a [`ContextMenu`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuEntry {
    /// A clickable labeled item; `hovered` drives the highlight.
    Item {
        /// The visible label.
        label: String,
        /// Whether the pointer is over this item.
        hovered: bool,
    },
    /// A thin divider between item groups.
    Separator,
}

/// A context menu widget.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    /// The bounding rectangle.
    pub rect: Rect,
    /// The entries in display order (empty = background-only panel).
    pub entries: Vec<ContextMenuEntry>,
}

/// Black-or-white ink readable on `background` (the shared HUD text
/// convention, [`Color::readable_ink`]).
fn readable_ink(background: Color) -> Color {
    background.readable_ink()
}

fn rect_contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.right()
        && point.y >= rect.origin.y
        && point.y <= rect.bottom()
}

impl ContextMenu {
    /// Creates a new context menu.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            entries: Vec::new(),
        }
    }

    /// Creates a menu with `entries` inside `rect`.
    #[must_use]
    pub fn with_entries(rect: Rect, entries: Vec<ContextMenuEntry>) -> Self {
        Self { rect, entries }
    }

    /// Inner padding in physical px (spacing token).
    #[must_use]
    pub fn padding(tokens: &DesignTokens, scale: f32) -> f32 {
        tokens.spacing.small as f32 * scale
    }

    /// Item row height in physical px (typography + spacing tokens).
    #[must_use]
    pub fn item_height(tokens: &DesignTokens, scale: f32) -> f32 {
        (tokens.typography.base_size + tokens.spacing.medium) as f32 * scale
    }

    /// Separator row height in physical px (spacing token).
    #[must_use]
    pub fn separator_height(tokens: &DesignTokens, scale: f32) -> f32 {
        tokens.spacing.medium as f32 * scale
    }

    /// The total height `entries` need at `scale`, padding included.
    #[must_use]
    pub fn measure_height(entries: &[ContextMenuEntry], tokens: &DesignTokens, scale: f32) -> f32 {
        let body: f32 = entries
            .iter()
            .map(|entry| match entry {
                ContextMenuEntry::Item { .. } => Self::item_height(tokens, scale),
                ContextMenuEntry::Separator => Self::separator_height(tokens, scale),
            })
            .sum();
        body + 2.0 * Self::padding(tokens, scale)
    }

    /// The row rect of entry `index` (content-inset, full inner width).
    #[must_use]
    pub fn entry_rect(&self, index: usize, tokens: &DesignTokens, scale: f32) -> Option<Rect> {
        if index >= self.entries.len() {
            return None;
        }
        let padding = Self::padding(tokens, scale);
        let mut y = self.rect.origin.y + padding;
        for entry in &self.entries[..index] {
            y += match entry {
                ContextMenuEntry::Item { .. } => Self::item_height(tokens, scale),
                ContextMenuEntry::Separator => Self::separator_height(tokens, scale),
            };
        }
        let height = match &self.entries[index] {
            ContextMenuEntry::Item { .. } => Self::item_height(tokens, scale),
            ContextMenuEntry::Separator => Self::separator_height(tokens, scale),
        };
        Some(Rect::from_parts(
            self.rect.origin.x + padding,
            y,
            self.rect.size.width - 2.0 * padding,
            height,
        ))
    }

    /// The index of the ITEM entry under `point` (separators, padding, and
    /// misses yield `None`).
    #[must_use]
    pub fn hit(&self, point: Point, tokens: &DesignTokens, scale: f32) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .find_map(|(index, entry)| match entry {
                ContextMenuEntry::Item { .. } => self
                    .entry_rect(index, tokens, scale)
                    .filter(|rect| rect_contains(*rect, point))
                    .map(|_| index),
                ContextMenuEntry::Separator => None,
            })
    }

    /// Whether `point` is inside the menu background.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        rect_contains(self.rect, point)
    }

    /// Draws the context menu into the display list.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        let contrast = Color::from_hex_token(&tokens.palette.contrast)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
        let accent = Color::from_hex_token(&tokens.palette.accent)
            .unwrap_or(Color::from_rgba8(255, 0, 255, 255));

        let radius = tokens.radii.medium as f32 * scale;

        if let Some(spec) = ShadowSpec::from_token(&tokens.shadows.medium, scale) {
            list.shadow(self.rect, radius, spec);
        }

        list.fill(
            Shape::Rect {
                rect: self.rect,
                radius,
            },
            contrast,
        );

        let ink = readable_ink(contrast);
        let font_size = tokens.typography.base_size as f32 * scale;
        let line_height = font_size * 1.2;
        for (index, entry) in self.entries.iter().enumerate() {
            let Some(rect) = self.entry_rect(index, tokens, scale) else {
                continue;
            };
            match entry {
                ContextMenuEntry::Item { label, hovered } => {
                    if *hovered {
                        list.fill(
                            Shape::Rect {
                                rect,
                                radius: tokens.radii.small as f32 * scale,
                            },
                            accent.with_alpha8(60),
                        );
                    }
                    list.text(TextCommand {
                        position: Point::new(
                            rect.origin.x + Self::padding(tokens, scale),
                            rect.origin.y + (rect.size.height - line_height) / 2.0,
                        ),
                        text: label.clone(),
                        font_size,
                        line_height,
                        color: ink,
                        family: Some(tokens.typography.family.clone()),
                        max_width: Some(rect.size.width),
                        anchor: TextAnchor::TopLeft,
                        bold: false,
                    });
                }
                ContextMenuEntry::Separator => {
                    let y = rect.origin.y + rect.size.height / 2.0;
                    list.stroke(
                        Shape::Line {
                            from: Point::new(rect.origin.x, y),
                            to: Point::new(rect.origin.x + rect.size.width, y),
                        },
                        scale,
                        ink.with_alpha8(40),
                    );
                }
            }
        }
    }
}

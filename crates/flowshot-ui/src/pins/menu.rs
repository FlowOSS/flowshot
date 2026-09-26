//! The pin context menu model: the Flameshot-parity item set (F27 BORROW:
//! copy / save / rotate right / rotate left / increase opacity / decrease
//! opacity / close, in that order with the two original separators), built
//! on the todo-19 [`ContextMenu`] widget. The widget owns layout metrics;
//! this module owns the action mapping and window-relative placement.

use flowshot_core::tokens::DesignTokens;

use crate::render::{DisplayList, Point, Rect};
use crate::widgets::{ContextMenu, ContextMenuEntry};

use super::strings;

/// Menu width in logical px (layout constant of the pin surface, like the
/// F27 `MARGIN`; the label column derives from the longest parity label at
/// the token base font size).
const MENU_WIDTH: f32 = 170.0;

/// What a clicked menu entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Copy the pin to the clipboard (todo-28 seam).
    Copy,
    /// Save the pin to a file (todo-29 seam).
    Save,
    /// Rotate 90 degrees clockwise.
    RotateRight,
    /// Rotate 90 degrees counter-clockwise.
    RotateLeft,
    /// Raise opacity by one tenth.
    IncreaseOpacity,
    /// Lower opacity by one tenth.
    DecreaseOpacity,
    /// Close the pin.
    Close,
}

/// The item set in display order (`None` = separator).
const ENTRIES: [Option<(MenuAction, &str)>; 9] = [
    Some((MenuAction::Copy, strings::MENU_COPY)),
    Some((MenuAction::Save, strings::MENU_SAVE)),
    None,
    Some((MenuAction::RotateRight, strings::MENU_ROTATE_RIGHT)),
    Some((MenuAction::RotateLeft, strings::MENU_ROTATE_LEFT)),
    Some((MenuAction::IncreaseOpacity, strings::MENU_INCREASE_OPACITY)),
    Some((MenuAction::DecreaseOpacity, strings::MENU_DECREASE_OPACITY)),
    None,
    Some((MenuAction::Close, strings::MENU_CLOSE)),
];

/// An open pin context menu.
#[derive(Debug, Clone, PartialEq)]
pub struct PinMenu {
    widget: ContextMenu,
    hovered: Option<usize>,
}

impl PinMenu {
    /// Opens the menu with its top-left at `at` (window-local physical px),
    /// clamped so the whole menu stays inside the `window` extent.
    #[must_use]
    pub fn open(at: Point, window: (f32, f32), tokens: &DesignTokens, scale: f32) -> Self {
        let entries: Vec<ContextMenuEntry> = ENTRIES
            .iter()
            .map(|entry| match entry {
                Some((_, label)) => ContextMenuEntry::Item {
                    label: (*label).to_owned(),
                    hovered: false,
                },
                None => ContextMenuEntry::Separator,
            })
            .collect();
        let width = MENU_WIDTH * scale;
        let height = ContextMenu::measure_height(&entries, tokens, scale);
        let x = at.x.clamp(0.0, (window.0 - width).max(0.0));
        let y = at.y.clamp(0.0, (window.1 - height).max(0.0));
        Self {
            widget: ContextMenu::with_entries(Rect::from_parts(x, y, width, height), entries),
            hovered: None,
        }
    }

    /// The menu background rect (window-local physical px).
    #[must_use]
    pub fn rect(&self) -> Rect {
        self.widget.rect
    }

    /// Updates the hover highlight; returns `true` when it changed (the
    /// caller repaints).
    pub fn hover(&mut self, point: Point, tokens: &DesignTokens, scale: f32) -> bool {
        let hit = self.widget.hit(point, tokens, scale);
        if hit == self.hovered {
            return false;
        }
        self.hovered = hit;
        for (index, entry) in self.widget.entries.iter_mut().enumerate() {
            if let ContextMenuEntry::Item { hovered, .. } = entry {
                *hovered = Some(index) == hit;
            }
        }
        true
    }

    /// The action of the menu item under `point`, when there is one.
    #[must_use]
    pub fn action_at(&self, point: Point, tokens: &DesignTokens, scale: f32) -> Option<MenuAction> {
        let index = self.widget.hit(point, tokens, scale)?;
        ENTRIES.get(index)?.as_ref().map(|(action, _)| *action)
    }

    /// Whether `point` is inside the menu background.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        self.widget.contains(point)
    }

    /// Draws the menu through the widget layer.
    pub fn draw(&self, list: &mut DisplayList, tokens: &DesignTokens, scale: f32) {
        self.widget.draw(list, tokens, scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> DesignTokens {
        DesignTokens::default()
    }

    #[test]
    fn opens_clamped_inside_the_window() {
        let menu = PinMenu::open(Point::new(390.0, 290.0), (414.0, 314.0), &tokens(), 1.0);
        let rect = menu.rect();
        assert!(rect.right() <= 414.0);
        assert!(rect.bottom() <= 314.0);
        // Top-left placement is untouched when it already fits.
        let menu = PinMenu::open(Point::new(10.0, 10.0), (414.0, 314.0), &tokens(), 1.0);
        assert_eq!(menu.rect().origin, Point::new(10.0, 10.0));
    }

    #[test]
    fn hit_maps_items_to_actions_and_skips_separators() {
        let t = tokens();
        let menu = PinMenu::open(Point::new(0.0, 0.0), (414.0, 314.0), &t, 1.0);
        let padding = ContextMenu::padding(&t, 1.0);
        let item_h = ContextMenu::item_height(&t, 1.0);
        let x = padding * 2.0;
        // First item row: Copy.
        let y = padding + item_h / 2.0;
        assert_eq!(
            menu.action_at(Point::new(x, y), &t, 1.0),
            Some(MenuAction::Copy)
        );
        // Third entry is the separator (rows: copy, save, sep).
        let sep_y = padding + 2.0 * item_h + ContextMenu::separator_height(&t, 1.0) / 2.0;
        assert_eq!(menu.action_at(Point::new(x, sep_y), &t, 1.0), None);
        // Last item row: Close.
        let close_y = menu.rect().size.height - padding - item_h / 2.0;
        assert_eq!(
            menu.action_at(Point::new(x, close_y), &t, 1.0),
            Some(MenuAction::Close)
        );
        // Outside the menu: nothing.
        assert_eq!(menu.action_at(Point::new(400.0, 300.0), &t, 1.0), None);
    }

    #[test]
    fn hover_tracks_the_pointed_item() {
        let t = tokens();
        let mut menu = PinMenu::open(Point::new(0.0, 0.0), (414.0, 314.0), &t, 1.0);
        let padding = ContextMenu::padding(&t, 1.0);
        let item_h = ContextMenu::item_height(&t, 1.0);
        assert!(menu.hover(Point::new(20.0, padding + item_h / 2.0), &t, 1.0));
        assert!(!menu.hover(Point::new(21.0, padding + item_h / 2.0), &t, 1.0));
        assert!(menu.hover(Point::new(21.0, 300.0), &t, 1.0));
    }
}

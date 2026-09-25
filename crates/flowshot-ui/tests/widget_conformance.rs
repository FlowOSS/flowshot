#![allow(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::collapsible_if
)]

use flowshot_core::tokens::DesignTokens;
use flowshot_ui::render::{Color, DisplayList, Rect, TextureId};
use flowshot_ui::widgets::{Button, button::ButtonState};

#[test]
fn widget_token_conformance() {
    let tokens = DesignTokens::default();
    let _accent = Color::from_hex_token(&tokens.palette.accent).unwrap();
    let contrast = Color::from_hex_token(&tokens.palette.contrast).unwrap();

    let mut list = DisplayList::new();
    let btn = Button::new(Rect::from_parts(10.0, 10.0, 100.0, 40.0)).state(ButtonState::Hover);
    btn.draw(&mut list, &tokens, 1.0, TextureId::new(1));

    let mut found_bg = false;

    for cmd in &list {
        if let flowshot_ui::render::Command::Fill { shape, color } = cmd {
            if let flowshot_ui::render::Shape::Rect { rect, radius } = shape
                && *rect == Rect::from_parts(10.0, 10.0, 100.0, 40.0)
            {
                assert_eq!(*color, contrast.with_alpha8(20));
                assert_eq!(*radius, tokens.radii.medium as f32);
                found_bg = true;
            }
        }
    }

    assert!(found_bg);
}

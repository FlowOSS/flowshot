//! Window-layer presentation helpers both egui windows share: the winit
//! scale-factor -> egui points conversions (the single logical<->physical
//! conversion point of the embedded stack) and the egui -> winit cursor-icon
//! projection.

use winit::window::CursorIcon;

/// winit reports the scale factor as `f64`; egui works in `f32` points.
#[expect(
    clippy::cast_possible_truncation,
    reason = "scale factors are small values far within f32 range"
)]
pub(crate) fn scale_to_ppp(scale_factor: f64) -> f32 {
    scale_factor as f32
}

/// Physical pixel extents as egui points at `pixels_per_point`.
#[expect(
    clippy::cast_precision_loss,
    reason = "u32 pixel extents -> f32 egui points; window sizes are far within f32 precision"
)]
pub(crate) fn points(width: u32, height: u32, pixels_per_point: f32) -> egui::Vec2 {
    egui::Vec2::new(
        width as f32 / pixels_per_point,
        height as f32 / pixels_per_point,
    )
}

pub(crate) fn cursor_icon(icon: egui::output::CursorIcon) -> CursorIcon {
    use egui::output::CursorIcon as Egui;
    match icon {
        // A hidden cursor has no winit icon equivalent; the egui windows
        // never request one, so the default arrow is the honest fallback.
        Egui::Default | Egui::None => CursorIcon::Default,
        Egui::ContextMenu => CursorIcon::ContextMenu,
        Egui::Help => CursorIcon::Help,
        Egui::PointingHand => CursorIcon::Pointer,
        Egui::Progress => CursorIcon::Progress,
        Egui::Wait => CursorIcon::Wait,
        Egui::Cell => CursorIcon::Cell,
        Egui::Crosshair => CursorIcon::Crosshair,
        Egui::Text => CursorIcon::Text,
        Egui::VerticalText => CursorIcon::VerticalText,
        Egui::Alias => CursorIcon::Alias,
        Egui::Copy => CursorIcon::Copy,
        Egui::Move => CursorIcon::Move,
        Egui::NoDrop => CursorIcon::NoDrop,
        Egui::NotAllowed => CursorIcon::NotAllowed,
        Egui::Grab => CursorIcon::Grab,
        Egui::Grabbing => CursorIcon::Grabbing,
        Egui::AllScroll => CursorIcon::AllScroll,
        Egui::ResizeHorizontal => CursorIcon::EwResize,
        Egui::ResizeVertical => CursorIcon::NsResize,
        Egui::ResizeNeSw => CursorIcon::NeswResize,
        Egui::ResizeNwSe => CursorIcon::NwseResize,
        Egui::ResizeNorth => CursorIcon::NResize,
        Egui::ResizeSouth => CursorIcon::SResize,
        Egui::ResizeEast => CursorIcon::EResize,
        Egui::ResizeWest => CursorIcon::WResize,
        Egui::ResizeNorthEast => CursorIcon::NeResize,
        Egui::ResizeNorthWest => CursorIcon::NwResize,
        Egui::ResizeSouthEast => CursorIcon::SeResize,
        Egui::ResizeSouthWest => CursorIcon::SwResize,
        Egui::ResizeColumn => CursorIcon::ColResize,
        Egui::ResizeRow => CursorIcon::RowResize,
        Egui::ZoomIn => CursorIcon::ZoomIn,
        Egui::ZoomOut => CursorIcon::ZoomOut,
    }
}

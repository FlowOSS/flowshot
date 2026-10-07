//! Shared helpers for the `flowshot-ui` live-QA examples (each example is
//! its own crate root and includes this module with `mod common;`).

use flowshot_ui::pins::WindowCustomizer;

/// Session-aware shell name - a copy of the daemon's
/// `execute::window::session_window_customizer` (flowshot-daemon depends on
/// flowshot-ui, so the examples cannot import it without a dependency
/// cycle; this module is the single copy on the examples side). Wayland
/// `app_id` vs X11 `WM_CLASS`, same `(general, instance)` signature on
/// both winit extension traits.
pub(crate) fn session_window_customizer(
    app_id: &'static str,
    title: &'static str,
) -> WindowCustomizer {
    use flowshot_actions::clipboard::{SessionKind, detect_session};
    match detect_session() {
        Ok(SessionKind::X11) => WindowCustomizer::new(move |attributes| {
            use winit::platform::x11::WindowAttributesExtX11;
            attributes.with_name(app_id, title)
        }),
        Ok(SessionKind::Wayland) | Err(_) => WindowCustomizer::new(move |attributes| {
            use winit::platform::wayland::WindowAttributesExtWayland;
            attributes.with_name(app_id, title)
        }),
    }
}

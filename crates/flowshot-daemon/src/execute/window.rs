//! The session-aware window-attributes hook shared by every window the
//! daemon's session children create (overlay, pin, launcher, settings,
//! consent): one shell-facing name, routed per display server.

use flowshot_actions::clipboard::{detect_session, SessionKind};
use flowshot_ui::pins::zoom::ResizeAnchor;
use flowshot_ui::pins::WindowCustomizer;

/// Builds the [`WindowCustomizer`] stamping the session's shell-facing
/// window name: the Wayland arm sets `app_id`/title via
/// `WindowAttributesExtWayland::with_name` (the pre-Phase-B behavior,
/// unchanged), the X11 arm sets `WM_CLASS` via
/// `WindowAttributesExtX11::with_name` (the same `(general, instance)`
/// signature on both winit extension traits).
///
/// Routing follows the daemon's session rule ([`detect_session`], the
/// `execute/backend.rs` precedent): `WAYLAND_DISPLAY` wins - an `XWayland`
/// session exports both; with no session variable at all the Wayland arm is
/// kept, since the UI's `require_display_server` gate rejects the window
/// before any customizer runs anyway.
#[must_use]
pub(crate) fn session_window_customizer(
    app_id: &'static str,
    title: &'static str,
) -> WindowCustomizer {
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

/// The pin zoom's floating-resize anchor policy for the session's display
/// server ([`ResizeAnchor`], same routing as [`session_window_customizer`]):
/// an X11 size-only `ConfigureRequest` keeps the window's top-left
/// stationary (i3 live-probed 2026-10-06), while the Wayland arm keeps the
/// live-probed Hyprland center-invariant default.
#[must_use]
pub(crate) fn session_resize_anchor() -> ResizeAnchor {
    match detect_session() {
        Ok(SessionKind::X11) => ResizeAnchor::TopLeft,
        Ok(SessionKind::Wayland) | Err(_) => ResizeAnchor::Center,
    }
}

/// Whether pin zoom resizes are client-driven for the session's display
/// server (same routing as [`session_resize_anchor`]): X11 needs the
/// client's `request_inner_size` `ConfigureRequest` - a `WM_NORMAL_HINTS`
/// update alone reconfigures nothing. On Wayland the flag must stay
/// false: winit resizes stateless (floating) windows client-side without
/// a `Resized` event, desyncing the pin's render surface (the min/max
/// hints in `pins/effects.rs` are the compositor-driven path there).
#[must_use]
pub(crate) fn session_client_resize() -> bool {
    matches!(detect_session(), Ok(SessionKind::X11))
}

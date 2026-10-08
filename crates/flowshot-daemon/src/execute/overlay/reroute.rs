//! The X11 headless reroute (the headless-verb seam): `--no-edit`
//! invocations whose target resolves without a window skip the overlay
//! child and ride the direct leg.

use flowshot_actions::ClipboardError;
use flowshot_actions::clipboard::{SessionKind, detect_session};
use flowshot_core::config::Region;
use flowshot_core::geometry::{LogicalPoint, LogicalRect};
use flowshot_ui::launcher::RegionGeometry;

use super::session::region_rect_of;
use crate::execute::backend::resolve_cursor;
use crate::execute::direct::{ScreenTarget, Target};
use crate::execute::{ExecCtx, ExecuteError};
use crate::request::CaptureRequest;

/// The X11 headless reroute target: `Some` only on an X11 session with
/// `--no-edit` and a window-less-resolvable target - a typed `WxH[+X+Y]`
/// region token, a persisted last region (`capture last`), or
/// `--region at-cursor`. Malformed tokens and `capture last` with nothing
/// persisted are NOT rerouted: they fall through to the overlay child
/// (Wayland parity - the interactive overlay is the honest path there).
/// Wayland sessions and the headless environment always get `Ok(None)` -
/// the caller's existing path runs untouched.
///
/// Thin env-reading glue: the decision itself is the injectable pure
/// [`reroute_decision`], and the only async work (the cursor read for an
/// offset-less token) happens exactly where the original ordering had it.
///
/// # Errors
///
/// [`ExecuteError::Usage`] for an offset-less geometry when the cursor
/// position is unresolved (the launcher dispatch's precedent).
pub(super) async fn x11_headless_region(
    request: &CaptureRequest,
    ctx: &ExecCtx,
) -> Result<Option<Target>, ExecuteError> {
    let session = detect_session();
    let saved_last_region =
        if request.no_edit && request.last_region && matches!(session, Ok(SessionKind::X11)) {
            ctx.load_config().0.capture.last_region
        } else {
            None
        };
    match reroute_decision(request, &session, saved_last_region) {
        Reroute::None => Ok(None),
        Reroute::Target(target) => Ok(Some(target)),
        Reroute::NeedsCursor(geometry) => {
            let cursor = resolve_cursor().await;
            Ok(Some(cursor_reroute(&geometry, cursor)?))
        }
    }
}

/// The outcome of the pure reroute decision.
enum Reroute {
    /// No reroute: the caller's existing path (the overlay child) runs.
    None,
    /// Reroute onto the direct leg with this target.
    Target(Target),
    /// An offset-less geometry token: the live cursor read decides (the
    /// async glue resolves it, then [`cursor_reroute`] completes).
    NeedsCursor(RegionGeometry),
}

/// The reroute decision with every environment input injectable (the
/// `shortcut/detect.rs` and `session_from_env` precedent): pure, so the
/// session gates protecting the Wayland overlay are unit-testable without
/// mutating process env (parallel-test race avoidance).
fn reroute_decision(
    request: &CaptureRequest,
    session: &Result<SessionKind, ClipboardError>,
    saved_last_region: Option<Region>,
) -> Reroute {
    if !request.no_edit || !matches!(session, Ok(SessionKind::X11)) {
        return Reroute::None;
    }
    if request.last_region {
        return match windowless_reroute(request, saved_last_region) {
            Some(target) => Reroute::Target(target),
            None => Reroute::None,
        };
    }
    if let Some(target) = windowless_reroute(request, None) {
        return Reroute::Target(target);
    }
    let Some(token) = request.region.as_deref() else {
        return Reroute::None;
    };
    let Ok(geometry) = RegionGeometry::parse(token) else {
        return Reroute::None;
    };
    if geometry.x.is_none() || geometry.y.is_none() {
        return Reroute::NeedsCursor(geometry);
    }
    match region_rect_of(&geometry, None) {
        Some(rect) => Reroute::Target(Target::Region(rect)),
        None => Reroute::None,
    }
}

/// The offset-less token completion: the resolved cursor feeds
/// [`region_rect_of`]; an unresolved cursor is the typed
/// [`ExecuteError::Usage`] (the launcher dispatch's precedent).
fn cursor_reroute(
    geometry: &RegionGeometry,
    cursor: Option<LogicalPoint>,
) -> Result<Target, ExecuteError> {
    region_rect_of(geometry, cursor)
        .map(Target::Region)
        .ok_or_else(|| {
            ExecuteError::Usage(
                "offset-less region geometry needs a resolved cursor position".to_owned(),
            )
        })
}

/// The cursor-independent reroute decision: `capture last` maps to the
/// persisted region (absent persistence = no reroute, the overlay child
/// waits for an interactive selection exactly like the Wayland launch),
/// `--region at-cursor` maps to the direct leg's output-under-cursor
/// resolution (its X11 cursor read and first-output fallback included).
/// Precedence mirrors the overlay's [`preselect_for`]: `last_region` wins
/// over a region token. Everything else (typed tokens needing the live
/// cursor, malformed tokens) stays with the async caller.
///
/// [`preselect_for`]: super::wiring::preselect_for
fn windowless_reroute(
    request: &CaptureRequest,
    saved_last_region: Option<Region>,
) -> Option<Target> {
    if request.last_region {
        return saved_last_region.map(|region| {
            Target::Region(LogicalRect::from_raw(
                f64::from(region.x),
                f64::from(region.y),
                f64::from(region.width),
                f64::from(region.height),
            ))
        });
    }
    if request.region.as_deref() == Some("at-cursor") {
        return Some(Target::Screen(ScreenTarget::Cursor));
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use flowshot_core::config::Region;
    use flowshot_core::geometry::LogicalRect;

    use super::*;

    fn request(last_region: bool, region: Option<&str>) -> CaptureRequest {
        CaptureRequest {
            no_edit: true,
            last_region,
            region: region.map(str::to_owned),
            ..CaptureRequest::default()
        }
    }

    #[test]
    fn persisted_last_region_becomes_the_region_target() {
        let saved = Region {
            x: -10,
            y: 20,
            width: 640,
            height: 480,
        };
        assert_eq!(
            windowless_reroute(&request(true, None), Some(saved)),
            Some(Target::Region(LogicalRect::from_raw(
                -10.0, 20.0, 640.0, 480.0
            )))
        );
    }

    #[test]
    fn absent_last_region_does_not_reroute() {
        assert_eq!(windowless_reroute(&request(true, None), None), None);
    }

    #[test]
    fn last_region_wins_over_a_region_token() {
        let saved = Region {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        assert_eq!(
            windowless_reroute(&request(true, Some("at-cursor")), Some(saved)),
            Some(Target::Region(LogicalRect::from_raw(
                0.0, 0.0, 100.0, 100.0
            )))
        );
    }

    #[test]
    fn at_cursor_token_becomes_the_screen_cursor_target() {
        assert_eq!(
            windowless_reroute(&request(false, Some("at-cursor")), None),
            Some(Target::Screen(ScreenTarget::Cursor))
        );
    }

    #[test]
    fn typed_malformed_and_absent_tokens_stay_with_the_caller() {
        assert_eq!(
            windowless_reroute(&request(false, Some("640x480+10+20")), None),
            None
        );
        assert_eq!(
            windowless_reroute(&request(false, Some("bogus")), None),
            None
        );
        assert_eq!(windowless_reroute(&request(false, None), None), None);
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "symmetric with no_session()'s Err arm"
    )]
    fn x11() -> Result<SessionKind, ClipboardError> {
        Ok(SessionKind::X11)
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "symmetric with no_session()'s Err arm"
    )]
    fn wayland() -> Result<SessionKind, ClipboardError> {
        Ok(SessionKind::Wayland)
    }

    fn no_session() -> Result<SessionKind, ClipboardError> {
        Err(ClipboardError::NoSession)
    }

    #[test]
    fn wayland_session_never_reroutes() {
        // The Wayland-protection gate: tokens that reroute on X11 must
        // leave a Wayland session on the overlay path.
        assert!(matches!(
            reroute_decision(&request(false, Some("at-cursor")), &wayland(), None),
            Reroute::None
        ));
        let saved = Region {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        assert!(matches!(
            reroute_decision(&request(true, None), &wayland(), Some(saved)),
            Reroute::None
        ));
    }

    #[test]
    fn no_session_never_reroutes() {
        assert!(matches!(
            reroute_decision(&request(false, Some("at-cursor")), &no_session(), None),
            Reroute::None
        ));
    }

    #[test]
    fn edit_mode_never_reroutes_even_on_x11() {
        let editable = CaptureRequest {
            no_edit: false,
            region: Some("at-cursor".to_owned()),
            ..CaptureRequest::default()
        };
        assert!(matches!(
            reroute_decision(&editable, &x11(), None),
            Reroute::None
        ));
    }

    #[test]
    fn x11_at_cursor_reroutes_to_the_screen_cursor_target() {
        assert!(matches!(
            reroute_decision(&request(false, Some("at-cursor")), &x11(), None),
            Reroute::Target(Target::Screen(ScreenTarget::Cursor))
        ));
    }

    #[test]
    fn x11_last_region_without_persistence_falls_through_to_the_overlay() {
        assert!(matches!(
            reroute_decision(&request(true, None), &x11(), None),
            Reroute::None
        ));
    }

    #[test]
    fn x11_persisted_last_region_reroutes() {
        let saved = Region {
            x: 1,
            y: 2,
            width: 30,
            height: 40,
        };
        assert!(matches!(
            reroute_decision(&request(true, None), &x11(), Some(saved)),
            Reroute::Target(Target::Region(_))
        ));
    }

    #[test]
    fn offsetless_token_needs_the_cursor() {
        assert!(matches!(
            reroute_decision(&request(false, Some("640x480")), &x11(), None),
            Reroute::NeedsCursor(_)
        ));
    }

    #[test]
    fn offset_token_reroutes_without_a_cursor_read() {
        assert!(matches!(
            reroute_decision(&request(false, Some("640x480+10+20")), &x11(), None),
            Reroute::Target(Target::Region(_))
        ));
    }

    #[test]
    fn malformed_token_falls_through() {
        assert!(matches!(
            reroute_decision(&request(false, Some("bogus")), &x11(), None),
            Reroute::None
        ));
    }

    #[test]
    fn unresolved_cursor_for_an_offsetless_token_is_usage() {
        let geometry = RegionGeometry::parse("640x480").unwrap();
        assert!(matches!(
            cursor_reroute(&geometry, None),
            Err(ExecuteError::Usage(_))
        ));
    }

    #[test]
    fn resolved_cursor_completes_the_offsetless_token() {
        let geometry = RegionGeometry::parse("640x480").unwrap();
        let cursor = Some(LogicalPoint::from_raw(100.0, 100.0));
        assert!(matches!(
            cursor_reroute(&geometry, cursor),
            Ok(Target::Region(_))
        ));
    }
}

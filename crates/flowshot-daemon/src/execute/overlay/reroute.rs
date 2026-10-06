//! The X11 headless reroute (plan decision #6's seam): `--no-edit`
//! invocations whose target resolves without a window skip the overlay
//! child and ride the direct leg.

use flowshot_core::config::Region;
use flowshot_core::geometry::LogicalRect;
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
/// # Errors
///
/// [`ExecuteError::Usage`] for an offset-less geometry when the cursor
/// position is unresolved (the launcher dispatch's precedent).
pub(super) async fn x11_headless_region(
    request: &CaptureRequest,
    ctx: &ExecCtx,
) -> Result<Option<Target>, ExecuteError> {
    if !request.no_edit
        || !matches!(
            flowshot_actions::clipboard::detect_session(),
            Ok(flowshot_actions::clipboard::SessionKind::X11)
        )
    {
        return Ok(None);
    }
    if request.last_region {
        let (config, _) = ctx.load_config();
        return Ok(windowless_reroute(request, config.capture.last_region));
    }
    if let Some(target) = windowless_reroute(request, None) {
        return Ok(Some(target));
    }
    let Some(token) = request.region.as_deref() else {
        return Ok(None);
    };
    let Ok(geometry) = RegionGeometry::parse(token) else {
        return Ok(None);
    };
    let cursor = if geometry.x.is_none() || geometry.y.is_none() {
        resolve_cursor().await
    } else {
        None
    };
    match region_rect_of(&geometry, cursor) {
        Some(rect) => Ok(Some(Target::Region(rect))),
        None => Err(ExecuteError::Usage(
            "offset-less region geometry needs a resolved cursor position".to_owned(),
        )),
    }
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
}

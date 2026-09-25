//! Mapping of the `wl_output` transform wire enum onto the shared geometry
//! [`Transform`].

use flowshot_core::geometry::Transform;
use wayland_client::protocol::wl_output;

/// Maps a `wl_output` transform onto the shared geometry enum.
///
/// The eight wire values map one-to-one. The wildcard arm exists only
/// because the generated protocol enum is `#[non_exhaustive]`; unknown
/// values cannot survive `wayland-client` message parsing, so it is
/// unreachable in practice and degrades to [`Transform::Normal`] with a
/// warning rather than failing the output.
#[must_use]
pub(crate) fn transform_from_wl_output(transform: wl_output::Transform) -> Transform {
    match transform {
        wl_output::Transform::Normal => Transform::Normal,
        wl_output::Transform::_90 => Transform::Rot90,
        wl_output::Transform::_180 => Transform::Rot180,
        wl_output::Transform::_270 => Transform::Rot270,
        wl_output::Transform::Flipped => Transform::Flipped,
        wl_output::Transform::Flipped90 => Transform::Flipped90,
        wl_output::Transform::Flipped180 => Transform::Flipped180,
        wl_output::Transform::Flipped270 => Transform::Flipped270,
        _ => {
            tracing::warn!(
                transform = ?transform,
                "unknown wl_output transform value; assuming normal orientation"
            );
            Transform::Normal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_wl_output_transforms_map_to_geometry_transforms() {
        let expected = [
            (wl_output::Transform::Normal, Transform::Normal),
            (wl_output::Transform::_90, Transform::Rot90),
            (wl_output::Transform::_180, Transform::Rot180),
            (wl_output::Transform::_270, Transform::Rot270),
            (wl_output::Transform::Flipped, Transform::Flipped),
            (wl_output::Transform::Flipped90, Transform::Flipped90),
            (wl_output::Transform::Flipped180, Transform::Flipped180),
            (wl_output::Transform::Flipped270, Transform::Flipped270),
        ];
        for (wire, geometry) in expected {
            assert_eq!(transform_from_wl_output(wire), geometry, "{wire:?}");
        }
    }
}

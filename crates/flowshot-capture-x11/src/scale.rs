//! Scale derivation for X11 outputs.
//!
//! X11 has no scale concept: the screen is a pixel-exact framebuffer and
//! `HiDPI` is a per-toolkit convention driven by `Xft.dpi`. `FlowShot`'s
//! shared geometry still needs a per-output scale for its logical space, so
//! this module derives one - documented as approximate:
//!
//! 1. The session-global `Xft.dpi` resource wins when the desktop set it
//!    ([`parse_xft_dpi`]): `scale = dpi / 96`.
//! 2. Otherwise a per-output RANDR physical-size heuristic applies
//!    ([`scale_from_mm`]): `dpi = px / (mm / 25.4)`, `scale = dpi / 96`.
//!    The pixel extent must be the *panel-native* (pre-transform) width and
//!    the millimeter extent the panel-native `mm_width`: the X server copies
//!    `GetMonitors` millimeter sizes straight from the output's EDID data
//!    without adjusting them for CRTC rotation, while the pixel box it
//!    reports is post-rotation.
//! 3. Either way the result is rounded to the nearest 0.25 and clamped to
//!    `[1.0, 4.0]` ([`quantize_scale`]).
//!
//! Every function here is pure; the property and RANDR reads that feed them
//! live in [`crate::output`].

/// The reference pixel density `FlowShot`'s logical space is defined against
/// (the `Xft.dpi` value that means "scale 1").
const REFERENCE_DPI: f64 = 96.0;

/// Millimeters per inch.
const MM_PER_INCH: f64 = 25.4;

/// The resource key carrying the desktop's DPI choice in the root
/// `RESOURCE_MANAGER` property.
const XFT_DPI_KEY: &str = "Xft.dpi:";

/// Extracts the `Xft.dpi` value from a `RESOURCE_MANAGER` property string.
///
/// The property holds `xrdb`-style `name:\tvalue` lines; this finds the
/// `Xft.dpi` line and parses its value. Returns `None` when the key is
/// absent, its value is not a number, or the number is not a usable density
/// (non-finite or non-positive) - all of which mean "the desktop made no
/// DPI statement" and fall through to the millimeter heuristic.
///
/// # Examples
///
/// ```
/// use flowshot_capture_x11::parse_xft_dpi;
///
/// assert_eq!(parse_xft_dpi("Xft.dpi:\t192\nXft.hinting:\t1\n"), Some(192.0));
/// assert_eq!(parse_xft_dpi("Xft.hinting:\t1\n"), None);
/// ```
#[must_use]
pub fn parse_xft_dpi(resource_manager: &str) -> Option<f64> {
    resource_manager
        .lines()
        .map(str::trim_start)
        .find_map(|line| line.strip_prefix(XFT_DPI_KEY))
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|dpi| dpi.is_finite() && *dpi > 0.0)
}

/// Derives a raw (unquantized) scale from a panel-native pixel extent and
/// the matching physical extent in millimeters: `dpi = px / (mm / 25.4)`,
/// `scale = dpi / 96`.
///
/// A zero `mm` (servers and outputs that report no physical size) or a zero
/// `px` falls back to `1.0`; the result is always finite and positive, never
/// `NaN`.
///
/// # Examples
///
/// ```
/// use flowshot_capture_x11::scale_from_mm;
///
/// // 1920 px over 508 mm (20 in) is 96 dpi: scale 1.
/// assert!((scale_from_mm(1920, 508) - 1.0).abs() < 1e-9);
/// // No physical size reported: the clean fallback.
/// assert_eq!(scale_from_mm(1920, 0), 1.0);
/// ```
#[must_use]
pub fn scale_from_mm(px: u32, mm: u32) -> f64 {
    if px == 0 || mm == 0 {
        return 1.0;
    }
    let dpi = f64::from(px) / (f64::from(mm) / MM_PER_INCH);
    dpi / REFERENCE_DPI
}

/// Rounds `scale` to the nearest 0.25 and clamps it into `[1.0, 4.0]`.
///
/// `NaN` falls back to `1.0`; infinities clamp to the range bounds like any
/// other out-of-range value.
///
/// # Examples
///
/// ```
/// use flowshot_capture_x11::quantize_scale;
///
/// assert_eq!(quantize_scale(2.2119), 2.25);
/// assert_eq!(quantize_scale(0.5), 1.0);
/// assert_eq!(quantize_scale(5.0), 4.0);
/// ```
#[must_use]
pub fn quantize_scale(scale: f64) -> f64 {
    if scale.is_nan() {
        return 1.0;
    }
    let quantized = (scale * 4.0).round() / 4.0;
    quantized.clamp(1.0, 4.0)
}

/// The output scale for `FlowShot`'s logical space: the session-global
/// `Xft.dpi` when the desktop set one, otherwise the per-output millimeter
/// heuristic over the panel-native extents - always quantized and clamped.
///
/// # Examples
///
/// ```
/// use flowshot_capture_x11::derive_scale;
///
/// // Xft.dpi wins over what the panel millimeters would suggest.
/// assert_eq!(derive_scale(Some(192.0), 1920, 508), 2.0);
/// assert_eq!(derive_scale(None, 3840, 508), 2.0);
/// ```
#[must_use]
pub fn derive_scale(xft_dpi: Option<f64>, native_px: u32, mm: u32) -> f64 {
    match xft_dpi {
        Some(dpi) => quantize_scale(dpi / REFERENCE_DPI),
        None => quantize_scale(scale_from_mm(native_px, mm)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // Quantized scales are exact binary fractions (0.25 steps) and the
    // exact-value fixtures use integral DPI, so `assert_eq!` on f64 is the
    // intended assertion; approximate fixtures compare with an epsilon.
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn parse_xft_dpi_reads_the_tab_separated_value() {
        let resource_manager = "Xft.antialias:\t1\nXft.dpi:\t192\nXft.hinting:\t1\n";
        assert_eq!(parse_xft_dpi(resource_manager), Some(192.0));
    }

    #[test]
    fn parse_xft_dpi_accepts_space_separation_and_fractions() {
        assert_eq!(parse_xft_dpi("Xft.dpi: 96"), Some(96.0));
        assert_eq!(parse_xft_dpi("Xft.dpi:\t143.5"), Some(143.5));
        assert_eq!(parse_xft_dpi("  Xft.dpi:\t120"), Some(120.0));
    }

    #[test]
    fn parse_xft_dpi_missing_key_is_none() {
        assert_eq!(parse_xft_dpi("Xft.antialias:\t1\nXft.hinting:\t1\n"), None);
        assert_eq!(parse_xft_dpi(""), None);
    }

    #[test]
    fn parse_xft_dpi_malformed_values_are_none() {
        assert_eq!(parse_xft_dpi("Xft.dpi:\tgarbage"), None);
        assert_eq!(parse_xft_dpi("Xft.dpi:"), None);
        assert_eq!(parse_xft_dpi("Xft.dpi:\t0"), None);
        assert_eq!(parse_xft_dpi("Xft.dpi:\t-96"), None);
    }

    #[test]
    fn parse_xft_dpi_does_not_match_longer_resource_names() {
        assert_eq!(parse_xft_dpi("Xft.dpiScaled:\t96"), None);
        assert_eq!(parse_xft_dpi("my.Xft.dpi:\t96"), None);
    }

    #[test]
    fn scale_from_mm_computes_dpi_over_reference() {
        // 3840 px over 508 mm (20 in) is 192 dpi = scale 2, within f64
        // epsilon (25.4 is not exact in binary).
        let scale = scale_from_mm(3840, 508);
        assert!((scale - 2.0).abs() < 1e-9, "{scale}");
        // This machine's panel: 2880 px over 344 mm is ~212.7 dpi ~ 2.215.
        let hidpi = scale_from_mm(2880, 344);
        assert!((hidpi - 2.2149).abs() < 1e-3, "{hidpi}");
    }

    #[test]
    fn scale_from_mm_zero_extents_fall_back_to_one() {
        assert_eq!(scale_from_mm(2880, 0), 1.0);
        assert_eq!(scale_from_mm(0, 344), 1.0);
        assert_eq!(scale_from_mm(0, 0), 1.0);
        assert!(scale_from_mm(2880, 0).is_finite());
    }

    #[test]
    fn quantize_scale_rounds_to_quarter_steps() {
        assert_eq!(quantize_scale(2.2119), 2.25);
        assert_eq!(quantize_scale(1.05), 1.0);
        assert_eq!(quantize_scale(1.125), 1.25);
        assert_eq!(quantize_scale(2.0), 2.0);
        assert_eq!(quantize_scale(1.4), 1.5);
    }

    #[test]
    fn quantize_scale_clamps_to_the_valid_range() {
        assert_eq!(quantize_scale(0.5), 1.0);
        assert_eq!(quantize_scale(0.0), 1.0);
        assert_eq!(quantize_scale(-3.0), 1.0);
        assert_eq!(quantize_scale(1.0), 1.0);
        assert_eq!(quantize_scale(4.0), 4.0);
        assert_eq!(quantize_scale(4.5), 4.0);
        assert_eq!(quantize_scale(f64::INFINITY), 4.0);
        assert_eq!(quantize_scale(f64::NEG_INFINITY), 1.0);
    }

    #[test]
    fn quantize_scale_nan_falls_back_to_one() {
        assert_eq!(quantize_scale(f64::NAN), 1.0);
    }

    #[test]
    fn derive_scale_prefers_xft_dpi_over_the_mm_heuristic() {
        // The mm heuristic on 2880 px / 344 mm would give 2.25; a desktop
        // that set Xft.dpi=96 means scale 1 and wins.
        assert_eq!(derive_scale(Some(96.0), 2880, 344), 1.0);
        assert_eq!(derive_scale(Some(192.0), 1920, 508), 2.0);
    }

    #[test]
    fn derive_scale_falls_back_to_mm_without_xft_dpi() {
        assert_eq!(derive_scale(None, 3840, 508), 2.0);
        assert_eq!(derive_scale(None, 2880, 344), 2.25);
        assert_eq!(derive_scale(None, 2880, 0), 1.0);
    }
}

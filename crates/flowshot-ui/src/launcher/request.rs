//! The launcher's typed dispatch vocabulary: the manual geometry grammar
//! (parsed once at the input boundary) and the request the Capture button
//! emits.
//!
//! The grammar mirrors the CLI's `--region <WxH[+X+Y]>` token (Oracle r4):
//! unsigned non-zero dimensions, optional independently signed offsets in
//! global logical pixels. The parser is a dialog-local copy because the
//! CLI's lives in `flowshot-cli` (which depends on this crate - the shared
//! home is `flowshot-core`, recorded for the orchestrator); the grammar
//! rules and the wire token format are identical by construction
//! ([`RegionGeometry::to_token`] round-trips through [`RegionGeometry::parse`]).

use std::fmt;

/// Why a geometry entry is not (yet) a capturable region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryIssue {
    /// The field is empty - shown as the grammar hint, not an error.
    Empty,
    /// The text is outside the `WxH[+X+Y]` grammar.
    Malformed,
}

impl fmt::Display for GeometryIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str(crate::launcher::strings::GEOMETRY_HINT),
            Self::Malformed => f.write_str(crate::launcher::strings::GEOMETRY_INVALID),
        }
    }
}

impl std::error::Error for GeometryIssue {}

/// A parsed manual geometry entry: a rectangle in global logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionGeometry {
    /// Region width (> 0).
    pub width: u32,
    /// Region height (> 0).
    pub height: u32,
    /// X offset (signed; multi-monitor layouts can be negative). `None` =
    /// centered at the cursor by the executor (the CLI's offset-less
    /// `WxH` semantics).
    pub x: Option<i32>,
    /// Y offset (signed). `None` = centered at the cursor by the executor.
    pub y: Option<i32>,
}

impl RegionGeometry {
    /// Parses the `WxH[+X+Y]` grammar (the CLI's `--region` rules).
    ///
    /// # Errors
    ///
    /// [`GeometryIssue::Empty`] for blank input, [`GeometryIssue::Malformed`]
    /// for anything outside the grammar (missing `x`, non-digit or zero
    /// dimensions, a malformed offset pair, trailing junk, overflow).
    pub fn parse(text: &str) -> Result<Self, GeometryIssue> {
        let token = text.trim();
        if token.is_empty() {
            return Err(GeometryIssue::Empty);
        }
        let invalid = GeometryIssue::Malformed;
        let (size, offsets) = match token.find(['+', '-']) {
            Some(index) => (&token[..index], Some(&token[index..])),
            None => (token, None),
        };
        let (width_str, height_str) = size.split_once('x').ok_or(invalid)?;
        let width = parse_dimension(width_str).ok_or(invalid)?;
        let height = parse_dimension(height_str).ok_or(invalid)?;
        let (x, y) = match offsets {
            None => (None, None),
            Some(rest) => {
                let (x_str, y_str) = split_offsets(rest).ok_or(invalid)?;
                let x = parse_coordinate(x_str).ok_or(invalid)?;
                let y = parse_coordinate(y_str).ok_or(invalid)?;
                (Some(x), Some(y))
            }
        };
        Ok(Self {
            width,
            height,
            x,
            y,
        })
    }

    /// The wire token (`CaptureRequest.region` vocabulary): `WxH` or
    /// `WxH+X+Y` with explicit signs.
    #[must_use]
    pub fn to_token(&self) -> String {
        let mut token = format!("{}x{}", self.width, self.height);
        if let (Some(x), Some(y)) = (self.x, self.y) {
            token.push_str(&signed(x));
            token.push_str(&signed(y));
        }
        token
    }
}

/// One `±N` offset with an explicit sign (`unsigned_abs` keeps `i32::MIN`
/// representable - `abs` would overflow).
fn signed(value: i32) -> String {
    if value < 0 {
        format!("-{}", value.unsigned_abs())
    } else {
        format!("+{value}")
    }
}

/// One `±N±N` offset pair tail (`+10+20`, `-10-20`, mixed signs allowed).
fn split_offsets(rest: &str) -> Option<(&str, &str)> {
    let second = rest.get(1..)?.find(['+', '-'])? + 1;
    Some(rest.split_at(second))
}

fn parse_dimension(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|value| *value > 0)
}

fn parse_coordinate(text: &str) -> Option<i32> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<i32>().ok()
}

/// What the Capture button dispatches (the binary layer maps it onto the
/// daemon's `Capture` / `CaptureScreen` commands - see the module header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherRequest {
    /// Manual geometry: the daemon's `Capture` with region semantics.
    Region {
        /// The validated geometry entry.
        geometry: RegionGeometry,
        /// The delay spinner value in milliseconds.
        delay_ms: u32,
    },
    /// A monitor from the live probe: the daemon's `CaptureScreen(index)`.
    /// `delay_ms` rides along for the binary layer (the daemon's
    /// `CaptureScreen(u32)` wire slot carries no delay - the binary layer
    /// owns the mapping decision).
    Screen {
        /// The output index in probe order (the `CaptureScreen` vocabulary).
        screen: u32,
        /// The delay spinner value in milliseconds.
        delay_ms: u32,
    },
}

//! The daemon-side `Invoke(argv)` parser (plan todo 32/35/38).
//!
//! The CLI forwards exactly four shapes over the lossless argv channel
//! (the `wire.rs` mapping table): `capture full|screen [<n|connector>]`
//! WITH modifiers, `pin [FILE]`, and `color`. A daemon cannot re-parse
//! with the CLI's clap surface (flowshot-cli depends on this crate - the
//! reverse edge would be circular), so this module hand-parses that FROZEN
//! subset with the CLI's grammar rules mirrored 1:1 (the todo-37
//! `RegionGeometry` precedent; a shared core home is the recorded
//! orchestrator follow-up). Unknown verbs/flags are typed usage errors,
//! never silent drops. Beyond the forwarded subset it also accepts the
//! bare/interactive `capture` forms so `busctl` callers get the full
//! surface (plan flow 10).

use std::path::PathBuf;

use crate::request::CaptureRequest;

use super::ExecuteError;
use super::direct::{ScreenTarget, Target};

/// One parsed `Invoke` call.
#[derive(Debug, Clone, PartialEq)]
pub enum InvokeCall {
    /// A window-less capture (full / one screen / explicit region).
    Direct(Target, CaptureRequest),
    /// An interactive overlay capture (region preselect rides the request).
    Interactive(CaptureRequest),
    /// `pin [FILE]`: pin an image file, or this daemon's last capture.
    Pin(Option<PathBuf>),
    /// `color`: the standalone eyedropper session.
    Color,
}

/// Parses one forwarded argv tail (everything after argv\[0\]).
///
/// # Errors
///
/// [`ExecuteError::Usage`] for empty argv, unknown verbs, unknown flags,
/// missing flag values, and malformed flag payloads.
pub fn parse(argv: &[String]) -> Result<InvokeCall, ExecuteError> {
    let (verb, rest) = argv
        .split_first()
        .ok_or_else(|| ExecuteError::Usage("empty Invoke argv".to_owned()))?;
    match verb.as_str() {
        "capture" => parse_capture(rest),
        "pin" => match rest {
            [] => Ok(InvokeCall::Pin(None)),
            [file] => Ok(InvokeCall::Pin(Some(PathBuf::from(file)))),
            _ => Err(ExecuteError::Usage(
                "pin takes at most one FILE argument".to_owned(),
            )),
        },
        "color" => {
            reject_extras(rest, "color")?;
            Ok(InvokeCall::Color)
        }
        other => Err(ExecuteError::Usage(format!(
            "unknown Invoke verb {other:?} (expected capture, pin, or color)"
        ))),
    }
}

fn parse_capture(args: &[String]) -> Result<InvokeCall, ExecuteError> {
    let mut target: Option<Target> = None;
    let mut request = CaptureRequest::default();
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "full" => assign_target(&mut target, Target::Full)?,
            "screen" => {
                // A flag directly after `screen` is the cursor form's first
                // modifier, never the selector (peek, don't swallow).
                let spec = match iter.peek() {
                    Some(next) if !next.starts_with('-') => {
                        let value = (*next).clone();
                        iter.next();
                        Some(value)
                    }
                    _ => None,
                };
                assign_target(&mut target, Target::Screen(parse_screen(spec.as_ref())?))?;
            }
            "region" | "last" => {
                // `capture region` is the bare interactive default;
                // `capture last` repeats the persisted region.
                if arg == "last" {
                    request.last_region = true;
                }
            }
            "-d" | "--delay" => {
                let value = next_value(&mut iter, arg)?;
                request.delay_ms = value.parse::<u32>().map_err(|_| {
                    ExecuteError::Usage(format!("--delay wants milliseconds, got {value:?}"))
                })?;
            }
            "--instant" => request.instant = true,
            "--no-edit" => request.no_edit = true,
            "-c" | "--copy" => request.copy = true,
            "-o" | "--output" => request.output = Some(next_value(&mut iter, arg)?),
            "--pin" => request.pin = true,
            "--upload" => request.upload = true,
            "--hide-cursor" => request.hide_cursor = true,
            "--region" => request.region = Some(next_value(&mut iter, arg)?),
            "--last-region" => request.last_region = true,
            "--raw" | "--print-geometry" => {
                return Err(ExecuteError::Usage(format!(
                    "{arg} is a one-shot stdout mode and never reaches the daemon"
                )));
            }
            other => {
                return Err(ExecuteError::Usage(format!(
                    "unknown capture argument {other:?}"
                )));
            }
        }
    }
    match target {
        Some(target) => Ok(InvokeCall::Direct(target, request)),
        None => Ok(InvokeCall::Interactive(request)),
    }
}

fn assign_target(slot: &mut Option<Target>, target: Target) -> Result<(), ExecuteError> {
    if slot.is_some() {
        return Err(ExecuteError::Usage(
            "capture takes exactly one target (full, screen, or region)".to_owned(),
        ));
    }
    *slot = Some(target);
    Ok(())
}

fn parse_screen(spec: Option<&String>) -> Result<ScreenTarget, ExecuteError> {
    let Some(spec) = spec else {
        return Ok(ScreenTarget::Cursor);
    };
    if spec.is_empty() || spec.contains(char::is_whitespace) {
        return Err(ExecuteError::Usage(
            "screen selector is empty or contains whitespace".to_owned(),
        ));
    }
    if spec.bytes().all(|byte| byte.is_ascii_digit()) {
        return spec
            .parse::<u32>()
            .map(ScreenTarget::Index)
            .map_err(|_| ExecuteError::Usage(format!("screen index {spec} out of range")));
    }
    Ok(ScreenTarget::Connector(spec.clone()))
}

fn next_value(
    iter: &mut std::iter::Peekable<std::slice::Iter<'_, String>>,
    flag: &str,
) -> Result<String, ExecuteError> {
    iter.next()
        .cloned()
        .ok_or_else(|| ExecuteError::Usage(format!("{flag} requires a value argument")))
}

fn reject_extras(rest: &[String], verb: &str) -> Result<(), ExecuteError> {
    if let Some(extra) = rest.first() {
        return Err(ExecuteError::Usage(format!(
            "unexpected argument {extra:?} after {verb}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    #[test]
    fn parses_full_with_modifiers() {
        let call = parse(&argv(&["capture", "full", "-d", "2000", "-c"])).unwrap();
        let InvokeCall::Direct(Target::Full, request) = call else {
            panic!("expected a direct full capture, got {call:?}");
        };
        assert_eq!(request.delay_ms, 2000);
        assert!(request.copy);
    }

    #[test]
    fn parses_screen_by_index_and_connector_and_cursor() {
        assert_eq!(
            parse(&argv(&["capture", "screen", "1"])).unwrap(),
            InvokeCall::Direct(
                Target::Screen(ScreenTarget::Index(1)),
                CaptureRequest::default()
            )
        );
        assert_eq!(
            parse(&argv(&["capture", "screen", "DP-1", "--pin"])).unwrap(),
            InvokeCall::Direct(
                Target::Screen(ScreenTarget::Connector("DP-1".to_owned())),
                CaptureRequest {
                    pin: true,
                    ..CaptureRequest::default()
                }
            )
        );
        assert_eq!(
            parse(&argv(&["capture", "screen"])).unwrap(),
            InvokeCall::Direct(
                Target::Screen(ScreenTarget::Cursor),
                CaptureRequest::default()
            )
        );
        // A flag directly after `screen` is NOT eaten as the selector.
        assert_eq!(
            parse(&argv(&["capture", "screen", "--copy"])).unwrap(),
            InvokeCall::Direct(
                Target::Screen(ScreenTarget::Cursor),
                CaptureRequest {
                    copy: true,
                    ..CaptureRequest::default()
                }
            )
        );
    }

    #[test]
    fn parses_bare_and_region_interactive_forms() {
        assert_eq!(
            parse(&argv(&["capture"])).unwrap(),
            InvokeCall::Interactive(CaptureRequest::default())
        );
        let InvokeCall::Interactive(request) = parse(&argv(&[
            "capture",
            "--region",
            "200x100+50+50",
            "--instant",
        ]))
        .unwrap() else {
            panic!("expected an interactive capture");
        };
        assert_eq!(request.region.as_deref(), Some("200x100+50+50"));
        assert!(request.instant);
        let InvokeCall::Interactive(request) = parse(&argv(&["capture", "last"])).unwrap() else {
            panic!("expected an interactive last-region capture");
        };
        assert!(request.last_region);
    }

    #[test]
    fn parses_pin_and_color() {
        assert_eq!(parse(&argv(&["pin"])).unwrap(), InvokeCall::Pin(None));
        assert_eq!(
            parse(&argv(&["pin", "/tmp/x.png"])).unwrap(),
            InvokeCall::Pin(Some(PathBuf::from("/tmp/x.png")))
        );
        assert_eq!(parse(&argv(&["color"])).unwrap(), InvokeCall::Color);
    }

    #[test]
    fn rejects_unknown_and_malformed_forms() {
        assert!(parse(&argv(&["gui"])).is_err());
        assert!(parse(&[]).is_err());
        assert!(parse(&argv(&["capture", "--bogus"])).is_err());
        assert!(parse(&argv(&["capture", "full", "screen"])).is_err());
        assert!(parse(&argv(&["capture", "full", "-d"])).is_err());
        assert!(parse(&argv(&["capture", "full", "-d", "soon"])).is_err());
        assert!(parse(&argv(&["capture", "full", "--raw"])).is_err());
        assert!(parse(&argv(&["color", "extra"])).is_err());
        assert!(parse(&argv(&["pin", "a", "b"])).is_err());
    }
}

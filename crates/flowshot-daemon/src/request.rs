//! Typed capture-request parsing at the `D-Bus` boundary (parse, don't
//! validate: the `a{sv}` wire bag becomes a typed struct exactly once).

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

use crate::error::DaemonError;

/// Wire keys of the `Capture(options)` vardict. Stable contract; the
/// CLI<->`D-Bus` mapping table is recorded in the docs.
pub const CAPTURE_OPTION_KEYS: &[&str] = &[
    "delay_ms",
    "instant",
    "no_edit",
    "copy",
    "output",
    "pin",
    "upload",
    "raw",
    "print_geometry",
    "hide_cursor",
    "region",
    "last_region",
];

/// The typed form of one `Capture(options)` call: the CLI
/// modifier set as data.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent spec-mandated capture modifier (the CLI surface)"
)]
pub struct CaptureRequest {
    /// `-d/--delay <ms>`: wait before capturing.
    pub delay_ms: u32,
    /// `--instant`: accept-on-select, skip the edit review.
    pub instant: bool,
    /// `--no-edit`: skip the editor, go straight to post-capture actions.
    pub no_edit: bool,
    /// `-c/--copy`: add the copy action for this invocation.
    pub copy: bool,
    /// `-o/--output <file|dir>`: explicit destination.
    pub output: Option<String>,
    /// `--pin`: add the pin action.
    pub pin: bool,
    /// `--upload`: add the upload action.
    pub upload: bool,
    /// `--raw`: PNG bytes to the invoker's stdout (forces one-shot mode in
    /// the CLI - stdout never routes over `D-Bus`).
    pub raw: bool,
    /// `--print-geometry`: `WxH+X+Y` to stdout (same one-shot rule).
    pub print_geometry: bool,
    /// `--hide-cursor`: exclude the cursor from the capture.
    pub hide_cursor: bool,
    /// `--region <WxH[+X+Y]|at-cursor>`: preselected region (grammar is
    /// parsed by the capture executor - the wire carries the
    /// raw token).
    pub region: Option<String>,
    /// `--last-region`: repeat the persisted `[capture].last_region`.
    pub last_region: bool,
}

impl CaptureRequest {
    /// Parses the `Capture` vardict. Unknown keys are ignored with a
    /// warning (forward compatibility: a newer CLI may talk to an older
    /// daemon); a known key with the wrong type is a typed error.
    ///
    /// # Errors
    ///
    /// [`DaemonError::InvalidArgs`] when a known key carries a value of
    /// the wrong `D-Bus` type.
    pub fn from_vardict(options: &HashMap<String, OwnedValue>) -> Result<Self, DaemonError> {
        for key in options.keys() {
            if !CAPTURE_OPTION_KEYS.contains(&key.as_str()) {
                tracing::warn!(key = %key, "ignoring unknown capture option");
            }
        }
        let mut request = Self::default();
        if let Some(value) = options.get("delay_ms") {
            request.delay_ms = u32_value("delay_ms", value)?;
        }
        for (key, slot) in [
            ("instant", &mut request.instant),
            ("no_edit", &mut request.no_edit),
            ("copy", &mut request.copy),
            ("pin", &mut request.pin),
            ("upload", &mut request.upload),
            ("raw", &mut request.raw),
            ("print_geometry", &mut request.print_geometry),
            ("hide_cursor", &mut request.hide_cursor),
            ("last_region", &mut request.last_region),
        ] {
            if let Some(value) = options.get(key) {
                *slot = bool_value(key, value)?;
            }
        }
        if let Some(value) = options.get("output") {
            request.output = Some(string_value("output", value)?);
        }
        if let Some(value) = options.get("region") {
            request.region = Some(string_value("region", value)?);
        }
        Ok(request)
    }
}

fn bool_value(key: &str, value: &OwnedValue) -> Result<bool, DaemonError> {
    match &**value {
        Value::Bool(flag) => Ok(*flag),
        other => Err(invalid_type(key, "boolean", other)),
    }
}

fn u32_value(key: &str, value: &OwnedValue) -> Result<u32, DaemonError> {
    match &**value {
        Value::U32(number) => Ok(*number),
        Value::U64(number) => {
            u32::try_from(*number).map_err(|_| invalid(key, "u32", "out-of-range u64"))
        }
        other => Err(invalid_type(key, "u32", other)),
    }
}

fn string_value(key: &str, value: &OwnedValue) -> Result<String, DaemonError> {
    match &**value {
        Value::Str(text) => Ok(text.to_string()),
        other => Err(invalid_type(key, "string", other)),
    }
}

fn invalid_type(key: &str, expected: &str, value: &Value<'_>) -> DaemonError {
    invalid(key, expected, &format!("got {}", value.value_signature()))
}

fn invalid(key: &str, expected: &str, got: &str) -> DaemonError {
    DaemonError::InvalidArgs(format!("capture option `{key}` expects {expected}, {got}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Array, Str, Value as ZValue};

    fn dict(entries: Vec<(&str, OwnedValue)>) -> HashMap<String, OwnedValue> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect()
    }

    fn owned(value: ZValue<'static>) -> OwnedValue {
        OwnedValue::try_from(value).unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn empty_vardict_yields_defaults() {
        let request =
            CaptureRequest::from_vardict(&HashMap::new()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(request, CaptureRequest::default());
    }

    #[test]
    fn full_vardict_parses_every_key() {
        let request = CaptureRequest::from_vardict(&dict(vec![
            ("delay_ms", owned(ZValue::U32(250))),
            ("instant", owned(ZValue::Bool(true))),
            ("no_edit", owned(ZValue::Bool(true))),
            ("copy", owned(ZValue::Bool(true))),
            ("output", owned(ZValue::Str(Str::from("/tmp/shot.png")))),
            ("pin", owned(ZValue::Bool(true))),
            ("upload", owned(ZValue::Bool(false))),
            ("raw", owned(ZValue::Bool(true))),
            ("print_geometry", owned(ZValue::Bool(true))),
            ("hide_cursor", owned(ZValue::Bool(true))),
            ("region", owned(ZValue::Str(Str::from("640x480+10+20")))),
            ("last_region", owned(ZValue::Bool(true))),
        ]))
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            request,
            CaptureRequest {
                delay_ms: 250,
                instant: true,
                no_edit: true,
                copy: true,
                output: Some("/tmp/shot.png".to_owned()),
                pin: true,
                upload: false,
                raw: true,
                print_geometry: true,
                hide_cursor: true,
                region: Some("640x480+10+20".to_owned()),
                last_region: true,
            }
        );
    }

    #[test]
    fn u64_delay_narrows_and_out_of_range_is_typed() {
        let narrowed =
            CaptureRequest::from_vardict(&dict(vec![("delay_ms", owned(ZValue::U64(7)))]))
                .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(narrowed.delay_ms, 7);
        let overflow = CaptureRequest::from_vardict(&dict(vec![(
            "delay_ms",
            owned(ZValue::U64(u64::from(u32::MAX) + 1)),
        )]));
        assert!(matches!(overflow, Err(DaemonError::InvalidArgs(_))));
    }

    #[test]
    fn wrong_type_is_a_typed_error_naming_the_key() {
        let result = CaptureRequest::from_vardict(&dict(vec![("copy", owned(ZValue::U32(1)))]));
        match result {
            Err(DaemonError::InvalidArgs(message)) => {
                assert!(message.contains("copy"), "message: {message}");
                assert!(message.contains("boolean"), "message: {message}");
            }
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let request = CaptureRequest::from_vardict(&dict(vec![
            ("from_the_future", owned(ZValue::U32(1))),
            ("copy", owned(ZValue::Bool(true))),
        ]))
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(request.copy);
    }

    #[test]
    fn array_valued_key_is_rejected_with_signature_detail() {
        let result = CaptureRequest::from_vardict(&dict(vec![(
            "region",
            owned(ZValue::Array(Array::from(vec![ZValue::U32(1)]))),
        )]));
        match result {
            Err(DaemonError::InvalidArgs(message)) => assert!(message.contains("region")),
            other => panic!("expected InvalidArgs, got {other:?}"),
        }
    }
}

//! The `D-Bus` wire contract: the CLI's vardict serializer must round-trip
//! through the DAEMON's parse boundary (`CaptureRequest::from_vardict`) and
//! stay inside the frozen `CAPTURE_OPTION_KEYS` vocabulary; the call mapping
//! must pick the lossless typed member or fall back to `Invoke(argv)`.

use std::collections::HashMap;

use flowshot_cli::invocation::{CaptureInvocation, CaptureSelection, ScreenSpec};
use flowshot_cli::wire::{WireCall, capture_call, capture_vardict};
use flowshot_daemon::CaptureRequest;
use flowshot_daemon::request::CAPTURE_OPTION_KEYS;
use proptest::prelude::*;
use zbus::zvariant::{OwnedValue, Value};

fn owned_map(vardict: HashMap<String, Value<'static>>) -> HashMap<String, OwnedValue> {
    vardict
        .into_iter()
        .map(|(key, value)| {
            let owned = OwnedValue::try_from(value).unwrap_or_else(|error| panic!("{error}"));
            (key, owned)
        })
        .collect()
}

fn round_trip(request: &CaptureRequest) -> CaptureRequest {
    let parsed = CaptureRequest::from_vardict(&owned_map(capture_vardict(request)));
    parsed.unwrap_or_else(|error| panic!("{error}"))
}

fn full_request() -> CaptureRequest {
    CaptureRequest {
        delay_ms: 250,
        instant: true,
        no_edit: true,
        copy: true,
        output: Some("/tmp/shot.png".to_owned()),
        pin: true,
        upload: true,
        raw: true,
        print_geometry: true,
        hide_cursor: true,
        region: Some("640x480+10+20".to_owned()),
        last_region: true,
    }
}

#[test]
fn full_request_roundtrips_through_the_daemon_parse_boundary() {
    // Given: every modifier set. When: serialized to the wire vocabulary and
    // parsed back by the daemon's boundary. Then: byte-equal typed request.
    assert_eq!(round_trip(&full_request()), full_request());
}

#[test]
fn default_request_serializes_to_an_empty_vardict_and_back() {
    let vardict = capture_vardict(&CaptureRequest::default());
    assert!(vardict.is_empty(), "lean encoding must omit defaults");
    assert_eq!(
        round_trip(&CaptureRequest::default()),
        CaptureRequest::default()
    );
}

#[test]
fn every_emitted_key_is_in_the_frozen_vocabulary() {
    let vardict = capture_vardict(&full_request());
    assert_eq!(
        vardict.len(),
        CAPTURE_OPTION_KEYS.len(),
        "a fully-set request must exercise every vocabulary key"
    );
    for key in vardict.keys() {
        assert!(
            CAPTURE_OPTION_KEYS.contains(&key.as_str()),
            "`{key}` is outside CAPTURE_OPTION_KEYS"
        );
    }
}

#[test]
fn single_modifier_serializes_lean() {
    let request = CaptureRequest {
        copy: true,
        ..CaptureRequest::default()
    };
    let vardict = capture_vardict(&request);
    assert_eq!(vardict.keys().collect::<Vec<_>>(), vec!["copy"]);
    assert_eq!(round_trip(&request), request);
}

fn any_capture_request() -> impl Strategy<Value = CaptureRequest> {
    (
        any::<u32>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<Option<String>>(),
        any::<Option<String>>(),
    )
        .prop_map(
            |(
                delay_ms,
                instant,
                no_edit,
                copy,
                pin,
                upload,
                raw,
                print_geometry,
                hide_cursor,
                last_region,
                output,
                region,
            )| {
                CaptureRequest {
                    delay_ms,
                    instant,
                    no_edit,
                    copy,
                    output,
                    pin,
                    upload,
                    raw,
                    print_geometry,
                    hide_cursor,
                    region,
                    last_region,
                }
            },
        )
}

proptest! {
    #[test]
    fn vardict_roundtrip_is_lossless_for_any_request(request in any_capture_request()) {
        prop_assert_eq!(round_trip(&request), request);
    }
}

fn invocation(selection: CaptureSelection, request: CaptureRequest) -> CaptureInvocation {
    CaptureInvocation {
        selection,
        request,
        one_shot: false,
    }
}

const ARGV: [&str; 3] = ["capture", "full", "-c"];

fn argv() -> Vec<String> {
    ARGV.iter().map(|token| (*token).to_owned()).collect()
}

#[test]
fn interactive_captures_use_the_typed_vardict_member() {
    let call = capture_call(
        &invocation(CaptureSelection::Interactive, CaptureRequest::default()),
        &argv(),
    );
    assert_eq!(call.member(), "Capture");
    assert_eq!(call, WireCall::Capture(HashMap::new()));

    let with_region = capture_call(
        &invocation(
            CaptureSelection::Interactive,
            CaptureRequest {
                region: Some("at-cursor".to_owned()),
                copy: true,
                ..CaptureRequest::default()
            },
        ),
        &argv(),
    );
    match with_region {
        WireCall::Capture(options) => {
            assert!(options.contains_key("region"));
            assert!(options.contains_key("copy"));
        }
        other => panic!("expected the typed Capture member, got {other:?}"),
    }
}

#[test]
fn modifier_free_full_and_indexed_screen_use_their_typed_members() {
    assert_eq!(
        capture_call(
            &invocation(CaptureSelection::Full, CaptureRequest::default()),
            &argv()
        ),
        WireCall::CaptureFull
    );
    assert_eq!(
        capture_call(
            &invocation(
                CaptureSelection::Screen(ScreenSpec::Index(2)),
                CaptureRequest::default()
            ),
            &argv()
        ),
        WireCall::CaptureScreen(2)
    );
}

#[test]
fn forms_the_typed_wire_cannot_carry_fall_back_to_invoke() {
    let modified = CaptureRequest {
        copy: true,
        ..CaptureRequest::default()
    };
    for invocation in [
        invocation(CaptureSelection::Full, modified.clone()),
        invocation(
            CaptureSelection::Screen(ScreenSpec::Index(1)),
            modified.clone(),
        ),
        invocation(
            CaptureSelection::Screen(ScreenSpec::Cursor),
            modified.clone(),
        ),
        invocation(
            CaptureSelection::Screen(ScreenSpec::Connector("DP-1".to_owned())),
            CaptureRequest::default(),
        ),
    ] {
        assert_eq!(
            capture_call(&invocation, &argv()),
            WireCall::Invoke(argv()),
            "{invocation:?} must ride the lossless argv channel"
        );
    }
}

#[test]
fn wire_call_member_tokens_are_stable() {
    assert_eq!(WireCall::CaptureFull.member(), "CaptureFull");
    assert_eq!(WireCall::CaptureScreen(1).member(), "CaptureScreen");
    assert_eq!(WireCall::Launcher.member(), "Launcher");
    assert_eq!(WireCall::Settings.member(), "Settings");
    assert_eq!(WireCall::Invoke(Vec::new()).member(), "Invoke");
}

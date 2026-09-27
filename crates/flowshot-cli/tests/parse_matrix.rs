//! The clap parse matrix (plan todo 35 QA: every subcommand + flag, the
//! grammar rejections, and the conflict rules). Structural assertions on
//! the typed resolution - no prose pinning.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use clap::Parser;
use clap::error::ErrorKind;
use flowshot_cli::args::{Cli, CompletionShell};
use flowshot_cli::exit::CliError;
use flowshot_cli::invocation::{
    CaptureInvocation, CaptureSelection, DaemonRun, Invocation, Resolved, ScreenSpec, argv_tail,
    resolve, validate_upload,
};
use flowshot_daemon::CaptureRequest;

fn cli(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("flowshot").chain(args.iter().copied()))
        .unwrap_or_else(|error| panic!("{error}"))
}

fn resolved(args: &[&str]) -> Invocation {
    match resolve(cli(args)) {
        Ok(Resolved::Command(invocation)) => invocation,
        Ok(Resolved::Session(spec)) => panic!("unexpected session verb ({})", spec.display()),
        Err(error) => panic!("{error}"),
    }
}

fn resolve_error(args: &[&str]) -> CliError {
    resolve(cli(args))
        .err()
        .unwrap_or_else(|| panic!("expected a usage error for {args:?}"))
}

fn clap_error(args: &[&str]) -> clap::Error {
    Cli::try_parse_from(std::iter::once("flowshot").chain(args.iter().copied()))
        .err()
        .unwrap_or_else(|| panic!("expected a clap rejection for {args:?}"))
}

fn capture_of(args: &[&str]) -> CaptureInvocation {
    match resolved(args) {
        Invocation::Capture(capture) => capture,
        other => panic!("expected a capture invocation, got {other:?}"),
    }
}

#[test]
fn bare_flowshot_is_the_default_capture() {
    // Given: no arguments at all. When: resolved. Then: interactive region
    // capture with the default modifier bag, daemon-forwarded.
    assert_eq!(
        resolved(&[]),
        Invocation::Capture(CaptureInvocation {
            selection: CaptureSelection::Interactive,
            request: CaptureRequest::default(),
            one_shot: false,
        })
    );
    assert_eq!(resolved(&["capture"]), resolved(&[]));
    assert_eq!(resolved(&["capture", "region"]), resolved(&[]));
}

#[test]
fn full_target_positional_and_flag_are_equivalent() {
    for args in [["capture", "full"], ["capture", "--full"]] {
        assert_eq!(capture_of(&args).selection, CaptureSelection::Full);
    }
}

#[test]
fn screen_target_forms_resolve_to_typed_specs() {
    assert_eq!(
        capture_of(&["capture", "screen"]).selection,
        CaptureSelection::Screen(ScreenSpec::Cursor)
    );
    assert_eq!(
        capture_of(&["capture", "screen", "2"]).selection,
        CaptureSelection::Screen(ScreenSpec::Index(2))
    );
    assert_eq!(
        capture_of(&["capture", "screen", "DP-1"]).selection,
        CaptureSelection::Screen(ScreenSpec::Connector("DP-1".to_owned()))
    );
    assert_eq!(
        capture_of(&["capture", "--screen", "0"]).selection,
        CaptureSelection::Screen(ScreenSpec::Index(0))
    );
    assert_eq!(
        capture_of(&["capture", "--screen", "HDMI-A-2"]).selection,
        CaptureSelection::Screen(ScreenSpec::Connector("HDMI-A-2".to_owned()))
    );
}

#[test]
fn last_target_forms_set_the_last_region_request_key() {
    for args in [["capture", "last"], ["capture", "--last-region"]] {
        let capture = capture_of(&args);
        assert!(capture.request.last_region, "{args:?} must set last_region");
        assert_eq!(capture.selection, CaptureSelection::Interactive);
    }
}

#[test]
fn every_modifier_lands_on_the_typed_request() {
    let capture = capture_of(&[
        "capture",
        "-d",
        "250",
        "--instant",
        "--no-edit",
        "-c",
        "-o",
        "/tmp/shot.png",
        "--pin",
        "--upload",
        "--raw",
        "--print-geometry",
        "--hide-cursor",
        "--region",
        "640x480+10+20",
    ]);
    assert_eq!(
        capture.request,
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
            last_region: false,
        }
    );
}

#[test]
fn stdout_flags_and_no_daemon_force_one_shot() {
    for args in [
        ["capture", "--raw"].as_slice(),
        ["capture", "--print-geometry"].as_slice(),
        ["capture", "--no-daemon"].as_slice(),
        ["capture", "full", "--raw"].as_slice(),
    ] {
        assert!(capture_of(args).one_shot, "{args:?} must be one-shot");
    }
    assert!(!capture_of(&["capture", "-c"]).one_shot);
}

#[test]
fn valid_region_grammar_tokens_are_accepted() {
    for token in [
        "at-cursor",
        "1x1",
        "640x480",
        "640x480+10+20",
        "640x480-10-20",
        "640x480+10-20",
        "3840x2160+0+0",
    ] {
        let capture = capture_of(&["capture", "--region", token]);
        assert_eq!(capture.request.region.as_deref(), Some(token));
    }
}

#[test]
fn malformed_region_tokens_are_usage_errors() {
    for token in [
        "",
        "garbage",
        "640",
        "640x",
        "x480",
        "640xx480",
        "640x480+",
        "640x480+10",
        "640x480+10+",
        "640x480++10+20",
        "0x480",
        "640x0",
        "4294967296x480",
        "640x480+2147483648+0",
        "640X480",
        "at_cursor",
    ] {
        match resolve_error(&["capture", &format!("--region={token}")]) {
            CliError::Usage(message) => assert!(message.contains("--region"), "{message}"),
            other => panic!("token {token:?}: expected Usage, got {other:?}"),
        }
    }
    // A leading-dash token is rejected even earlier (clap flag syntax);
    // the `=` form proves the grammar layer rejects it too.
    match resolve_error(&["capture", "--region=-640x480"]) {
        CliError::Usage(_) => {}
        other => panic!("expected Usage, got {other:?}"),
    }
}

#[test]
fn screen_spec_without_the_screen_target_is_a_usage_error() {
    for args in [["capture", "region", "1"], ["capture", "full", "DP-1"]] {
        assert!(
            matches!(resolve_error(&args), CliError::Usage(_)),
            "{args:?} must be rejected"
        );
    }
}

#[test]
fn invalid_screen_selectors_are_usage_errors() {
    for spec in ["", "a b", "4294967296", "99999999999999999999"] {
        assert!(
            matches!(
                resolve_error(&["capture", "screen", spec]),
                CliError::Usage(_)
            ),
            "spec {spec:?} must be rejected"
        );
    }
}

#[test]
fn conflicting_target_selectors_are_clap_rejections() {
    for args in [
        ["capture", "full", "--full"].as_slice(),
        ["capture", "region", "--region", "1x1"].as_slice(),
        ["capture", "--full", "--region", "1x1"].as_slice(),
        ["capture", "--full", "--last-region"].as_slice(),
        ["capture", "--region", "1x1", "--last-region"].as_slice(),
        ["capture", "--screen", "1", "--full"].as_slice(),
        ["capture", "screen", "--screen", "1"].as_slice(),
        ["capture", "last", "--full"].as_slice(),
    ] {
        let error = clap_error(args);
        assert_eq!(
            error.kind(),
            ErrorKind::ArgumentConflict,
            "{args:?}: {error}"
        );
        assert!(error.use_stderr(), "{args:?} must report on stderr");
    }
}

#[test]
fn dialog_is_standalone() {
    assert_eq!(
        resolved(&["capture", "--dialog"]),
        Invocation::Launcher { one_shot: false }
    );
    assert_eq!(
        resolved(&["capture", "--dialog", "--no-daemon"]),
        Invocation::Launcher { one_shot: true }
    );
    for args in [
        ["capture", "--dialog", "-c"].as_slice(),
        ["capture", "--dialog", "-d", "10"].as_slice(),
        ["capture", "--dialog", "full"].as_slice(),
        ["capture", "--dialog", "--raw"].as_slice(),
        ["capture", "--dialog", "screen", "1"].as_slice(),
    ] {
        assert_eq!(
            clap_error(args).kind(),
            ErrorKind::ArgumentConflict,
            "{args:?} must conflict"
        );
    }
}

#[test]
fn pin_color_settings_and_alias_resolve() {
    assert_eq!(resolved(&["pin"]), Invocation::Pin(None));
    assert_eq!(
        resolved(&["pin", "/tmp/x.png"]),
        Invocation::Pin(Some(PathBuf::from("/tmp/x.png")))
    );
    assert_eq!(resolved(&["color"]), Invocation::Color);
    assert_eq!(resolved(&["settings"]), Invocation::Settings);
    assert_eq!(resolved(&["config"]), Invocation::Settings);
}

#[test]
fn session_verb_resolves_outside_the_runtime_command_set() {
    // Given: the hidden session-child verb. When: resolved. Then: it lands
    // in the pre-runtime Session lane, never in the dispatched Invocation
    // set (the nested-runtime panic guard - main runs it before building
    // the tokio runtime).
    assert!(matches!(
        resolve(cli(&["session", "--spec", "/tmp/spec.json"])),
            Ok(Resolved::Session(spec)) if spec.as_path() == Path::new("/tmp/spec.json")
    ));
}

#[test]
fn daemon_subcommand_mirrors_the_helper_binary() {
    assert_eq!(
        resolved(&["daemon"]),
        Invocation::Daemon(DaemonRun {
            auto_spawned: false,
            idle_grace_secs: 60,
            config: None,
        })
    );
    assert_eq!(
        resolved(&[
            "daemon",
            "--auto-spawned",
            "--idle-grace",
            "30",
            "--config",
            "/tmp/f.toml"
        ]),
        Invocation::Daemon(DaemonRun {
            auto_spawned: true,
            idle_grace_secs: 30,
            config: Some(PathBuf::from("/tmp/f.toml")),
        })
    );
}

#[test]
fn completions_accepts_the_six_shells() {
    for (token, shell) in [
        ("bash", CompletionShell::Bash),
        ("zsh", CompletionShell::Zsh),
        ("fish", CompletionShell::Fish),
        ("elvish", CompletionShell::Elvish),
        ("pwsh", CompletionShell::Pwsh),
        ("nushell", CompletionShell::Nushell),
    ] {
        assert_eq!(
            resolved(&["completions", token]),
            Invocation::Completions(shell)
        );
    }
    assert_eq!(
        clap_error(&["completions", "csh"]).kind(),
        ErrorKind::InvalidValue
    );
    assert_eq!(
        clap_error(&["completions"]).kind(),
        ErrorKind::MissingRequiredArgument
    );
}

#[test]
fn print_bind_help_overrides_any_subcommand() {
    assert_eq!(resolved(&["--print-bind-help"]), Invocation::PrintBindHelp);
    assert_eq!(
        resolved(&["capture", "--print-bind-help"]),
        Invocation::PrintBindHelp
    );
    assert_eq!(
        resolved(&["completions", "bash", "--print-bind-help"]),
        Invocation::PrintBindHelp
    );
}

#[test]
fn bus_address_is_a_global_knob() {
    let leading = cli(&["--bus-address", "unix:path=/tmp/bus", "capture", "-c"]);
    assert_eq!(leading.bus_address.as_deref(), Some("unix:path=/tmp/bus"));
    let nested = cli(&["capture", "-c", "--bus-address", "unix:path=/tmp/bus"]);
    assert_eq!(nested.bus_address.as_deref(), Some("unix:path=/tmp/bus"));
}

#[test]
fn unknown_flags_and_bad_values_are_clap_rejections_on_stderr() {
    let unknown = clap_error(&["capture", "--frobnicate"]);
    assert!(unknown.use_stderr());
    assert_eq!(
        clap_error(&["capture", "-d", "4294967296"]).kind(),
        ErrorKind::ValueValidation
    );
    assert!(clap_error(&["capture", "-d", "-5"]).use_stderr());
    assert!(clap_error(&["nonsense"]).use_stderr());
}

#[test]
fn argv_tail_skips_the_program_name_and_rejects_non_unicode() {
    let argv = vec![OsString::from("flowshot"), OsString::from("capture")];
    assert_eq!(
        argv_tail(&argv).unwrap_or_else(|error| panic!("{error}")),
        vec!["capture".to_owned()]
    );
    let broken = vec![OsString::from("flowshot"), OsString::from_vec(vec![0xff])];
    assert!(matches!(
        argv_tail(&broken),
        Err(CliError::NonUnicodeArg(_))
    ));
}

#[test]
fn upload_without_a_client_id_is_a_usage_error_with_a_settings_hint() {
    let mut config = flowshot_core::Config::default();
    let request = CaptureRequest {
        upload: true,
        ..CaptureRequest::default()
    };
    match validate_upload(&request, &config) {
        Err(CliError::Usage(message)) => {
            assert!(message.contains("upload.client_id"), "{message}");
            assert!(message.contains("flowshot settings"), "{message}");
        }
        other => panic!("expected the unconfigured-upload rejection, got {other:?}"),
    }
    config.upload.client_id = "abc123".to_owned();
    validate_upload(&request, &config).unwrap_or_else(|error| panic!("{error}"));
    validate_upload(
        &CaptureRequest::default(),
        &flowshot_core::Config::default(),
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

//! Structural help-surface checks (acceptance: `flowshot
//! --help` + every subcommand help renders). Token-presence assertions on
//! the machine-consumed surface (subcommand names, flag spellings) - never
//! prose pinning.

use clap::CommandFactory;
use clap::Parser;
use clap::error::ErrorKind;
use flowshot_cli::args::Cli;

fn render_help(args: &[&str]) -> String {
    let error = Cli::try_parse_from(std::iter::once("flowshot").chain(args.iter().copied()))
        .err()
        .unwrap_or_else(|| panic!("{args:?} must produce the help rendering"));
    assert!(!error.use_stderr(), "help renders on stdout");
    error.to_string()
}

#[test]
fn top_level_help_lists_the_whole_authoritative_surface() {
    let help = render_help(&["--help"]);
    for token in [
        "capture",
        "pin",
        "color",
        "settings",
        "config",
        "daemon",
        "completions",
        "--print-bind-help",
        "--bus-address",
        "--version",
        "--help",
    ] {
        assert!(help.contains(token), "top-level help misses {token}");
    }
}

#[test]
fn capture_help_lists_every_flameshot_modifier() {
    let help = render_help(&["capture", "--help"]);
    for token in [
        "--full",
        "--screen",
        "--region",
        "--last-region",
        "-d",
        "--delay",
        "--instant",
        "--no-edit",
        "-c",
        "--copy",
        "-o",
        "--output",
        "--pin",
        "--upload",
        "--raw",
        "--print-geometry",
        "--hide-cursor",
        "--dialog",
        "--no-daemon",
        "region",
        "full",
        "screen",
        "last",
    ] {
        assert!(help.contains(token), "capture help misses {token}");
    }
}

#[test]
fn every_subcommand_help_renders() {
    for args in [
        ["capture", "--help"],
        ["pin", "--help"],
        ["color", "--help"],
        ["settings", "--help"],
        ["daemon", "--help"],
        ["completions", "--help"],
    ] {
        let help = render_help(&args);
        assert!(!help.is_empty(), "{args:?} rendered nothing");
    }
}

#[test]
fn help_and_version_exit_through_the_success_class() {
    for args in [["--help"], ["-h"], ["--version"], ["-V"]] {
        let error = Cli::try_parse_from(std::iter::once("flowshot").chain(args.iter().copied()))
            .err()
            .unwrap_or_else(|| panic!("{args:?} must short-circuit"));
        assert!(!error.use_stderr(), "{args:?} renders on stdout (exit 0)");
        assert!(
            error.kind() == ErrorKind::DisplayHelp || error.kind() == ErrorKind::DisplayVersion,
            "{args:?}: unexpected kind {:?}",
            error.kind()
        );
    }
}

#[test]
fn the_derived_command_carries_the_binary_name_for_generators() {
    // Completions and man pages derive their file/bin names from this.
    let command = Cli::command();
    assert_eq!(command.get_name(), "flowshot");
    let capture = command
        .get_subcommands()
        .find(|sub| sub.get_name() == "capture")
        .unwrap_or_else(|| panic!("the capture subcommand must exist"));
    assert!(
        capture.get_arguments().count() > 15,
        "the Flameshot modifier set must be complete"
    );
}

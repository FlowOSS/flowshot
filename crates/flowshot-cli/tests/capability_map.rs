//! The capability-coverage gate (replaces the Flameshot
//! parity-diff). Every row of `cli_capability_map.toml` must be either
//! MAPPED - with argv that parses through the real clap surface - or
//! explicitly DROPPED with a reason. Zero unmapped rows. The parity
//! matrix references these row ids (single source of truth).

use std::collections::HashSet;

use clap::Parser;
use clap::error::ErrorKind;
use flowshot_cli::args::Cli;
use flowshot_cli::exit::legacy_hint;
use serde::Deserialize;

#[derive(Deserialize)]
struct Map {
    capability: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    id: String,
    flameshot: String,
    disposition: String,
    argv: Option<Vec<String>>,
    reason: Option<String>,
}

fn load_map() -> Map {
    let text = include_str!("cli_capability_map.toml");
    toml::from_str(text).unwrap_or_else(|error| panic!("capability map must parse: {error}"))
}

#[test]
fn every_capability_row_is_mapped_or_explicitly_dropped() {
    let map = load_map();
    assert!(
        map.capability.len() >= 30,
        "the Flameshot CLI inventory (5 verbs + 15 flags + modes) must be fully enumerated, got {}",
        map.capability.len()
    );
    let mut ids = HashSet::new();
    for row in &map.capability {
        assert!(ids.insert(&row.id), "duplicate row id {}", row.id);
        assert!(
            !row.flameshot.is_empty(),
            "{}: missing flameshot token",
            row.id
        );
        match row.disposition.as_str() {
            "mapped" => {
                let argv = row
                    .argv
                    .as_ref()
                    .unwrap_or_else(|| panic!("{}: mapped without argv", row.id));
                assert!(!argv.is_empty(), "{}: empty argv", row.id);
                let parsed = Cli::try_parse_from(
                    std::iter::once("flowshot".to_owned()).chain(argv.iter().cloned()),
                );
                assert!(
                    parsed.is_ok(),
                    "{}: argv {argv:?} does not parse the surface: {}",
                    row.id,
                    parsed
                        .err()
                        .map_or_else(String::new, |error| error.to_string())
                );
            }
            "dropped" => {
                let reason = row
                    .reason
                    .as_ref()
                    .unwrap_or_else(|| panic!("{}: dropped without a reason", row.id));
                assert!(
                    reason.len() > 10,
                    "{}: reason is not a real explanation",
                    row.id
                );
            }
            other => panic!(
                "{}: unmapped disposition {other:?} (zero unmapped rows allowed)",
                row.id
            ),
        }
    }
}

#[test]
fn the_plan_mandated_drops_are_present() {
    let map = load_map();
    let dropped: HashSet<&str> = map
        .capability
        .iter()
        .filter(|row| row.disposition == "dropped")
        .map(|row| row.id.as_str())
        .collect();
    // DROPPED(reason) entries for config-mutating flags, the
    // gui verb (interface), and the -e inversion.
    for required in [
        "flag-config-autostart",
        "flag-config-notifications",
        "flag-config-filename",
        "flag-config-trayicon",
        "flag-config-showhelp",
        "flag-config-maincolor",
        "flag-config-contrastcolor",
        "flag-config-check",
        "flag-edit-inversion",
    ] {
        assert!(
            dropped.contains(required),
            "{required} must be a dropped row"
        );
    }
}

#[test]
fn legacy_flameshot_verbs_are_rejected_with_did_you_mean_hints() {
    for verb in ["gui", "launcher", "screen"] {
        let error = Cli::try_parse_from(["flowshot", verb])
            .err()
            .unwrap_or_else(|| panic!("`{verb}` must be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidSubcommand, "{verb}");
        assert!(error.use_stderr(), "{verb} must report on stderr (exit 2)");
        let hint = legacy_hint(verb).unwrap_or_else(|| panic!("{verb} must carry a hint"));
        assert!(hint.contains("flowshot capture"), "{verb}: {hint}");
    }
}

#[test]
fn the_new_surface_verbs_are_registered() {
    for verb in ["capture", "pin", "color", "settings", "config", "daemon"] {
        let result = Cli::try_parse_from(["flowshot", verb]);
        assert!(result.is_ok(), "{verb} must parse: {:?}", result.err());
    }
    // `completions` is registered but requires its shell argument; a
    // missing-argument rejection (not InvalidSubcommand) proves it.
    let error = Cli::try_parse_from(["flowshot", "completions"])
        .err()
        .unwrap_or_else(|| panic!("completions requires a shell argument"));
    assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
}

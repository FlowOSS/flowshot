//! The parity-matrix validator (plan todo 38, Metis #18): the root
//! `tests/parity_matrix.toml` must enumerate every draft-F12 item with
//! exactly one disposition - implemented+verified, OUT(reason), or a CLI
//! `capability_row` REFERENCE (Momus r4 SIMPLIFY: the todo-35
//! `cli_capability_map.toml` stays the single source of truth for CLI
//! dispositions). ZERO unmapped rows in either direction, the
//! Amendment-#3 drop list complete (the todo-2 acceptance: "todo 38
//! validator consumes it"), the full TYPE_* 0..24 enum present, and every
//! path-shaped `verified_by` entry existing on disk. Runs in CI via
//! `cargo test --workspace`.

#![allow(clippy::unwrap_used)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
struct Matrix {
    meta: Meta,
    row: Vec<Row>,
}

#[derive(Deserialize)]
struct Meta {
    plan: String,
    draft: String,
    cli_capability_map: String,
}

#[derive(Deserialize)]
struct Row {
    id: String,
    domain: String,
    item: String,
    #[serde(default)]
    implemented_by: Vec<u32>,
    #[serde(default)]
    verified_by: Vec<String>,
    #[serde(default)]
    out: Option<String>,
    #[serde(default, rename = "capability_row")]
    capability_ref: Option<String>,
}

#[derive(Deserialize)]
struct CapabilityMap {
    capability: Vec<CapabilityRow>,
}

#[derive(Deserialize)]
struct CapabilityRow {
    id: String,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn load_matrix() -> Matrix {
    let path = repo_root().join("tests").join("parity_matrix.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} unreadable: {error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("{} invalid: {error}", path.display()))
}

/// The canonical Amendment-#3 drop list (plan todo 2). Every key must be
/// an OUT row carrying the DROPPED(user-amendment-3) token.
const AMENDMENT3_DROPS: [&str; 15] = [
    "showQuitPrompt",
    "allowMultipleGuiInstances",
    "antialiasingPinZoom",
    "insecurePixelate",
    "captureActiveMonitor",
    "autoCloseIdleDaemon",
    "showHelp",
    "showDesktopNotification",
    "showAbortNotification",
    "predefinedColorPaletteLarge",
    "keepOpenAppLauncher",
    "useX11LegacyScreenshot",
    "uiLanguage",
    "historyConfirmationToDelete",
    "uploadClientSecret",
];

/// The full Flameshot capturetool.h TYPE_* enum (0..24, 7 unassigned
/// upstream, 13 ENABLE_IMGUR-gated upstream but always-on here).
const TYPE_ROWS: [&str; 24] = [
    "type-00-pencil",
    "type-01-drawer",
    "type-02-arrow",
    "type-03-selection",
    "type-04-rectangle",
    "type-05-circle",
    "type-06-marker",
    "type-08-moveselection",
    "type-09-undo",
    "type-10-copy",
    "type-11-save",
    "type-12-exit",
    "type-13-imageuploader",
    "type-14-open-app",
    "type-15-pixelate",
    "type-16-redo",
    "type-17-pin",
    "type-18-text",
    "type-19-circlecount",
    "type-20-sizeincrease",
    "type-21-sizedecrease",
    "type-22-invert",
    "type-23-accept",
    "type-24-cancel",
];

#[test]
fn every_row_carries_exactly_one_disposition() {
    let matrix = load_matrix();
    assert!(!matrix.row.is_empty(), "the matrix has no rows");
    let mut seen = HashSet::new();
    for row in &matrix.row {
        assert!(seen.insert(&row.id), "duplicate row id {:?}", row.id);
        assert!(!row.domain.is_empty(), "{:?}: empty domain", row.id);
        assert!(!row.item.is_empty(), "{:?}: empty item", row.id);
        let implemented = !row.implemented_by.is_empty();
        let verified = !row.verified_by.is_empty();
        let out = row.out.as_ref().is_some_and(|reason| !reason.is_empty());
        let reference = row
            .capability_ref
            .as_ref()
            .is_some_and(|reference| !reference.is_empty());
        let shapes = [
            implemented && verified && !out && !reference,
            out && !implemented && !verified && !reference,
            reference && !implemented && !verified && !out,
        ];
        assert_eq!(
            shapes.iter().filter(|valid| **valid).count(),
            1,
            "{:?}: exactly one disposition shape required (implemented_by+verified_by | out | capability_row), got \
             implemented={implemented} verified={verified} out={out} reference={reference}",
            row.id
        );
        if reference {
            assert_eq!(
                row.domain, "cli",
                "{:?}: capability_row is CLI-only",
                row.id
            );
        }
    }
}

#[test]
fn cli_rows_reference_the_capability_map_both_ways() {
    let matrix = load_matrix();
    let map_path = repo_root().join(&matrix.meta.cli_capability_map);
    let map: CapabilityMap = toml::from_str(
        &std::fs::read_to_string(&map_path)
            .unwrap_or_else(|error| panic!("{} unreadable: {error}", map_path.display())),
    )
    .unwrap();
    let map_ids: HashSet<String> = map.capability.iter().map(|row| row.id.clone()).collect();
    let referenced: HashSet<String> = matrix
        .row
        .iter()
        .filter_map(|row| row.capability_ref.clone())
        .collect();
    let dangling: Vec<&String> = referenced.difference(&map_ids).collect();
    assert!(
        dangling.is_empty(),
        "matrix references capability-map ids that do not exist: {dangling:?}"
    );
    let unmapped: Vec<&String> = map_ids.difference(&referenced).collect();
    assert!(
        unmapped.is_empty(),
        "capability-map rows with NO matrix reference (zero-unmapped rule): {unmapped:?}"
    );
}

#[test]
fn amendment3_drops_are_out_rows_with_the_recorded_token() {
    let matrix = load_matrix();
    let by_id: HashMap<&str, &Row> = matrix
        .row
        .iter()
        .map(|row| (row.id.as_str(), row))
        .collect();
    for key in AMENDMENT3_DROPS {
        let id = format!("ini-{key}");
        let row = by_id.get(id.as_str()).unwrap_or_else(|| {
            panic!("Amendment-#3 drop {key:?} has no {id:?} row (todo-2 acceptance)");
        });
        assert_eq!(row.domain, "ini-key", "{id:?}: wrong domain");
        let out = row
            .out
            .as_deref()
            .unwrap_or_else(|| panic!("drop {key:?} is not an OUT row"));
        assert!(
            out.starts_with("DROPPED(user-amendment-3)"),
            "drop {key:?} carries the wrong OUT token: {out:?}"
        );
    }
}

#[test]
fn the_full_type_enum_is_enumerated() {
    let matrix = load_matrix();
    let ids: HashSet<&str> = matrix.row.iter().map(|row| row.id.as_str()).collect();
    for expected in TYPE_ROWS {
        assert!(ids.contains(expected), "missing TYPE_* row {expected:?}");
    }
}

#[test]
fn verified_by_paths_exist_and_meta_points_at_real_files() {
    let matrix = load_matrix();
    let root = repo_root();
    for relative in [
        &matrix.meta.plan,
        &matrix.meta.draft,
        &matrix.meta.cli_capability_map,
    ] {
        assert!(
            root.join(relative).exists(),
            "meta path does not exist: {relative}"
        );
    }
    for row in &matrix.row {
        for entry in &row.verified_by {
            if entry.starts_with("test:") {
                continue;
            }
            assert!(
                root.join(entry).exists(),
                "row {:?}: verified_by path does not exist: {entry}",
                row.id
            );
        }
    }
}

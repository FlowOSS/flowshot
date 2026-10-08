//! Doc-sync test: every config field in the schema appears in
//! `docs/config-reference.md`.
//!
//! This is the "doc-gen test in CI" acceptance: the config
//! reference must stay in sync with the Rust schema. The test serializes
//! [`Config::default()`] to TOML, extracts every leaf key, and asserts
//! each one appears (backtick-quoted) in the reference document.
//!
//! When a field is added to `config.rs` without updating the doc, this
//! test fails with a message naming the missing key.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    // Test file: panics on setup failure are the correct behavior.
)]

use std::collections::BTreeSet;
use std::path::Path;

use flowshot_core::Config;

/// Extracts every leaf key from a TOML table, dotted for nested tables.
///
/// For example, `[tools.arrow]` with `style = "straight"` yields
/// `"tools.arrow.style"`. Array values are leaves (the key itself, not
/// the elements).
fn extract_leaf_keys(table: &toml::Table, prefix: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for (key, value) in table {
        let full_key = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(nested) => {
                keys.extend(extract_leaf_keys(nested, &full_key));
            }
            toml::Value::Array(_)
            | toml::Value::Boolean(_)
            | toml::Value::Integer(_)
            | toml::Value::Float(_)
            | toml::Value::String(_)
            | toml::Value::Datetime(_) => {
                keys.insert(full_key);
            }
        }
    }
    keys
}

/// The leaf key name (last segment of a dotted path).
fn leaf_name(dotted: &str) -> &str {
    dotted.rsplit('.').next().unwrap_or(dotted)
}

#[test]
fn config_reference_documents_every_schema_field() {
    // Given: the default config serialized to TOML.
    let config = Config::default();
    let toml_text = config
        .to_toml_string()
        .unwrap_or_else(|error| panic!("config serialization failed: {error}"));
    let table: toml::Table =
        toml::from_str(&toml_text).unwrap_or_else(|error| panic!("TOML parse failed: {error}"));

    // When: we extract every leaf key from the schema.
    let schema_keys = extract_leaf_keys(&table, "");
    assert!(!schema_keys.is_empty(), "schema must have at least one key");

    // And: we read the config reference document.
    let doc_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/config-reference.md")
        .canonicalize()
        .unwrap_or_else(|error| panic!("config-reference.md not found at expected path: {error}"));
    let doc_content = std::fs::read_to_string(&doc_path)
        .unwrap_or_else(|error| panic!("failed to read config-reference.md: {error}"));

    // Then: every leaf key name (the final segment) must appear in the doc.
    // We check the leaf name (not the full dotted path) because the doc
    // organizes keys under section headers like `## [tools.arrow]` and
    // lists them as `style`, `reverse`, etc. in tables.
    let mut missing = Vec::new();
    for dotted_key in &schema_keys {
        let name = leaf_name(dotted_key);
        // The doc uses backtick-quoted key names in tables.
        let quoted = format!("`{name}`");
        if !doc_content.contains(&quoted) {
            missing.push(dotted_key.clone());
        }
    }

    assert!(
        missing.is_empty(),
        "config-reference.md is missing the following schema field(s): {}\n\
         Add each key to the appropriate section of docs/config-reference.md.",
        missing.join(", ")
    );
}

#[test]
fn config_reference_documents_config_version() {
    // Given: the config reference document.
    let doc_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/config-reference.md")
        .canonicalize()
        .expect("config-reference.md path");
    let doc_content = std::fs::read_to_string(&doc_path).expect("read config-reference.md");

    // Then: `config_version` is documented (it is a top-level key, not
    // inside a table, so the leaf-name check above covers it; this test
    // makes the intent explicit).
    assert!(
        doc_content.contains("`config_version`"),
        "config-reference.md must document `config_version`"
    );
}

//! The consent session-child contract through the REAL binary and the
//! REAL hidden `session` verb (the `session_dispatch.rs` pattern): the
//! detached daemon spawn hands the child a lean `kind: "consent"` spec,
//! and the child must dispatch it into the dialog leg - which fails
//! TYPED and invisible in a headless environment (the display-server
//! check fires before any window exists) while recording NO answer (a
//! failed prompt re-arms the daemon-startup dialog; only a saved
//! choice writes the config).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FLOWSHOT_DAEMON: &str = env!("CARGO_BIN_EXE_flowshot-daemon");

/// The child fails at the display-server check instantly; the budget
/// turns a hang into a loud failure instead of stalling the suite.
const CHILD_BUDGET: Duration = Duration::from_secs(60);

/// A per-run scratch root (process- and clock-namespaced: concurrent
/// checkouts cannot collide).
fn scratch_root(tag: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("flowshot-{}-{}-{tag}", std::process::id(), nonce));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `command` to completion under [`CHILD_BUDGET`], killing it on
/// expiry (a bounded live run, never a suite hang).
fn run_bounded(command: &mut Command) -> Output {
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + CHILD_BUDGET;
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("the child did not finish within {CHILD_BUDGET:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The lean consent spec the detached daemon spawn writes: only the
/// kind and the two paths - every other field rides its serde default.
fn consent_spec(result_path: &Path, config_path: &Path) -> String {
    serde_json::json!({
        "kind": "consent",
        "result_path": result_path,
        "config_path": config_path,
    })
    .to_string()
}

#[test]
fn consent_child_fails_typed_headless_and_records_no_answer() {
    // Given: a lean consent spec and a headless environment (no
    // display-server socket - neither WAYLAND_DISPLAY nor DISPLAY - and no
    // bus), so the dialog's display-server check fails fast without ever
    // touching a compositor or opening a window.
    let dir = scratch_root("consent-session");
    let xdg = dir.join("xdg-runtime");
    std::fs::create_dir_all(&xdg).unwrap();
    let spec_path = dir.join("spec.json");
    let result_path = dir.join("result.json");
    let config_path = dir.join("flowshot.toml");
    std::fs::write(&spec_path, consent_spec(&result_path, &config_path)).unwrap();

    // When: the real daemon binary runs the hidden session verb.
    let output = run_bounded(
        Command::new(FLOWSHOT_DAEMON)
            .arg("session")
            .arg("--spec")
            .arg(&spec_path)
            .env("HOME", &dir)
            .env("XDG_RUNTIME_DIR", &xdg)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", xdg.join("absent-bus").display()),
            ),
    );

    // Then: no panic, a typed Failed result whose exit code the process
    // exit propagates, and NO config write (the failed prompt re-arms).
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(
        output.status.code(),
        Some(101),
        "the session child panicked: {stderr}"
    );
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&result_path)
            .unwrap_or_else(|error| panic!("no result JSON ({error}); stderr: {stderr}")),
    )
    .unwrap();
    assert_eq!(result["outcome"], "failed", "result: {result}");
    assert_eq!(
        output.status.code().unwrap(),
        i32::from(u8::try_from(result["exit_code"].as_u64().unwrap()).unwrap()),
        "the process exit must propagate the result's mapped code"
    );
    assert!(
        !config_path.exists(),
        "a failed prompt is not an answer; nothing may be written"
    );

    std::fs::remove_dir_all(&dir).ok();
}

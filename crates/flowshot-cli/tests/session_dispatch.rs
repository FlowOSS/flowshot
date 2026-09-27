//! The session-child dispatch regression (the live-found defect): the
//! hidden `session` verb used to dispatch INSIDE the CLI's tokio runtime,
//! so the child leg's own `block_on` panicked with "Cannot start a runtime
//! from within a runtime" - every interactive capture died before its
//! window opened. The verb must run on the main thread BEFORE the runtime
//! is built; this test drives the real binary through the real verb.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FLOWSHOT: &str = env!("CARGO_BIN_EXE_flowshot");

/// The headless child fails at the first capture-ladder step (~1 s); the
/// budget turns a hang into a loud failure instead of stalling the suite.
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

/// A valid overlay [`SessionSpec`] (every `CaptureRequest` field - the
/// spec contract has no per-field defaults).
fn overlay_spec(result_path: &Path) -> String {
    serde_json::json!({
        "kind": "overlay",
        "result_path": result_path,
        "image_path": null,
        "config_path": null,
        "request": {
            "delay_ms": 0,
            "instant": false,
            "no_edit": false,
            "copy": false,
            "output": null,
            "pin": false,
            "upload": false,
            "raw": false,
            "print_geometry": false,
            "hide_cursor": false,
            "region": null,
            "last_region": false
        },
        "color_mode": false,
        "forward_to_daemon": false,
        "pin": null
    })
    .to_string()
}

#[test]
fn session_child_fails_gracefully_headless_without_a_nested_runtime_panic() {
    // Given: a valid overlay spec and a headless environment (no Wayland
    // socket, no bus), so the child's capture preparation fails fast
    // without ever touching a compositor or opening a window.
    let dir = scratch_root("session-dispatch");
    let xdg = dir.join("xdg-runtime");
    std::fs::create_dir_all(&xdg).unwrap();
    let spec_path = dir.join("spec.json");
    let result_path = dir.join("result.json");
    std::fs::write(&spec_path, overlay_spec(&result_path)).unwrap();

    // When: the real binary runs the hidden session verb.
    let output = run_bounded(
        Command::new(FLOWSHOT)
            .arg("session")
            .arg("--spec")
            .arg(&spec_path)
            .env("HOME", &dir)
            .env("XDG_RUNTIME_DIR", &xdg)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", xdg.join("absent-bus").display()),
            ),
    );

    // Then: no nested-runtime panic (the defect signature), and the child
    // reported a typed Failed result whose exit code the process exit
    // propagates.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Cannot start a runtime"),
        "the session verb re-entered a tokio runtime: {stderr}"
    );
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

    std::fs::remove_dir_all(&dir).ok();
}

//! Empty overlay QA harness (plan todo 13).
//!
//! Spawns one borderless-fullscreen transparent window per monitor, hides
//! the system cursor, draws the custom crosshair, and idles at zero CPU
//! (`RedrawRequested`-driven frames). Esc on any window tears down all
//! windows and exits 0; startup failures exit 1 with a typed error.
//!
//! Live QA executed 2026-09-25 on the real 2-monitor Hyprland session (GUI
//! ban lifted per plan Verification strategy): per-monitor fullscreen
//! windows, no-reflow jq diff, Esc-exits-0 via the stdin injector, idle-CPU
//! sample, and the no-session error path - observed values recorded in
//! `.omo/evidence/task-13-flowshot.json`.
//!
//! With `--features test-drive`, stdin lines inject synthetic events through
//! the production routing path (no external injection tools needed):
//!
//! ```text
//! move <slot> <x> <y>      surface-local physical px
//! press <slot> <key>       key: escape | enter
//! release <slot> <key>
//! ```

use std::process::ExitCode;

use flowshot_ui::{OverlayRuntime, UiError};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowshot: overlay failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), UiError> {
    let runtime = OverlayRuntime::new()?;
    #[cfg(feature = "test-drive")]
    spawn_stdin_injector(runtime.handle().clone());
    runtime.run()
}

#[cfg(feature = "test-drive")]
fn spawn_stdin_injector(handle: flowshot_ui::OverlayHandle) {
    use std::io::BufRead;

    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let Some(input) = parse_command(&line) else {
                continue;
            };
            if let Err(error) = handle.inject_event(input) {
                eprintln!("flowshot: injection failed: {error}");
                break;
            }
        }
    });
}

#[cfg(feature = "test-drive")]
fn parse_command(line: &str) -> Option<flowshot_ui::SyntheticInput> {
    use flowshot_ui::{SyntheticInput, WindowSlot};
    use winit::keyboard::KeyCode;

    let mut parts = line.split_whitespace();
    match parts.next()? {
        "move" => {
            let slot = parts.next()?.parse::<usize>().ok()?;
            let x = parts.next()?.parse::<f64>().ok()?;
            let y = parts.next()?.parse::<f64>().ok()?;
            Some(SyntheticInput::pointer_moved(WindowSlot::new(slot), x, y))
        }
        command @ ("press" | "release") => {
            let slot = parts.next()?.parse::<usize>().ok()?;
            let code = match parts.next()? {
                "escape" => KeyCode::Escape,
                "enter" => KeyCode::Enter,
                _ => return None,
            };
            let slot = WindowSlot::new(slot);
            Some(if command == "press" {
                SyntheticInput::key_press(slot, code)
            } else {
                SyntheticInput::key_release(slot, code)
            })
        }
        _ => None,
    }
}

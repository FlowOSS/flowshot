//! Probes the live Wayland session and prints the report as JSON.
//!
//! Connects on the dedicated capture thread, enumerates outputs, detects
//! capture capabilities, prints everything to stdout, and exits 0. Any
//! failure prints the typed error (with its environment hint) to stderr and
//! exits 1.

use std::process::ExitCode;

use flowshot_capture_wayland::CaptureThread;

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("flowshot wayland probe failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, Box<dyn std::error::Error>> {
    let thread = CaptureThread::spawn()?;
    let snapshot = thread.snapshot()?;
    thread.shutdown();
    Ok(serde_json::to_string_pretty(&snapshot)?)
}

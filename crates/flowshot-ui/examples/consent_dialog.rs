//! Live QA harness for the first-launch telemetry consent dialog.
//!
//! Spawns the real [`flowshot_ui::consent::ConsentWindow`] on the live
//! session with the production seams wired the way the binary layer will
//! wire them (persistence callback + `app_id=flowshot-consent`), prints
//! the recorded choice, and exits on any dismissal path.
//!
//! Usage:
//!
//! ```text
//! cargo run -p flowshot-ui --example consent_dialog
//! ```

use std::process::ExitCode;

use flowshot_ui::consent::{ConsentCallback, ConsentWindow, ConsentWindowOptions};

mod common;

use common::session_window_customizer;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let options = ConsentWindowOptions {
        on_choice: Some(ConsentCallback::new(|choice| {
            println!(
                "CONSENT CHOICE: enabled={} technical_details={} asked_on_first_launch={}",
                choice.enabled, choice.include_technical_details, choice.asked_on_first_launch
            );
        })),
        window_customizer: Some(session_window_customizer(
            "flowshot-consent",
            "Help improve FlowShot?",
        )),
        ..ConsentWindowOptions::default()
    };
    let result = ConsentWindow::new(options).and_then(ConsentWindow::run);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowshot: consent dialog failed: {error}");
            ExitCode::FAILURE
        }
    }
}

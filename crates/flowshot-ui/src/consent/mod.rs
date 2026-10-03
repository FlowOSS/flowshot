//! The first-launch telemetry consent dialog: a small egui window on the
//! embedded stack (`crate::egui_host`) - the launcher-dialog pattern -
//! asking the two-tier opt-in question exactly once and handing the answer
//! to the binary layer for persistence.
//!
//! # Trigger rule (who shows the dialog when)
//!
//! The RESIDENT DAEMON is the single prompt owner: at daemon startup,
//! after the config load, `asked_on_first_launch == false` spawns this
//! dialog as a DETACHED session child - never awaited, so no capture or
//! command flow ever blocks on it ([`should_prompt`] is the pure decision
//! the binary layer consumes; the daemon-side session wiring is a
//! binary-layer follow-up, this crate stays pure). One-shot `--no-daemon`
//! runs and session children (overlay, launcher, settings, pin) NEVER
//! prompt: a dialog mid-capture is worse UX than deferral, and a one-shot
//! process is gone before the user could answer. Deferral is safe - the
//! next daemon startup asks, and the settings window's Telemetry card
//! offers the same choice at any time.
//!
//! # Answer contract (recorded exactly once)
//!
//! EVERY user dismissal path records an answer through the
//! [`ConsentCallback`] seam: "Save choice" (or Enter) writes the two
//! checkbox answers; "Not now", Esc, and the window close button all write
//! the deferred choice ([`ConsentModel::deferred`]: both flags false) -
//! always with `asked_on_first_launch = true`, so the dialog never nags
//! again. A programmatic [`ConsentHandle::request_exit`] is NOT an answer
//! (nothing is written; the prompt re-arms), and a killed process records
//! nothing either - honest, because nothing was chosen.
//!
//! # GDPR-honest defaults
//!
//! Both checkboxes start UNCHECKED even though telemetry is "recommended":
//! the recommendation lives in the label text, never in a pre-ticked box -
//! opt-in means the user's own click. The two checkboxes are independent
//! config fields; the daemon gates the tier-2 payload on both anyway.
//!
//! # Persistence seam (read-only config consumption)
//!
//! This crate never writes the config file: the chosen
//! [`TelemetryConfig`](flowshot_core::config::TelemetryConfig) goes to the
//! binary layer through [`ConsentCallback`]
//! exactly once per dialog lifetime, and the binary layer merges it into
//! the loaded config and persists through
//! [`Config::save`](flowshot_core::config::Config::save) (the
//! migration-safe save path). Like the settings card, an answer takes
//! effect on the NEXT process start - every process fixes its telemetry
//! state at init (first init wins).

mod app;
mod frame;
mod model;
mod offscreen;
mod options;
mod strings;
mod ui;
mod window;

#[cfg(test)]
mod tests;

pub use model::{ConsentModel, PromptSurface, should_prompt};
pub use offscreen::render_offscreen;
pub use options::{ConsentCallback, ConsentWindowOptions};
pub use ui::ConsentAction;
pub use window::{ConsentEvent, ConsentHandle, ConsentWindow};

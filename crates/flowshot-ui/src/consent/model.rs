//! The consent dialog's typed state and the pure trigger rule (who shows
//! the dialog when) - both GPU-free and window-free, so the whole consent
//! contract is unit-testable without a display.

use flowshot_core::config::TelemetryConfig;

/// The process surface that could ask for first-launch consent.
///
/// The trigger rule (module header): the resident daemon is the single
/// prompt owner; every other surface defers to the next daemon startup or
/// to the settings window's Telemetry card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSurface {
    /// The resident daemon's startup, after the config load - the ONLY
    /// prompting surface. Sessions are the daemon's children, so it spawns
    /// the dialog detached and never awaits it (no command blocks).
    DaemonStartup,
    /// A one-shot `--no-daemon` CLI run: never prompts - a dialog
    /// mid-capture is worse UX than deferral, and the process is gone
    /// before the user could answer.
    OneShotRun,
    /// A session child (overlay / launcher / settings / pin): never
    /// prompts - the capture flow must never be blocked or interrupted.
    SessionChild,
}

/// The trigger rule: prompt exactly while the question is unanswered, and
/// only on the daemon-startup surface.
///
/// Any recorded answer (`asked_on_first_launch = true` - "Save choice" AND
/// every user dismissal path) silences it for good; only a killed process
/// (no answer recorded) or a factory Reset legitimately re-arms it.
#[must_use]
pub fn should_prompt(config: &TelemetryConfig, surface: PromptSurface) -> bool {
    match surface {
        PromptSurface::DaemonStartup => !config.asked_on_first_launch,
        PromptSurface::OneShotRun | PromptSurface::SessionChild => false,
    }
}

/// The consent dialog's checkbox state.
///
/// Both boxes start UNCHECKED: GDPR-honest opt-in means the "recommended"
/// is label text, never a pre-ticked box - consent must be the user's own
/// click (the recorded decision, module header).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsentModel {
    send: bool,
    details: bool,
}

impl ConsentModel {
    /// The deferred answer ("Not now", Esc, the window close button): both
    /// flags false, the question recorded - the dialog never nags again.
    #[must_use]
    pub const fn deferred() -> TelemetryConfig {
        TelemetryConfig {
            enabled: false,
            include_technical_details: false,
            asked_on_first_launch: true,
        }
    }

    /// The "send telemetry" checkbox.
    #[must_use]
    pub const fn send(&self) -> bool {
        self.send
    }

    /// The "send telemetry" checkbox, for the widget binding.
    pub fn send_mut(&mut self) -> &mut bool {
        &mut self.send
    }

    /// The "technical details" checkbox.
    #[must_use]
    pub const fn details(&self) -> bool {
        self.details
    }

    /// The "technical details" checkbox, for the widget binding.
    pub fn details_mut(&mut self) -> &mut bool {
        &mut self.details
    }

    /// "Save choice": the two checkbox answers as the config write, with
    /// the question recorded. The checkboxes are INDEPENDENT fields - a
    /// details-without-send answer persists the preference for a later
    /// opt-in (the daemon gates the tier-2 payload on both flags anyway).
    #[must_use]
    pub const fn saved_choice(&self) -> TelemetryConfig {
        TelemetryConfig {
            enabled: self.send,
            include_technical_details: self.details,
            asked_on_first_launch: true,
        }
    }
}

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
/// ONLY a saved answer (`asked_on_first_launch = true`, written by "Save
/// choice") silences it for good. Every dismissal ("Not now", Esc, the
/// window close) records NOTHING, so the next daemon start asks again -
/// the re-ask contract the user directed over a silent-forever deferral;
/// the settings window's Telemetry card is the permanent control. A killed
/// process or a factory Reset re-arms it the same way.
#[must_use]
pub fn should_prompt(config: &TelemetryConfig, surface: PromptSurface) -> bool {
    match surface {
        PromptSurface::DaemonStartup => !config.asked_on_first_launch,
        PromptSurface::OneShotRun | PromptSurface::SessionChild => false,
    }
}

/// The consent dialog's checkbox state.
///
/// Defaults (USER DIRECTIVE 2026-10-04, overriding the earlier
/// both-unchecked GDPR-conservative default): "send telemetry" starts
/// CHECKED - the recommended tier-1 opt-in the user can uncheck before
/// saving; the GDPR-relevant details box starts UNCHECKED (recommended
/// off). Consent remains an AFFIRMATIVE ACT: nothing is sent - and
/// nothing is written - until the user saves; every dismissal path
/// records nothing regardless of the pre-check, and the next daemon
/// start asks again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentModel {
    send: bool,
    details: bool,
}

impl Default for ConsentModel {
    fn default() -> Self {
        Self {
            send: true,
            details: false,
        }
    }
}

impl ConsentModel {
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

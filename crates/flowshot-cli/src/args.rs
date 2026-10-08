//! The clap-derive command surface (the authoritative interface spec;
//! Flameshot interface parity is deliberately dropped, capability parity
//! kept).
//!
//! Parsing only produces the raw [`Cli`]; semantic validation (region
//! grammar, screen specs, cross-flag rules clap cannot express) happens in
//! [`crate::invocation`], which yields the typed [`crate::invocation::
//! Invocation`]. Usage-class failures exit 2 ([`crate::exit`]).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// `FlowShot` - capture, annotate, and share screenshots (`FlowOSS`).
///
/// Bare `flowshot` is `flowshot capture`: interactive region selection with
/// the annotation editor. A daemon is auto-spawned when absent so clipboard
/// offers and pins outlive this process; `--no-daemon` forces an in-process
/// one-shot instead.
#[derive(Debug, Parser)]
#[command(
    name = "flowshot",
    version,
    about = "FlowShot - capture, annotate, and share screenshots (FlowOSS)",
    long_about = "FlowShot - capture, annotate, and share screenshots (FlowOSS).\n\nBare flowshot is flowshot capture: interactive region selection with the annotation editor. A daemon is auto-spawned when absent so clipboard offers and pins outlive this process; --no-daemon forces an in-process one-shot instead."
)]
pub struct Cli {
    /// The command to run (default: capture).
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Print paste-ready compositor bind snippets for the global-shortcut
    /// fallback path and exit (overrides any subcommand).
    #[arg(long, global = true)]
    pub print_bind_help: bool,

    /// Session-bus address to talk to the daemon on (default: the session
    /// bus). Test/QA knob, mirrors the flowshot-daemon helper binary.
    #[arg(long, global = true, value_name = "ADDRESS")]
    pub bus_address: Option<String>,
}

/// The `FlowShot` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Capture the screen (default target: interactive region selection).
    Capture(CaptureArgs),
    /// Pin the last capture, or an image file, to the screen.
    Pin {
        /// Image file to pin (default: the last capture).
        file: Option<PathBuf>,
    },
    /// Pick a color from the screen (eyedropper); hex value to clipboard.
    Color,
    /// Open the settings UI (TOML config lives at
    /// ~/.config/flowshot/flowshot.toml; see flowshot-config(5)).
    #[command(visible_alias = "config")]
    Settings,
    /// Run the foreground daemon (for the systemd user unit or any other
    /// init supervisor; the daemon is init-agnostic).
    Daemon(DaemonArgs),
    /// Internal: run one window session from a spec file (the daemon's
    /// child-process contract - winit allows one event loop per
    /// process; hidden from the user surface).
    #[command(hide = true)]
    Session {
        /// The session spec JSON path.
        #[arg(long, value_name = "PATH")]
        spec: PathBuf,
    },
    /// Generate shell completions.
    Completions {
        /// Shell to generate completions for.
        shell: CompletionShell,
    },
}

/// The optional positional capture target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum CaptureTarget {
    /// Interactive region selection (default).
    #[default]
    Region,
    /// The full desktop (all outputs).
    Full,
    /// A single output: screen [<n|connector>]; NO argument captures the
    /// output under the cursor.
    Screen,
    /// Repeat the last captured region (persisted per the config).
    Last,
}

/// `flowshot capture` arguments: one optional target selector (positional
/// target OR one of --full/--screen/--region/--last-region) plus the
/// modifier set.
// The boolean flags are the independent spec-mandated capture modifiers of
// the CLI surface (same shape as the daemon's CaptureRequest).
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent spec-mandated capture modifier (the CLI surface)"
)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct CaptureArgs {
    /// What to capture (default: interactive region selection).
    #[arg(value_enum, conflicts_with_all = ["full", "screen", "region", "last_region", "dialog"])]
    pub target: Option<CaptureTarget>,

    /// Output to capture with the screen target: index (from 0) or
    /// connector name (e.g. DP-1); omit for the output under the cursor.
    #[arg(value_name = "N|CONNECTOR")]
    pub screen_spec: Option<String>,

    /// Capture the full desktop (all outputs).
    #[arg(long, conflicts_with_all = ["screen", "region", "last_region", "dialog"])]
    pub full: bool,

    /// Capture one output: index (from 0) or connector name (e.g. DP-1).
    #[arg(
        long,
        value_name = "N|CONNECTOR",
        conflicts_with_all = ["region", "last_region", "dialog"]
    )]
    pub screen: Option<String>,

    /// Preselect a region: a size (centered at the cursor), a size with
    /// offsets (global logical pixels; signed offsets allowed), or the
    /// literal "at-cursor".
    #[arg(long, value_name = "WxH[+X+Y]|at-cursor", conflicts_with_all = ["last_region", "dialog"])]
    pub region: Option<String>,

    /// Repeat the last captured region.
    #[arg(long, conflicts_with = "dialog")]
    pub last_region: bool,

    /// Wait MS milliseconds before capturing.
    #[arg(short = 'd', long, value_name = "MS")]
    pub delay: Option<u32>,

    /// Accept the selection on first mouse release: run the export actions
    /// immediately, no editor review step.
    #[arg(long)]
    pub instant: bool,

    /// Skip the editor; go straight to the post-capture actions (editing is
    /// the default).
    #[arg(long)]
    pub no_edit: bool,

    /// Add the copy-to-clipboard action for this invocation.
    #[arg(short = 'c', long)]
    pub copy: bool,

    /// Save destination: an existing directory or a full file path.
    #[arg(short = 'o', long, value_name = "FILE|DIR")]
    pub output: Option<PathBuf>,

    /// Add the pin-to-screen action.
    #[arg(long)]
    pub pin: bool,

    /// Add the upload action (needs a configured provider client id; see
    /// flowshot-config(5)).
    #[arg(long)]
    pub upload: bool,

    /// Write the PNG bytes to stdout (forces the in-process one-shot mode;
    /// stdout is never routed over D-Bus).
    #[arg(long)]
    pub raw: bool,

    /// Print the selection geometry `WxH+X+Y` to stdout (forces the
    /// in-process one-shot mode).
    #[arg(long)]
    pub print_geometry: bool,

    /// Exclude the cursor from the capture.
    #[arg(long)]
    pub hide_cursor: bool,

    /// Open the manual-coordinate capture launcher dialog instead of the
    /// overlay (replaces the Flameshot `launcher` subcommand).
    #[arg(
        long,
        conflicts_with_all = [
            "screen_spec",
            "delay",
            "instant",
            "no_edit",
            "copy",
            "output",
            "pin",
            "upload",
            "raw",
            "print_geometry",
            "hide_cursor",
        ]
    )]
    pub dialog: bool,

    /// Run in-process as a one-shot instead of forwarding to the daemon
    /// (documented trade-off: one-shot clipboard offers die with this
    /// process).
    #[arg(long)]
    pub no_daemon: bool,
}

/// `flowshot daemon` arguments (mirrors the `flowshot-daemon` helper
/// binary; the systemd user unit runs `flowshot daemon`).
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct DaemonArgs {
    /// Run as the auto-spawned helper: exit after the idle grace when no
    /// persistence reason holds (default: supervised, always persists).
    #[arg(long)]
    pub auto_spawned: bool,

    /// Idle grace in seconds for --auto-spawned mode.
    #[arg(long, value_name = "SECONDS", default_value_t = 60)]
    pub idle_grace: u64,

    /// Config file to load (default: &lt;xdg-config-home&gt;/flowshot/flowshot.toml).
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

/// Shells `flowshot completions` generates for (`clap_complete` family +
/// nushell; wayshot precedent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    /// GNU bash.
    Bash,
    /// Z shell.
    Zsh,
    /// fish.
    Fish,
    /// Elvish.
    Elvish,
    /// PowerShell.
    Pwsh,
    /// Nushell.
    Nushell,
}

//! The validated typed invocation: clap's raw [`Cli`] becomes an
//! [`Invocation`] exactly once (parse, don't validate), carrying the
//! daemon's [`CaptureRequest`] for the modifier bag. Everything downstream
//! (wire mapping, dispatch, the execution seam) consumes typed
//! values, never argv strings.

use std::ffi::OsString;
use std::path::PathBuf;

use flowshot_daemon::CaptureRequest;

use crate::args::{CaptureArgs, CaptureTarget, Cli, Command, CompletionShell};
use crate::exit::CliError;
use crate::strings;

/// The resolved CLI entry point. The split is the nested-runtime guard
/// (the live-found overlay-session defect): the hidden session-child verb
/// MUST run on the main thread BEFORE any tokio runtime exists - the child
/// legs (`overlay_child` and friends) build their own runtimes, and a
/// `block_on` from within an entered runtime panics ("Cannot start a
/// runtime from within a runtime"). Keeping `Session` OUT of
/// [`Invocation`] makes the illegal path unrepresentable: [`crate::dispatch`]
/// only ever sees [`Resolved::Command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// `<exe> session --spec PATH`: the window-session child
    /// (internal process-model contract; runs runtime-free at entry).
    Session(PathBuf),
    /// Every user-facing verb (dispatched inside the CLI's tokio runtime).
    Command(Invocation),
}

/// One fully validated CLI invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// `flowshot` / `flowshot capture ...`.
    Capture(CaptureInvocation),
    /// `flowshot capture --dialog`: the manual-coordinate launcher dialog
    /// (the daemon's `Launcher` wire member; the launcher-dialog surface).
    Launcher {
        /// In-process instead of daemon-forwarded (`--no-daemon`).
        one_shot: bool,
    },
    /// `flowshot pin [FILE]`.
    Pin(Option<PathBuf>),
    /// `flowshot color`.
    Color,
    /// `flowshot settings` (alias `config`).
    Settings,
    /// `flowshot daemon`.
    Daemon(DaemonRun),
    /// `flowshot completions <shell>`.
    Completions(CompletionShell),
    /// `--print-bind-help` (overrides any subcommand).
    PrintBindHelp,
}

/// A validated `capture` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureInvocation {
    /// What gets captured.
    pub selection: CaptureSelection,
    /// The modifier bag (daemon wire vocabulary).
    pub request: CaptureRequest,
    /// In-process one-shot: `--no-daemon`, or a stdout-producing flag
    /// (`--raw`/`--print-geometry` FORCE one-shot - stdout never routes
    /// over D-Bus, Oracle r4 F-5.iii).
    pub one_shot: bool,
}

/// The capture target after selector resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSelection {
    /// Interactive region selection (optionally preselected via
    /// `request.region` / `request.last_region`).
    Interactive,
    /// The full desktop.
    Full,
    /// A single output.
    Screen(ScreenSpec),
}

/// Which output `capture screen` targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenSpec {
    /// No argument: the output under the cursor (fixes the capability
    /// Flameshot hard-blocks on Wayland).
    Cursor,
    /// Numeric output index (from 0).
    Index(u32),
    /// Output connector name (e.g. `DP-1`).
    Connector(String),
}

/// `flowshot daemon` parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonRun {
    /// Auto-spawned helper mode (smart idle exit) vs supervised.
    pub auto_spawned: bool,
    /// Idle grace for the auto-spawned mode, in seconds.
    pub idle_grace_secs: u64,
    /// Explicit config path override.
    pub config: Option<PathBuf>,
}

/// The parsed `--region` grammar (`WxH[+X+Y]|at-cursor`, Oracle r4). The
/// WIRE carries the raw token ([`CaptureRequest::region`]); the executor
/// re-parses it against the output layout. This typed form is
/// the CLI-side validation: malformed tokens exit 2 before anything is
/// forwarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionToken {
    /// Center/anchor the preselection at the cursor.
    AtCursor,
    /// A preselected rectangle in global logical pixels.
    Rect {
        /// Width (> 0).
        width: u32,
        /// Height (> 0).
        height: u32,
        /// X offset (signed; multi-monitor layouts can be negative).
        x: Option<i32>,
        /// Y offset (signed).
        y: Option<i32>,
    },
}

/// Resolves the raw clap output into the typed entry point.
///
/// # Errors
///
/// [`CliError::Usage`] for grammar violations clap cannot express
/// (region tokens, screen specs, stray positional arguments) and
/// [`CliError::NonUnicodeArg`] for paths the wire cannot carry.
pub fn resolve(cli: Cli) -> Result<Resolved, CliError> {
    if cli.print_bind_help {
        return Ok(Resolved::Command(Invocation::PrintBindHelp));
    }
    match cli.command {
        Some(Command::Session { spec }) => Ok(Resolved::Session(spec)),
        // Bare `flowshot` = `flowshot capture`.
        None => capture(&CaptureArgs::default()).map(Resolved::Command),
        Some(Command::Capture(args)) => capture(&args).map(Resolved::Command),
        Some(Command::Pin { file }) => Ok(Resolved::Command(Invocation::Pin(file))),
        Some(Command::Color) => Ok(Resolved::Command(Invocation::Color)),
        Some(Command::Settings) => Ok(Resolved::Command(Invocation::Settings)),
        Some(Command::Daemon(args)) => Ok(Resolved::Command(Invocation::Daemon(DaemonRun {
            auto_spawned: args.auto_spawned,
            idle_grace_secs: args.idle_grace,
            config: args.config,
        }))),
        Some(Command::Completions { shell }) => {
            Ok(Resolved::Command(Invocation::Completions(shell)))
        }
    }
}

/// The argv tail (everything after argv\[0\]) as the `Invoke(as)` wire
/// carries it. The daemon re-parses it with THIS crate's clap surface.
///
/// # Errors
///
/// [`CliError::NonUnicodeArg`] when any element is not valid Unicode.
pub fn argv_tail(argv: &[OsString]) -> Result<Vec<String>, CliError> {
    argv.iter()
        .skip(1)
        .map(|arg| {
            arg.to_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| CliError::NonUnicodeArg(arg.clone()))
        })
        .collect()
}

fn capture(args: &CaptureArgs) -> Result<Invocation, CliError> {
    if args.dialog {
        return Ok(Invocation::Launcher {
            one_shot: args.no_daemon,
        });
    }
    let selection = resolve_selection(args)?;
    let request = build_request(args)?;
    // Stdout-producing flags FORCE one-shot (stdout never routes over
    // D-Bus); --no-daemon requests it explicitly.
    let one_shot = args.no_daemon || request.raw || request.print_geometry;
    Ok(Invocation::Capture(CaptureInvocation {
        selection,
        request,
        one_shot,
    }))
}

fn resolve_selection(args: &CaptureArgs) -> Result<CaptureSelection, CliError> {
    if args.screen_spec.is_some() && args.target != Some(CaptureTarget::Screen) {
        return Err(CliError::Usage(
            strings::SCREEN_SPEC_WITHOUT_TARGET.to_owned(),
        ));
    }
    if args.full {
        return Ok(CaptureSelection::Full);
    }
    if let Some(spec) = &args.screen {
        return Ok(CaptureSelection::Screen(parse_screen_spec(spec)?));
    }
    match args.target {
        // Region preselection (--region/--last-region) rides in the
        // request; the selection stays interactive.
        None | Some(CaptureTarget::Region | CaptureTarget::Last) => {
            Ok(CaptureSelection::Interactive)
        }
        Some(CaptureTarget::Full) => Ok(CaptureSelection::Full),
        Some(CaptureTarget::Screen) => Ok(CaptureSelection::Screen(match &args.screen_spec {
            Some(spec) => parse_screen_spec(spec)?,
            None => ScreenSpec::Cursor,
        })),
    }
}

fn build_request(args: &CaptureArgs) -> Result<CaptureRequest, CliError> {
    if let Some(token) = &args.region {
        parse_region_token(token)?;
    }
    Ok(CaptureRequest {
        delay_ms: args.delay.unwrap_or(0),
        instant: args.instant,
        no_edit: args.no_edit,
        copy: args.copy,
        output: match &args.output {
            Some(path) => Some(path_to_string(path)?),
            None => None,
        },
        pin: args.pin,
        upload: args.upload,
        raw: args.raw,
        print_geometry: args.print_geometry,
        hide_cursor: args.hide_cursor,
        region: args.region.clone(),
        last_region: args.last_region || args.target == Some(CaptureTarget::Last),
    })
}

fn path_to_string(path: &std::path::Path) -> Result<String, CliError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::NonUnicodeArg(path.as_os_str().to_owned()))
}

/// Parses a `<n|connector>` screen selector: all-ASCII-digit strings are
/// indices, anything else non-empty and whitespace-free is a connector
/// name.
///
/// # Errors
///
/// [`CliError::Usage`] for empty, whitespace-bearing, or out-of-range
/// selectors.
pub fn parse_screen_spec(spec: &str) -> Result<ScreenSpec, CliError> {
    if spec.is_empty() || spec.contains(char::is_whitespace) {
        return Err(CliError::Usage(strings::SCREEN_SPEC_EMPTY.to_owned()));
    }
    if spec.bytes().all(|byte| byte.is_ascii_digit()) {
        return spec
            .parse::<u32>()
            .map(ScreenSpec::Index)
            .map_err(|_| CliError::Usage(strings::SCREEN_SPEC_OUT_OF_RANGE.to_owned()));
    }
    Ok(ScreenSpec::Connector(spec.to_owned()))
}

/// Validates the `--region` grammar: `at-cursor`, `WxH`, or `WxH±X±Y`
/// (each offset independently signed; global logical pixels).
///
/// # Errors
///
/// [`CliError::Usage`] for any token outside the grammar.
pub fn parse_region_token(token: &str) -> Result<RegionToken, CliError> {
    const AT_CURSOR: &str = "at-cursor";
    if token == AT_CURSOR {
        return Ok(RegionToken::AtCursor);
    }
    let invalid = || CliError::Usage(format!("{} (got {token:?})", strings::REGION_INVALID));
    let (size, offsets) = match token.find(['+', '-']) {
        Some(index) => (&token[..index], Some(&token[index..])),
        None => (token, None),
    };
    let (width_str, height_str) = size.split_once('x').ok_or_else(invalid)?;
    let width = parse_dimension(width_str).ok_or_else(invalid)?;
    let height = parse_dimension(height_str).ok_or_else(invalid)?;
    let (x, y) = match offsets {
        None => (None, None),
        Some(rest) => {
            let (x_str, y_str) = split_offsets(rest).ok_or_else(invalid)?;
            let x = parse_coordinate(x_str).ok_or_else(invalid)?;
            let y = parse_coordinate(y_str).ok_or_else(invalid)?;
            (Some(x), Some(y))
        }
    };
    Ok(RegionToken::Rect {
        width,
        height,
        x,
        y,
    })
}

/// One `±N` offset pair tail (`+10+20`, `-10-20`, mixed signs allowed).
fn split_offsets(rest: &str) -> Option<(&str, &str)> {
    let second = rest[1..].find(['+', '-'])? + 1;
    Some((&rest[..second], &rest[second..]))
}

fn parse_dimension(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|value| *value > 0)
}

fn parse_coordinate(text: &str) -> Option<i32> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<i32>().ok()
}

/// The `--upload` gate: the shared default client-id was
/// dropped, so upload with an unconfigured `[upload].client_id` is a
/// usage-class rejection with a settings hint.
///
/// # Errors
///
/// [`CliError::Usage`] when `request.upload` is set but no client id is
/// configured.
pub fn validate_upload(
    request: &CaptureRequest,
    config: &flowshot_core::Config,
) -> Result<(), CliError> {
    if request.upload && config.upload.client_id.trim().is_empty() {
        return Err(CliError::Usage(strings::UPLOAD_UNCONFIGURED.to_owned()));
    }
    Ok(())
}

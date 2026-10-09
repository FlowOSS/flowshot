<img src="assets/logo.png" width="128" height="128" alt="FlowShot logo: a violet-to-azure gradient circle with white lens arcs">

# FlowShot

**Screenshot and annotation for Linux — completely free software, native on
Wayland *and* X11.**

[![CI](https://github.com/FlowOSS/flowshot/actions/workflows/ci.yml/badge.svg)](https://github.com/FlowOSS/flowshot/actions/workflows/ci.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)
[![Platform](https://img.shields.io/badge/session-Wayland%20%E2%80%A2%20X11-6366F1)](#desktop-support)

Capture a region, annotate it, and get it where you need it — clipboard,
disk, a pin on your screen, or an upload — without fighting your compositor.
FlowShot is built by [FlowOSS](https://github.com/FlowOSS) as one native
codebase for modern Linux desktops: no XWayland fallbacks, no per-compositor
forks, and no account, cloud, or telemetry you did not ask for.

## Why FlowShot

- **Correct pixels, everywhere.** All geometry is physical-pixels-first, so
  captures on mixed-DPI and fractional-scaling setups are neither doubled nor
  halved — and region selection spans every monitor as one surface.
- **Native on Wayland and X11.** The runtime probes your live session and
  picks the best capture backend (compositor protocols, KWin's D-Bus
  interface, the XDG portal, or native X11) — never a nested-X11 hack.
- **A clipboard that survives.** The background daemon owns the clipboard
  offer, so what you copied is still there after the capture window closes.
- **Secure redaction.** The pixelate tool samples only the fringes of the
  region and never reads the interior pixels — the redaction is not
  reversible by block-mean attacks. There is no insecure mode to enable.
- **Private by default.** Telemetry is opt-in, self-hosted, and off until you
  explicitly consent; images never leave your machine, identifiers are
  stripped, and file paths are scrubbed from crash reports.
- **A daemon that knows when to leave.** It stays alive only while something
  needs it — tray, global shortcuts, open pins, a held clipboard offer — and
  exits after an idle grace otherwise.

## Features

**Capture** — interactive region selection (spanning monitors), fullscreen,
one output by index or connector name, the output under the cursor, repeat
last region, delayed capture, accept-on-select (`--instant`), region
preselect (`--region WxH[+X+Y]|at-cursor`), a manual-coordinate launcher
dialog (`--dialog`), a color picker (`flowshot color`), and D-Bus triggers
for scripting.

**Annotate** — pencil, line, arrow (straight or curved, reversible head),
rectangle (configurable corner radius), ellipse, highlighter, text with real
input-method (IME) support, numbered step bubbles with automatic
renumbering, secure pixelate, gaussian blur, color inversion, move, and an
eyedropper. Plus a pixel magnifier with hex/RGB readout, snapping grid,
geometry HUD, digit-key and mouse-wheel tool sizing, undo/redo, and z-order
control for every object.

**After capture** — copy to clipboard (PNG/JPEG), save to disk (PNG/JPEG/WebP
with quality control, `strftime` filename patterns, collision numeration),
pin to screen (drag, zoom-to-cursor, rotate, opacity), upload (Imgur behind a
pluggable provider trait, with history and delete hashes; ships unconfigured,
you bring your own client id), open with another app, and desktop
notifications. The default behavior is one ordered action list
(`[save].actions`), not a soup of interacting booleans.

**Shell** — single-instance background daemon on the session bus
(`org.flowoss.FlowShot`) with the smart lifecycle above; status tray (SNI)
with a per-monitor capture submenu; global shortcuts through the XDG portal
with paste-ready compositor binds as the fallback (`flowshot
--print-bind-help`); TOML configuration with versioned migration and a
settings UI; shell completions (bash, zsh, fish, elvish, PowerShell,
nushell) and man pages (`flowshot(1)`, `flowshot-config(5)`).

## Install

Build from source (Rust stable — see
[rust-toolchain.toml](rust-toolchain.toml)):

```sh
cargo build --release
# The multiplexed binary:
./target/release/flowshot --help
```

Build-time requirements: a C toolchain, `clang` (for libclang — the build
generates PipeWire FFI bindings), `pkg-config`, and the PipeWire development
headers (`libpipewire-0.3-dev` on Debian/Ubuntu, `pipewire` on Arch/Fedora). At runtime you need a Wayland compositor or an X11
server, a D-Bus session bus, PipeWire, and — for the portal-based features —
`xdg-desktop-portal` plus your desktop's portal backend. The tray needs an
SNI host (waybar's tray module, KDE Plasma's system tray, the GNOME
AppIndicator extension). A Vulkan-capable GPU driver (Mesa or equivalent)
powers the overlay renderer.

## Quick start

```sh
flowshot                          # interactive region capture with the editor
flowshot capture full             # the whole desktop, all outputs
flowshot capture screen           # the output under the cursor
flowshot capture screen DP-1      # one output by connector name (or index)
flowshot capture last             # repeat the last captured region
flowshot capture -d 2000          # capture after a 2 second delay
flowshot capture --instant        # accept on first mouse release, no editor
flowshot capture --no-edit -c     # skip the editor, straight to clipboard
flowshot capture -o ~/Pictures    # save into a directory
flowshot capture --pin            # also pin the result to the screen
flowshot capture --raw > out.png  # PNG bytes to stdout
flowshot pin [FILE]               # pin the last capture, or an image file
flowshot color                    # pick a color, hex to clipboard
flowshot settings                 # open the settings UI
flowshot completions bash         # shell completions
flowshot --print-bind-help        # paste-ready hotkey snippets for your desktop
```

Per-invocation flags (`-c`, `-o`, `--pin`, `--upload`) merge into your
configured `[save].actions` set for that one capture; they never rewrite the
config file. Exit codes: `0` success, `1` infrastructure failure, `2` usage
error, `3` user-cancelled, `4` capture-backend failure, `5` permission
denied, `6` export failure. Full surface: `flowshot --help`,
`man flowshot`, and `man flowshot-config`.

## Desktop support

| Desktop | Capture backend | Cursor-aware preselect | Notes |
|---|---|---|---|
| Hyprland | ext-image-copy-capture-v1 | Yes (ICC cursor session + Hyprland IPC) | Fully supported; verified live. [Guide](docs/setup-hyprland.md) |
| sway 1.10+ / wlroots | ext-image-copy-capture-v1 (wlr-screencopy fallback) | Yes (ICC cursor session) | [Guide](docs/setup-sway.md) |
| COSMIC | ext-image-copy-capture-v1 | Yes (ICC cursor session) | Source-verified |
| niri | wlr-screencopy | No (first-motion fallback) | ICC support is an open upstream PR |
| KDE Plasma | KWin ScreenShot2 D-Bus | First-motion fallback | Needs the packaged `.desktop` file. [Guide](docs/setup-kde.md) |
| GNOME | XDG portal (Screenshot / ScreenCast) | No (GNOME exposes no cursor-position API) | Portal picker appears on some paths. [Guide](docs/setup-gnome.md) |
| i3 / X11 desktops | native X11 (GetImage, MIT-SHM fast path) | One-shot (incl. `--region at-cursor`) | Fully supported; verified live on i3. Multi-monitor spanning on X11 is not yet hardware-verified. [Guide](docs/setup-x11.md) |

The runtime picks the backend by probing the live session, not from this
table; a compositor that grows protocol support is picked up without a
FlowShot update. Where a desktop's own interfaces force an honest
degradation (GNOME's cursor position, niri's ICC), the table says so.
Details: [the backend ladder ADR](docs/architecture/adr-002-capture-backend-ladder.md).

## Configuration

One optional TOML file at `~/.config/flowshot/flowshot.toml`. A missing or
corrupt file never breaks a capture: defaults apply and a warning is logged;
old files migrate forward automatically.

```toml
config_version = 3

[save]
path = ""                    # empty = platform pictures directory
filename_pattern = "%F_%H-%M"
actions = ["copy"]           # ordered set: copy, save, pin, upload,
                             # copy-path, notify, open-with

[ui]
accent_color = "#6366F1"

[daemon]
tray = false
startup_launch = false
```

Every key, its type, default, and range:
[docs/config-reference.md](docs/config-reference.md).

## Telemetry

FlowShot has telemetry, and it is opt-in: nothing is sent until you
explicitly enable it (a first-launch dialog asks once; the settings UI and
`[telemetry]` config group control it afterwards). Crash and error reports
go to a self-hosted Sentry instance — no third-party analytics, ever.
Screenshots and image data are never transmitted; identifiers are stripped
and file paths are scrubbed from every event. A second, separate consent
gates the technical-detail tier (kernel, monitor layout, GPU family). The
full field list lives in
[docs/config-reference.md](docs/config-reference.md).

## Project status

Pre-1.0, in active development. The capture backends, overlay, annotation
editor, pins, export actions, daemon, tray, shortcuts, settings UI, launcher
dialog, color picker, and the CLI wired end to end through the daemon are
implemented and verified — unit and property tests plus live-session QA with
recorded evidence ([docs/verification.md](docs/verification.md)). X11
support shipped in two live-verified phases on i3. Packaging (AUR first) is
in progress. Still open: live KDE Plasma and GNOME session checks
(stub/source-verified meanwhile), and a Hyprland permission-denial pass that
needs a compositor restart.

What FlowShot deliberately does not do: no screen recording, no OCR, no
scroll capture, no update checker, no GNOME shell extension, no
Windows/macOS code yet (the porting gates and roadmap are in
[docs/architecture/adr-006-cross-platform-gates.md](docs/architecture/adr-006-cross-platform-gates.md)).

## Development

```sh
just check                    # THE gate: fmt + clippy -D warnings + tests
                              # + platform-purity + cargo deny
cargo build                   # build the workspace
cargo test --workspace        # unit, property, and stubbed integration tests
```

CI runs the same gates (`.github/workflows/ci.yml`). Architectural decisions
are recorded as ADRs in [docs/architecture/](docs/architecture/); the
verification policy and its evidence classes are in
[docs/verification.md](docs/verification.md). Contributor guide:
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

GPL-3.0-or-later — see [LICENSE](LICENSE). Third-party notices:
[NOTICES](NOTICES).

<img src="assets/logo.png" width="128" height="128" alt="FlowShot logo: a violet-to-azure gradient circle with white lens arcs">

# FlowShot

FlowShot is a screenshot tool for Linux/Wayland, built by FlowOSS. X11
sessions are covered for headless capture (full/screen/region with copy, save,
raw output); the interactive editor, pins, and dialogs are Wayland-only in
this release. It covers the
Flameshot feature set (region, fullscreen, and per-monitor capture; the full
annotation kit; pins; upload; tray; global hotkeys) with the Wayland breakage
that tool never fixed actually repaired, and with its own deliberately
redesigned command line and config. It is capability-compatible with Flameshot,
not interface-compatible.

License: GPL-3.0-or-later. Status: pre-1.0, in active development (see
[Project status](#project-status)).

## Why it exists

Flameshot's open Wayland bugs are the spec. FlowShot was founded to fix them,
and each fix is verified on a live compositor, not assumed:

- **Region selection spans all monitors again.** One fullscreen overlay per
  output, one shared global-logical selection space; a drag started on one
  monitor continues onto the next.
- **Mouse-position-aware capture** on Hyprland, wlroots compositors, and COSMIC:
  the capture starts pre-selected at the output (or region) under the cursor.
  The position comes from the compositor's own protocols, not from Xwayland.
- **Clipboard that survives.** The background daemon owns the clipboard offer,
  so pasted content stays valid after the capture window closes.
- **Correct pixels on mixed-DPI setups.** All geometry is physical-pixels
  first; per-output scale is applied at exactly one conversion boundary, so a
  capture of a scale-2 monitor is neither doubled nor halved.
- **Pins that do not distort** under fractional scaling.
- **A crosshair you can actually see**, and a `--hide-cursor` option.
- **Built-in global shortcuts** through the XDG GlobalShortcuts portal, with
  paste-ready compositor binds as the fallback.
- **Never a forced Xwayland fallback.** The X11 support that ships is native
  X11 (xcb `GetImage`), and a Wayland session never routes to it.

The per-desktop reality, including the places where a compositor's protocols
force an honest degradation (GNOME), lives in [docs/](docs/).

## Feature inventory

Capture modes:

- Interactive region selection (multi-monitor spanning), fullscreen, single
  output by index or connector name, output under the cursor, repeat last
  region, delayed capture, accept-on-select (`--instant`), region preselect
  (`--region WxH[+X+Y]|at-cursor`).
- Manual-coordinate capture launcher dialog (`--dialog`).
- D-Bus triggers for scripting and second-instance forwarding.

Annotation editor:

- Pencil, line, arrow (straight or curved shaft, reversible head), rectangle
  (configurable corner radius), ellipse/circle, highlighter, text with real
  input-method (IME) support, numbered step bubbles with automatic
  renumbering, secure pixelate, gaussian blur, color inversion, selection and
  object move tools, eyedropper.
- Pixel magnifier (square or circle) with a hex/RGB readout, snapping grid,
  geometry HUD, digit-key tool sizing, mouse-wheel sizing, undo/redo, and
  z-ordering (raise/lower/reorder) for every object.
- The pixelate tool is secure-only: it samples the fringes of the redacted
  region and never reads the interior pixels, so the redaction is not
  reversible by block-mean attacks. There is no insecure mode to enable.

After capture:

- Copy to clipboard (PNG or JPEG), save to disk (PNG/JPEG/WebP with quality
  control, `strftime` filename patterns, collision numeration), pin to screen
  (drag, zoom-to-cursor, rotate, opacity), upload (Imgur behind a pluggable
  provider trait, with history and delete hashes; ships unconfigured, you
  register your own client id), open with another app, desktop notifications.
- The default action set is one ordered list (`[save].actions`), not a soup of
  interacting booleans.

Application shell:

- Single-instance background daemon on the session bus
  (`org.flowoss.FlowShot`) with a smart lifecycle: it stays alive only while
  something needs it (tray, registered shortcuts, a held clipboard offer, open
  pins) and exits after an idle grace otherwise. Init-agnostic: it is a plain
  foreground process; a systemd user unit is one supervised option.
- Status tray (SNI) with a per-monitor capture submenu.
- Global shortcuts via the XDG portal, with per-compositor bind snippets as
  the fallback (`flowshot --print-bind-help`).
- TOML configuration with versioned migration and a settings UI. Shell
  completions (bash, zsh, fish, elvish, PowerShell, nushell) and man pages
  (`flowshot(1)`, `flowshot-config(5)`).

## Install

Pre-built packages land with the packaging milestone:

- AUR (`flowshot`, `flowshot-git`): **TBD**
- Nix flake: **TBD**
- Flatpak (`org.flowoss.FlowShot`): **TBD**

Until then, build from source (Rust stable, see
[rust-toolchain.toml](rust-toolchain.toml)):

```sh
cargo build --release
# The multiplexed binary:
./target/release/flowshot --help
```

Runtime requirements: a Wayland compositor or (headless capture only) an X11
server, a D-Bus session bus, and (for the
portal features) `xdg-desktop-portal` plus your desktop's portal backend. The
tray needs an SNI host (waybar's tray module, KDE Plasma's system tray, the
GNOME AppIndicator extension).

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
flowshot capture --print-geometry # print WxH+X+Y of the selection
flowshot pin [FILE]               # pin the last capture, or an image file
flowshot color                    # pick a color, hex to clipboard
flowshot settings                 # open the settings UI
flowshot daemon                   # foreground daemon (what the systemd unit runs)
flowshot completions bash         # shell completions
flowshot --print-bind-help        # paste-ready hotkey snippets for your desktop
```

`flowshot capture --region` takes `WxH` (centered on the cursor), `WxH+X+Y`
(signed offsets, global logical pixels), or `at-cursor` (preselects the whole
output under the cursor). Per-invocation flags (`-c`, `-o`, `--pin`,
`--upload`) merge into the configured `[save].actions` set for that one
capture; they never write the config file.

Exit codes: `0` success, `1` infrastructure failure, `2` usage error,
`3` user-cancelled, `4` capture-backend failure, `5` permission denied,
`6` export failure. Legacy Flameshot verbs (`gui`, `launcher`, `screen` as a
bare subcommand) are rejected with a hint. Full surface: `flowshot --help`,
`man flowshot`, and `man flowshot-config`.

## Configuration

One TOML file at `~/.config/flowshot/flowshot.toml`. A missing or corrupt file
never breaks a capture: defaults apply and a warning is logged. Unknown keys
are ignored. Old files are migrated forward automatically (`config_version`).

```toml
config_version = 2

[capture]
hide_cursor = false
save_last_region = true

[save]
path = ""                    # empty = platform pictures directory
filename_pattern = "%F_%H-%M"
extension = "png"
jpeg_quality = 75
clipboard_format = "png"
actions = ["copy"]           # ordered set: copy, save, pin, upload,
                             # copy-path, notify, open-with

[editor]
draw_color = "#FF0000"
draw_thickness = 3
font_family = "Noto Sans"
font_size = 8
magnifier = false
magnifier_shape = "square"
grid = false
undo_limit = 100
side_panel = true

[ui]
accent_color = "#6366F1"
contrast_color = "#0F172A"
dim_opacity = 190

[daemon]
tray = false
notifications = true
startup_launch = false
```

Every key, its type, default, and range: [docs/config-reference.md](docs/config-reference.md).

## Desktop support

| Desktop | Capture backend | Cursor-aware preselect | Notes |
|---|---|---|---|
| Hyprland | ext-image-copy-capture-v1 | Yes (ICC cursor session + Hyprland IPC) | Fully supported; verified live. [Guide](docs/setup-hyprland.md) |
| sway 1.10+ / wlroots | ext-image-copy-capture-v1 (wlr-screencopy fallback) | Yes (ICC cursor session) | [Guide](docs/setup-sway.md) |
| COSMIC | ext-image-copy-capture-v1 | Yes (ICC cursor session) | Source-verified, not run live here |
| niri | wlr-screencopy | No (first-motion fallback) | ICC support is an open upstream PR |
| KDE Plasma | KWin ScreenShot2 D-Bus | First-motion fallback | Needs the packaged `.desktop` file. [Guide](docs/setup-kde.md) |
| GNOME | XDG portal (Screenshot / ScreenCast) | No (GNOME exposes no cursor-position API) | Portal picker appears on some paths. [Guide](docs/setup-gnome.md) |
| i3 / X11 desktops | xcb GetImage (MIT-SHM fd-passing fast path) | One-shot (XQueryPointer; `--region at-cursor` is Phase B) | Headless capture only in this release — editor/pins/dialogs need Wayland. [Guide](docs/setup-x11.md) |

The runtime picks the backend by probing the live session, not from this
table; a compositor that grows protocol support is picked up without a
FlowShot update. Details: [docs/architecture/adr-002-capture-backend-ladder.md](docs/architecture/adr-002-capture-backend-ladder.md).

## Development

```sh
cargo build                 # build the workspace
cargo test --workspace      # unit, property, and stubbed integration tests
cargo clippy --workspace -- -D warnings
cargo fmt --check
cargo deny check            # license policy (deny.toml)
```

CI runs the same gates (`.github/workflows/ci.yml`). UI correctness is
verified on a live Hyprland session with screenshot evidence; the policy and
its evidence-trail conventions are documented in
[docs/verification.md](docs/verification.md). The architectural decisions are
recorded as ADRs in [docs/architecture/](docs/architecture/).

## Project status

The capture backends, overlay, annotation editor, pins, export actions,
daemon, tray, shortcuts, settings UI, launcher dialog, and the CLI wired end
to end through the daemon are implemented and verified (unit/property tests
plus live-session QA; see
[docs/verification.md](docs/verification.md)). X11 Phase A shipped
2026-10-04: the headless capture path (full/screen/region, copy/save/raw/
print-geometry/delay, daemon-owned clipboard) is live-verified on i3,
evidence bundle `.omo/evidence/x11-phase-a/`; the interactive UI on X11
remains Phase B. Still open before the first
release:

- Packaging: AUR, Nix flake, Flatpak (**TBD**).
- The live checks this QA machine cannot run: KDE Plasma and GNOME sessions
  (stub/source-verified meanwhile), and a Hyprland permission-denial pass
  that needs a compositor restart. The queue and its classes are documented
  in [docs/verification.md](docs/verification.md).

What FlowShot deliberately does not do: no screen recording, no OCR, no scroll
capture, no telemetry, no update checker, no importing Flameshot's old config,
no GNOME shell extension, no Windows/macOS code yet (the porting gates and
roadmap, including the shipped X11 phase, are in
[docs/architecture/adr-006-cross-platform-gates.md](docs/architecture/adr-006-cross-platform-gates.md)).

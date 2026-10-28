<div align="center">
  <img src="assets/logo.png" width="80" height="80" alt="FlowShot logo: a violet-to-azure gradient circle with white lens arcs">
  <h1>FlowShot</h1>
  <p><strong>Screenshot and annotation for Linux: free, open source, native on Wayland and X11.</strong></p>

  <p>
    <a href="https://github.com/FlowOSS/flowshot/releases"><img src="https://img.shields.io/github/v/release/FlowOSS/flowshot?color=6366F1&label=release" alt="GitHub release"></a>
    <a href="https://aur.archlinux.org/packages/flowshot-bin"><img src="https://img.shields.io/aur/version/flowshot-bin?color=1793D1&logo=arch-linux&logoColor=white&label=flowshot-bin" alt="AUR flowshot-bin"></a>
    <a href="https://aur.archlinux.org/packages/flowshot-git"><img src="https://img.shields.io/aur/version/flowshot-git?color=1793D1&logo=arch-linux&logoColor=white&label=flowshot-git" alt="AUR flowshot-git"></a>
    <a href="https://github.com/FlowOSS/flowshot/actions/workflows/ci.yml"><img src="https://github.com/FlowOSS/flowshot/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0+-blue" alt="License: GPL-3.0+"></a>
    <a href="#desktop-support"><img src="https://img.shields.io/badge/session-Wayland-6366F1" alt="Wayland"></a>
    <a href="#desktop-support"><img src="https://img.shields.io/badge/session-X11-6366F1" alt="X11"></a>
  </p>
</div>

<p align="center">
  <img src="assets/preview.png" alt="FlowShot capture and annotation preview" width="850">
</p>

Capture any region of your screen, annotate it with modern tools, and export it instantly: copy to clipboard, save to disk, pin it to float on your desktop, or upload. 

Built natively for modern Linux desktops: no XWayland bridges, no per-compositor hacks, and full support for mixed-DPI multi-monitor setups.

## Why FlowShot

- **Pixel-perfect across mixed displays:** Geometry is physical-pixels-first, meaning fractional scaling and mixed-DPI monitors are captured accurately without blur or misaligned crops. Region selection spans all monitors as one unified canvas.
- **Native Wayland and X11 integration:** Probes your active session and selects the best native capture protocol (ext-image-copy-capture, wlr-screencopy, KWin ScreenShot2, XDG portal, or native X11 SHM).
- **GPU accelerated with CPU fallback:** Renders smoothly via Vulkan/wgpu with an automatic, zero-dependency software rasterizer fallback if GPU memory runs out or Vulkan is unavailable.
- **Privacy-first:** No data leaves your machine without your explicit confirmation. Crash reporting is opt-in, self-hosted, and transparent.
- **Secure redaction:** Pixelate and blur tools never leak interior pixel data, preventing reverse-engineering via block-mean recovery.

## Compared to Flameshot

FlowShot is not trying to be Flameshot, but it is trying to be the thing
Flameshot has never quite managed on modern Linux:

- **Capture and annotation run at least 2x faster than Flameshot** on the
  machines we test on. These are our own measurements, taken on the same
  desktops, and the comparison is worth reproducing rather than trusting.
- **Some long-standing upstream problems are unrepresentable here by design.**
  Mixed-DPI and fractional-scaling corruption (upstream #4871) cannot occur:
  geometry is physical-pixels-first, so there is no logical/physical mix to
  get wrong ([ADR-001](docs/architecture/adr-001-physical-first-geometry.md)).
  Flameshot's pixelation can be reversed; ours cannot be recovered from
  ([ADR-005](docs/architecture/adr-005-secure-pixelate.md)).

## Features

### Capture
- Region selection across all monitors as one canvas, plus full-desktop and single-monitor capture
- Delayed capture, preselected regions, accept-on-select, and a precision magnifier with live RGB/HEX readouts

### Annotation
- Pencil, lines, arrows (straight and curved), rectangles — outlined and filled
- Text with IME support, highlighter, auto-incrementing step counters
- Secure pixelation and blur, snapping grid, geometry HUD, palette editor, undo/redo

### Export
- Clipboard (PNG / JPEG), disk with configurable filename patterns and quality
- Pin to desktop as always-on-top reference windows
- Upload with copyable links, or open in an external viewer

## Installation

### Arch Linux (AUR)

FlowShot is available on the Arch User Repository:

```sh
# Pre-built binary package (recommended)
yay -S flowshot-bin

# Or build latest git trunk from source
yay -S flowshot-git
```

### Other Linux Distributions

Pre-compiled binary releases and AppImages are available on the [Releases page](https://github.com/FlowOSS/flowshot/releases).

<details>
<summary><strong>Building from source (for developers)</strong></summary>

Requires a Rust stable toolchain, `clang`, `pkg-config`, and PipeWire development headers (`pipewire` on Arch/Fedora, `libpipewire-0.3-dev` on Debian/Ubuntu):

```sh
git clone https://github.com/FlowOSS/flowshot.git
cd flowshot
cargo build --release

# Run the binary:
./target/release/flowshot capture
```
</details>

## Quick Start

The simplest way to use FlowShot is with your keyboard:

```sh
flowshot                     # Open the interactive screenshot overlay
flowshot settings            # Open the graphical settings window
flowshot color               # Quick eyedropper tool (copies HEX to clipboard)
```

### Hotkey Setup

To bind FlowShot to your **Print Screen** button:

* **Hyprland** (`hyprland.conf` / `hyprland.lua`):
  ```ini
  bind = ,Print,exec,flowshot capture
  ```
* **Sway / i3**:
  ```ini
  bindsym Print exec flowshot capture
  ```
* **GNOME / KDE:** Open system settings -> Keyboard Shortcuts -> Add custom shortcut -> Command: `flowshot capture`.

To view paste-ready shortcut snippets tailored for your current desktop, run:
```sh
flowshot --print-bind-help
```

## Desktop Support

| Desktop Environment | Capture Backend | Cursor Positioning | Status |
|---|---|---|---|
| **Hyprland** | ext-image-copy-capture | Native ICC cursor session + IPC | Verified live |
| **QuickShell (COSMIC)** | ext-image-copy-capture | Native ICC cursor session | Verified live |
| **X11 (i3, bspwm, XFCE)** | Native X11 (MIT-SHM fast path) | Native XFixes cursor | Verified live (minor cosmetic issues) |
| **Sway / wlroots** | ext-image-copy-capture / screencopy | Native ICC cursor session | Implemented, not verified by us |
| **KDE Plasma** | KWin ScreenShot2 D-Bus | Native cursor tracking | Implemented, not verified by us |
| **COSMIC** | ext-image-copy-capture | Native ICC cursor session | Implemented, not verified by us |
| **GNOME** | XDG Portal (ScreenCast / Screenshot) | Portal cursor support | Implemented, not verified by us |

Three of these have been exercised on real hardware by the people who maintain
this project: Hyprland, QuickShell, and X11. Everything below that line is
implemented and covered by tests, but nobody here has run it on a real desktop
yet: GNOME, COSMIC, Sway and KDE in particular. If you are on one of them, an
issue naming your compositor and version is the most useful thing you can file
here.

FlowShot probes your live session automatically at runtime, picking the fastest
available backend without requiring manual configuration.

**Windows:** a native build exists and ships in the release artifacts, but no
maintainer here runs Windows, so none of it has been exercised on real hardware.
DXGI/WGC capture, the overlay, and the save dialog are implemented against the
documented APIs. If you are a Windows user, telling us what works and what
doesn't is the fastest way to make that build honest.

## Configuration

Settings can be changed graphically in the settings window (`flowshot settings`) or in `~/.config/flowshot/flowshot.toml`.

```toml
config_version = 4

[save]
path = ""                    # default: system Pictures directory
filename_pattern = "%F_%H-%M"
actions = ["copy"]           # default actions: copy, save, pin, upload

[ui]
accent_color = "#6366F1"
cursor_style = "crosshair"   # "crosshair", "pointer", or "full-axis"

[daemon]
tray = true
```

For the complete list of options, ranges, and defaults, see the [Configuration Reference](docs/config-reference.md).

## Privacy & Crash Reporting

FlowShot is built with a privacy-first approach:
- **No data leaves your computer without consent:** You are prompted on first launch to choose whether to send crash diagnostics. If you decline or dismiss the prompt, error reporting is completely inactive, even anonymously.
- **No image data is ever sent:** Screenshots, image buffers, and file paths are never transmitted.
- **Self-hosted and minimal:** Crash reports are sent to our self-hosted Sentry instance, not third-party advertising or analytics networks.
- You can review or toggle reporting preferences at any time in `flowshot settings` under the General tab.

## Project Status

FlowShot is under active development towards v1.0:
- Native Wayland (ICC, screencopy, KWin) and native X11 backends are implemented; Hyprland, QuickShell and X11 are verified live.
- GPU-accelerated rendering with automatic CPU fallback is active.
- AUR packages (`flowshot-bin` and `flowshot-git`) are maintained.
- Codebase maintains comprehensive test suites, platform purity checks, and strict formatting.

Contributions, bug reports, and feedback are welcome!

## Development

```sh
just check                    # Full quality gate: fmt + clippy + tests + deny
cargo test --workspace        # Run the complete test suite
cargo build --workspace       # Build all crates
```

Contributor guidelines: [CONTRIBUTING.md](CONTRIBUTING.md).  
Architecture Decision Records: [docs/architecture/](docs/architecture/).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for details. Third-party notices: [NOTICES](NOTICES).

---

## Disclaimer

Written largely by a language model, like most code you'll open this year. What that sentence does not tell you is that someone here still maintains it: issues get a reply, reports get reproduced, and the ones that are real get fixed.

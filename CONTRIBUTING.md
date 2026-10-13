# Contributing to FlowShot

FlowShot is a native screenshot and annotation tool for Linux, supporting
both Wayland and X11. This document covers the build, test, and commit
conventions used across the project.

## Prerequisites

### Build-time dependencies
- Rust stable (channel pinned in `rust-toolchain.toml`)
- C toolchain (gcc/clang)
- `clang` (for libclang to generate PipeWire FFI bindings)
- `pkg-config`
- PipeWire development headers (`pipewire` on Arch/Fedora, `libpipewire-0.3-dev` on Debian/Ubuntu)

### Runtime requirements
- A Wayland compositor (Hyprland, Sway, KDE Plasma, GNOME, COSMIC, niri) or an X11 server/window manager (i3, bspwm, XFCE)
- D-Bus session bus
- PipeWire
- A Vulkan-capable GPU driver (Mesa or proprietary) for hardware rendering (software rendering fallback is built-in)
- For portal features: `xdg-desktop-portal` plus your desktop's portal backend
- For the system tray: an SNI host (waybar tray, Plasma system tray, GNOME AppIndicator)

## Building

```sh
cargo build --workspace
```

The workspace contains eight member crates:

| Crate | Purpose |
|---|---|
| `flowshot-core` | Platform-free config, physical geometry, scene graph, design tokens |
| `flowshot-capture` | Capture backend trait and negotiation ladder |
| `flowshot-capture-wayland` | Wayland capture backends (ext-image-copy-capture, wlr-screencopy, KWin D-Bus, portal) |
| `flowshot-capture-x11` | X11 capture backend (GetImage with MIT-SHM fast path, RANDR, XFixes cursor) |
| `flowshot-ui` | Winit overlay, annotation editor, pins, settings, wgpu and CPU raster renderer |
| `flowshot-actions` | Export actions (save, copy, pin, upload, desktop notifications) |
| `flowshot-daemon` | Session-bus daemon, single-instance lifecycle, system tray, shortcuts |
| `flowshot-cli` | Command-line interface and D-Bus client forwarding |

## Testing

```sh
cargo test --workspace
```

The project maintains 1350+ tests covering unit tests, property testing (proptest),
wire round-trips, integration tests, and headless QA suites.

Key test targets:
- **Config**: `cargo test -p flowshot-core config::` (schema, migrations, serialization round-trips)
- **Geometry**: `cargo test -p flowshot-core geometry::` (physical-first coordinates, proptest)
- **Scene**: `cargo test -p flowshot-core scene::` (annotation objects, undo/redo stack)
- **Renderer Parity**: `cargo test -p flowshot-ui --test parity` (cross-rasterizer pixel parity)
- **Wire Round-trip**: `cargo test -p flowshot-cli --test wire_roundtrip` (CLI to daemon D-Bus encoding)

### Headless Visual QA

To test the UI and renderer without opening live windows on your desktop:

```sh
# Run the complete headless visual QA suite:
cargo run -p flowshot-ui --example qa_bundle --features test-drive

# Run end-to-end headless integration:
cargo run -p flowshot-ui --example e2e_headless --features test-drive
```

### Doc-Sync Test

`crates/flowshot-core/tests/doc_sync.rs` asserts that every config field in
`config.rs` is documented in `docs/config-reference.md`. When adding or
modifying a configuration field, update the reference doc or the test will fail.

## Quality Gates

Before committing any changes, run the project gate suite:

```sh
just check
```

This gate runs the following checks in order:

1. `cargo fmt --check`: code formatting
2. `cargo clippy --workspace --all-targets -- -D warnings`: strict workspace lints
3. `cargo test --workspace`: full test suite
4. `./scripts/purity-gate.sh`: architecture purity (ensures core, capture, and ui remain platform-free)
5. `cargo deny check`: dependency advisories, bans, and license validation

All checks must pass. Continuous Integration (`.github/workflows/ci.yml`) executes the same suite.

## Engineering Standards

- **Code Safety**: `#![forbid(unsafe_code)]` holds across all crates.
- **Error Handling**: Typed errors via `thiserror` in all library crates; `anyhow` is restricted to binary entry points. Never swallow errors silently.
- **Documentation**: Module-level `//!` header comments on all modules. Rustdoc builds with zero warnings (`#![deny(missing_docs)]` on `flowshot-capture`, warn elsewhere).
- **No Unwrapped Panics**: `unwrap()` and `expect()` are denied by workspace lints in library code (use typed results).

## Commit Conventions

- **Conventional Commits**: `type(scope): subject`
  - Examples: `fix(daemon): exempt settings from session lock`, `feat(ui): add crosshair cursor option`
- **Scope vocabulary**: `core`, `capture`, `wayland`, `x11`, `ui`, `editor`, `actions`, `daemon`, `cli`, `packaging`, `docs`, `gates`
- **Quality gate required**: `just check` must be green before committing.
- **Linear history**: Keep commits atomic and focused.

## Project Resources

- [README.md](README.md): Overview, quick start, desktop support
- [docs/](docs/): Setup guides for Hyprland, Sway, KDE, GNOME, and X11
- [docs/config-reference.md](docs/config-reference.md): Complete configuration options and defaults
- [docs/architecture/](docs/architecture/): Architecture Decision Records (ADRs)

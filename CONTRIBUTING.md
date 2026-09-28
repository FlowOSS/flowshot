# Contributing to FlowShot

FlowShot is a screenshot tool for Linux/Wayland. This document covers the
build, test, and commit conventions used during development.

## Prerequisites

- Rust stable (channel pinned in `rust-toolchain.toml`)
- A Wayland compositor (Hyprland, sway, COSMIC, niri, KDE Plasma, or GNOME)
- A D-Bus session bus
- For portal features: `xdg-desktop-portal` plus your desktop's portal backend
- For the tray: an SNI host (waybar tray, Plasma system tray, GNOME AppIndicator)

## Building

```sh
cargo build --workspace
```

The workspace contains seven crates:

| Crate | Purpose |
|---|---|
| `flowshot-core` | Config, geometry, scene model, design tokens |
| `flowshot-capture` | Capture backend trait + ladder |
| `flowshot-capture-wayland` | Wayland capture backend (ext-image-copy-capture, wlr-screencopy) |
| `flowshot-ui` | Overlay, editor, pins, settings, renderer |
| `flowshot-actions` | Export actions (save, copy, pin, upload, notify) |
| `flowshot-daemon` | Session-bus daemon, lifecycle, tray, shortcuts |
| `flowshot-cli` | Command-line interface |

## Testing

```sh
cargo test --workspace
```

The workspace has 1200+ tests across unit, property (proptest), integration,
and headless QA suites. Key test categories:

- **Config**: `cargo test -p flowshot-core config::` — schema, migration, round-trip
- **Geometry**: `cargo test -p flowshot-core geometry::` — physical-first coordinates, proptest
- **Scene**: `cargo test -p flowshot-core scene::` — annotation objects, undo/redo
- **Parity**: `cargo test -p flowshot-ui --test parity` — renderer cross-rasterizer parity
- **Capability map**: `cargo test -p flowshot-cli --test parity_matrix` — CLI↔daemon mapping

### Doc-sync test

`crates/flowshot-core/tests/doc_sync.rs` asserts every config field in
`config.rs` appears in `docs/config-reference.md`. When adding a config field,
update the reference doc or the test fails.

## Quality gates

Before every commit, run:

```sh
just check
```

This runs (in order):

1. `cargo fmt --check` — formatting
2. `cargo clippy --workspace --all-targets -- -D warnings` — lints
3. `cargo test --workspace` — all tests
4. `./scripts/purity-gate.sh` — crate-boundary purity (no wayland/x11/dbus leaks in core/capture/ui)
5. `cargo deny check` — license policy

All must pass. CI (`.github/workflows/ci.yml`) runs the same gates.

## Engineering standards (Amendment #4)

- **Lints**: Every crate inherits `[workspace.lints]` via `lints.workspace = true`
- **Safety**: `#![forbid(unsafe_code)]` in all lib crates except `flowshot-capture-wayland`
- **Errors**: `thiserror` in all lib crates; `anyhow` only at binary top level
- **Docs**: `//!` module header on every module; `#![deny(missing_docs)]` on core + capture
- **No panics in lib sources** outside tests

## Commit conventions

- **Conventional Commits**: `type(scope): subject`
- **Scope vocabulary**: `workspace`, `core`, `capture`, `wayland`, `ui`, `editor`, `actions`, `daemon`, `shell`, `cli`, `packaging`, `docs`, `gates`, `e2e`
- **Gate before every commit**: `just check` must be green
- **Linear history** on `main`

## QA evidence conventions

Live-session QA produces evidence files in `.omo/evidence/` (gitignored), one
bundle per verified change. The evidence policy and its conventions are
documented in `docs/verification.md`.

Visible-window QA items (requiring a user-present compositor session) are
queued in `.omo/evidence/gui-qa-batch.md` and treated as user-gated, not
failures.

## Configuration

FlowShot reads one TOML file: `~/.config/flowshot/flowshot.toml`. The full
schema is documented in `docs/config-reference.md`. A missing or corrupt file
never breaks a capture: defaults apply and a warning is logged.

## D-Bus API

The daemon exposes a session-bus API documented in `docs/dbus-api.md`. The CLI
forwards to the daemon over D-Bus when a running instance is detected.

## Project resources

- [README.md](README.md) — overview, quick start, desktop support
- [docs/](docs/) — per-desktop setup guides, architecture ADRs, config reference
- [docs/verification.md](docs/verification.md) — QA evidence policy
- [docs/architecture/](docs/architecture/) — architectural decision records

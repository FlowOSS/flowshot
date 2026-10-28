# Contributing to FlowShot

FlowShot is a native screenshot and annotation tool for Linux, supporting
both Wayland and X11. This document covers the build, test, and commit
conventions used across the project.

## Prerequisites

### Build-time dependencies
- Rust stable (channel pinned in `rust-toolchain.toml`)
- `clang` (libclang generates the PipeWire FFI bindings) and `gcc` for the C bits
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

The workspace contains nine member crates:

| Crate | Purpose |
|---|---|
| `flowshot-core` | Platform-free config, physical geometry, scene graph, design tokens |
| `flowshot-capture` | Capture backend trait and negotiation ladder |
| `flowshot-capture-wayland` | Wayland capture backends (ext-image-copy-capture, wlr-screencopy, KWin D-Bus, portal) |
| `flowshot-capture-x11` | X11 capture backend (GetImage with MIT-SHM fast path, RANDR, XFixes cursor) |
| `flowshot-capture-windows` | Windows capture ladder: WGC, DXGI desktop duplication, GDI `BitBlt` fallback |
| `flowshot-ui` | Winit overlay, annotation editor, pins, settings, wgpu and CPU raster renderer |
| `flowshot-actions` | Export actions (save, copy, pin, upload, desktop notifications) |
| `flowshot-daemon` | Session-bus daemon, single-instance lifecycle, system tray, shortcuts |
| `flowshot-cli` | Command-line interface and D-Bus client forwarding |

## Testing

```sh
cargo test --workspace
```

The project maintains 1,500+ tests (1,537 as reported by `cargo test --workspace`
on 2026-10-28) covering unit tests, property testing (proptest), wire round-trips,
golden rendering fixtures, private-bus D-Bus stubs, HTTP stubs for upload, and
headless QA suites.

Tests land with the code: a module is not "done" when it compiles, it is done
when a test that would fail without it is green. See
[docs/verification.md](docs/verification.md) for the evidence classes and what
"verified" means here.

Key test targets:
- **Config**: `cargo test -p flowshot-core config::` (schema, migrations, serialization round-trips)
- **Geometry**: `cargo test -p flowshot-core geometry::` (physical-first coordinates, proptest)
- **Scene**: `cargo test -p flowshot-core scene::` (annotation objects, undo/redo stack)
- **Renderer Parity**: `cargo test -p flowshot-ui --test parity` (cross-rasterizer pixel parity)
- **Wire Round-trip**: `cargo test -p flowshot-cli --test wire_roundtrip` (CLI to daemon D-Bus encoding)
- **Windows-target check**: `just check-windows` (typechecks the Windows crate set and re-runs the purity gate)

Test placement is per-crate, not workspace-wide: inline `#[cfg(test)] mod tests`
is the norm everywhere except `flowshot-ui`, which uses sibling `tests.rs` and
`*_tests.rs` files.

### Headless Visual QA

To test the UI and renderer without opening live windows on your desktop:

```sh
# Run the complete headless visual QA suite (the last argument is the output dir):
cargo run -p flowshot-ui --example qa_bundle --features test-drive /tmp/qa_bundle

# Run end-to-end headless integration:
cargo run -p flowshot-ui --example e2e_headless --features test-drive
```

`flowshot-ui/tests/parity.rs` **skips with a printed message when no GPU adapter
exists**, so a green headless run proves nothing about renderer pixel parity —
check the log names a real adapter.

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

All checks must pass before a commit lands.

CI (`.github/workflows/ci.yml`) is **not** the same suite. The Linux job runs
purity first, then fmt, then clippy, then `cargo test` **without `--workspace`**,
then `cargo deny` via `EmbarkStudios/cargo-deny-action@v2`; a second Windows job
runs a scoped crate list (`-p flowshot-core -p flowshot-capture -p
flowshot-capture-windows -p flowshot-ui -p flowshot-actions -p flowshot-daemon
-p flowshot-cli`) and builds the release binaries. Local `just check` is
therefore strictly stronger than CI: green locally does not imply green on
`main`, and green on `main` does not imply the whole workspace was tested.

CI's deny step queries the same live OSV database as `cargo deny check`, so a
newly published advisory turns CI red on the next push. When an advisory has an
upgrade path, fix it with a targeted `cargo update -p <pkg>` and commit the lock
move as `chore(deps)` — never add it to `deny.toml`'s ignore list, which is
reserved for stacks with no safe upgrade.

## Engineering Standards

- **Code Safety**: `#![forbid(unsafe_code)]` holds across all crates.
- **Error Handling**: Typed errors via `thiserror` in all library crates; `anyhow` is restricted to binary entry points. Never swallow errors silently.
- **Documentation**: Module-level `//!` header comments on all modules. Rustdoc builds with zero warnings (`#![deny(missing_docs)]` on `flowshot-capture`, warn elsewhere).
- **No Unwrapped Panics**: `unwrap()` and `expect()` are denied by workspace lints, tests included.
- **Lint suppression**: `#[expect(lint, reason = "…")]` with a reason is the norm; bare `#[allow]` survives only in a handful of older spots (`core`, `flowshot-ui`, one in `daemon`).
- **File size**: the convention is a 250 pure-LOC ceiling per file; nothing in `just check` measures it, and a few files exceed it deliberately (`geometry.rs`, `scene.rs`, `config.rs`).
- **No runtime shell-outs**: no grim/slurp/wtype spawning in shipped code, and no spawning in library code. Those binaries are QA oracles only.
- **Graphics**: never `Backends::all()` — `ui/src/gpu/instance.rs` pins `PRIMARY` because NVIDIA's `eglTerminate` segfaults even on a Vulkan adapter. egui is a guest inside the wgpu renderer and may be used only by `settings`, `launcher`, and `consent`.
- **Packaging is owner-gated**: AUR / `.deb` / AppImage and CI packaging move only on the maintainer's explicit word, never on an automated prompt or an agent's judgment.

## Commit Conventions

- **Conventional Commits**: `type(scope): subject`
  - Examples: `fix(daemon): exempt settings from session lock`, `feat(ui): add crosshair cursor option`
- **Scope vocabulary**, as actually used in history: `core`, `capture`, `wayland`, `ui`, `editor`, `daemon`, `cli`, `ci`, `release`, `packaging`, `deps`, `windows`, `dxgi`, `build`, `parity`, `docs`. `chore(deps)` is skipped from the generated changelog by `cliff.toml`, which is the right subject for a lockfile bump.
- **Quality gate required**: `just check` must be green before committing.
- **Linear history**: Keep commits atomic and focused.

## Project Resources

- [README.md](README.md): Overview, quick start, desktop support
- [docs/](docs/): Setup guides for Hyprland, Sway, KDE, GNOME, and X11
- [docs/config-reference.md](docs/config-reference.md): Complete configuration options and defaults
- [docs/architecture/](docs/architecture/): Architecture Decision Records — adr-001 through adr-007
- [docs/verification.md](docs/verification.md): what "verified" means, the evidence classes, and the landed-bundle records
- [docs/dbus-api.md](docs/dbus-api.md): the daemon's session-bus interface
- [docs/codebase-comparison.md](docs/codebase-comparison.md): FlowShot vs Flameshot, measured
- [docs/porting-roadmap.md](docs/porting-roadmap.md) and [docs/upgrade-evaluation.md](docs/upgrade-evaluation.md): dependency and stack plans
- [docs/research/](docs/research/): protocol and compositor behaviour write-ups

## When a new default ships

Additive config keys need nothing: `#[serde(default)]` covers them. A changed
value inside a list a user may have pinned is the opposite case — `toolbar_buttons`
etc. are pinned `Vec<String>`, so an existing config never sees the new default.
That is what `MIGRATIONS` in `flowshot-core/src/config.rs` is for: bump
`CONFIG_VERSION`, register `(old_version, migrate_fn)`, append the new ids without
re-laying out a user's own order, and never materialize a key the user did not
write. Bumping `CONFIG_VERSION` also means updating `docs/config-reference.md`
and `flowshot-cli/src/config_man.rs` — `tests/doc_sync.rs` fails if you miss one.

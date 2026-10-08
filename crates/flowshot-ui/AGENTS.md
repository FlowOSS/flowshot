# crates/flowshot-ui

Crate boundary, distinct domain: 270 files, 5 top dirs, 89% Rust; `lib.rs` re-exports 87 symbols over 19 `pub mod`s.

## OVERVIEW

The winit 0.30 + wgpu 30 overlay runtime: multi-monitor capture overlay, the
batched 2D renderer (lyon tessellation + cosmic-text glyph atlas), the
selection engine, the annotation editor, the widget kit, the pin windows, and
the three embedded-egui surfaces (settings / launcher / consent). `rlib` only,
edition 2024, `#![forbid(unsafe_code)]` + `#![warn(missing_docs)]`.

## STRUCTURE

```
flowshot-ui/
├── src/       # everything — see src/AGENTS.md for the module map
├── tests/     # parity.rs (1692 L, cross-rasterizer) + 4 offscreen suites
├── examples/  # 15 QA harnesses; 4 need --features test-drive
├── icons/     # 23 source SVGs → build.rs rasterizes the atlas
├── fonts/     # vendored Inter x3 — include_bytes! by egui_host/theme ONLY
└── build.rs   # resvg at BUILD time; no runtime SVG stack
```

## WHERE TO LOOK

| Task | Location |
|------|----------|
| Module architecture, coordinate spaces, conventions | `src/AGENTS.md` |
| Add an icon | drop `icons/<kebab-name>.svg` using `currentColor`; rebuild regenerates the `Icon` enum variant + `rect()` |
| Cross-rasterizer render correctness | `tests/parity.rs` — wgpu vs a DEV-ONLY tiny-skia reference |

## FEATURES

- `test-drive` — public synthetic-input seam (`OverlayCore::inject_event`,
  `OverlayHandle::inject_event`) so mouse paths are QA-able without ydotool.
  flowshot-daemon re-exports it as its own `test-drive`.
- `perf-trace` — live frame-timing profiler (`src/perf.rs`, p50/p95/p99 over
  tracing). Compiled OUT by default; production carries zero instrumentation.

## BUILD SCRIPT

`build.rs` rasterizes every `icons/*.svg` (`currentColor` → `#FFFFFF`) into an
8-column atlas of 28px cells (24px icon + 2px padding), and `assets/logo.svg`
into `logo_128.rgba` (premultiplied RGBA for egui's `ColorImage`). Emits
`$OUT_DIR/icons.rs`; `rerun-if-changed` covers `icons/` and the logo. It is the
only shipped-code file with a file-scope `#![allow(clippy::unwrap_used, ...)]`.

## ANTI-PATTERNS (THIS CRATE)

- **Purity-gated.** `src/` may not import wayland/x11/dbus/pipewire/unix-FFI
  crates, use `winit::platform::*` or `*ExtWayland`, or carry any
  `cfg(target_os/target_family/target_arch/unix/...)`. The single env probe
  (`WAYLAND_DISPLAY` via `std::env::var_os`) is allowlisted by construction.
- `flowshot-capture-wayland` and `flowshot-actions` are **dev-dependencies
  only** (examples/tests composition; the two allowlist entries exist for
  them). The lib talks to `flowshot-capture` — the contract crate — and never
  to the platform crate.
- `tiny-skia` / `image` are DEV-ONLY parity oracles, never a runtime CPU fallback.
- `egui-winit` is deliberately absent (not in `Cargo.lock`): the hand-feed bridge `egui_host::input` is the tested surface.

## COMMANDS

```bash
just check                    # fmt + clippy -D warnings + test + purity + deny
cargo test -p flowshot-ui     # this crate
cargo run -p flowshot-ui --example qa_bundle --features test-drive
```

## NOTES

- `tests/parity.rs` SKIPS every test when no GPU adapter exists (not even lavapipe) — headless CI stays green, this machine runs real.
- GPU policy lives in one place: `src/gpu/instance.rs` pins
  `Backends::PRIMARY` (never `all()` — NVIDIA's EGL teardown segfaults), and
  `src/adapter.rs` floors `max_texture_dimension_2d` at 4096.
- Two font paths: `egui_host/theme` embeds the vendored Inter; the overlay
  resolves families from cosmic-text's system DB (fontconfig fallback = the
  CJK path). A new `fonts/*.ttf` therefore changes ONLY the egui surfaces.

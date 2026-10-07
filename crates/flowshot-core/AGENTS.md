# crates/flowshot-core

**Generated:** 2026-10-06 | **Commit:** 36714a7 | **Branch:** main
Score 12 (distinct domain) — 13 files / 6.1k LOC, 6 `pub mod`s, imported by all
six sibling crates. Purity-gated `rlib`, no build script.

## OVERVIEW

The platform-free vocabulary: versioned TOML config, physical-first geometry
algebra, the annotation scene graph with snapshot undo, and the design tokens.
Nothing here knows a display server exists.

## STRUCTURE

```
flowshot-core/
├── src/config.rs    # 435 pure LOC — [capture][save][editor][tools.*][pin][upload][ui][daemon][telemetry]
├── src/geometry.rs  # 656 pure LOC — OutputInfo/OutputLayout, transforms, crop algebra
├── src/scene.rs     # 602 pure LOC — Scene, UndoStack, ToolObject dispatch
│   └── scene/       # arrow.rs, counter.rs, objects.rs, test_support.rs — per-tool paint geometry
├── src/tokens.rs    # DesignTokens: colors, radii, spacing, typography, easing
├── src/types.rs     # leaf value types
├── src/error.rs     # CoreError
└── tests/doc_sync.rs
```

## WHERE TO LOOK

| Task | Location |
|------|----------|
| Add or rename a config key | `config.rs`, then `docs/config-reference.md` — `tests/doc_sync.rs` fails CI until the doc lists the key |
| Multi-monitor / mixed-DPI math | `geometry.rs`; `OutputLayout::crop_rects` is the region-capture contract consumed by both `flowshot-ui` and `capture-wayland/src/stitch.rs` |
| New annotation tool | `scene.rs` (`ToolObject` variant + undo) then `scene/<tool>.rs` for paint geometry |
| Any color, radius, spacing, easing | `tokens.rs` — the UI derives every visual constant from `DesignTokens` |
| Config schema versioning / migration | `CONFIG_VERSION` in `config.rs` |

## CONVENTIONS

- **Physical pixels first.** Geometry carries per-output scale and transform;
  an averaged scale is never stored and logical values are always derived.
- **Undo is snapshot-based** (`UndoStack`, `DEFAULT_UNDO_LIMIT`): tools replace
  object state, so no inverse operation is ever written.
- Tool geometry is a **clean-room Flameshot reimplementation** — each
  `scene/*.rs` header cites the upstream file and commit (`arrowtool.cpp
  @ 2d478061`). Keep the citation when you touch the math; it is the audit trail.
- `tests/doc_sync.rs` serializes `Config::default()`, extracts every leaf key,
  and asserts each appears backtick-quoted in `docs/config-reference.md`.
- `proptest` (dev-dep) drives the geometry transform and scene round-trip
  properties. Narrow `#[allow(clippy::cast_*)]` with a trailing reason comment
  marks the px/scale cast boundaries; in this subtree `core` is the only crate
  still spelling those `#[allow]` rather than `#[expect(.., reason=)]`.

## ANTI-PATTERNS (THIS CRATE)

- Zero platform coupling. `scripts/purity-gate.sh` scans `src/` **including
  inline test modules** for wayland/x11/dbus/pipewire/unix-FFI imports and any
  `cfg(target_os|target_family|target_arch|target_env|unix|...)`.
- No secrets in the schema: the telemetry DSN is a build-time constant in
  `flowshot-daemon/src/telemetry`, NEVER a `[telemetry]` config key.
- Never re-declare geometry types downstream — `flowshot-capture` reuses
  `OutputInfo`/`Rect` from here by rule, and a duplicate is a review defect.

## COMMANDS

```bash
cargo test -p flowshot-core                  # inline suites + doc_sync
cargo test -p flowshot-core --test doc_sync  # the config<->doc gate alone
```

## NOTES

- The lib-code outliers over the 250 pure-LOC ceiling all live here:
  `geometry.rs` (656), `scene.rs` (602), `config.rs` (435). The split precedent
  is `scene.rs` + `scene/`; extending it to geometry/config is open work.
- `chrono`, `bytes` and `image` are declared in `Cargo.toml` but imported
  nowhere in `src/` — dead dependency edges no gate currently catches.
- `#![warn(missing_docs)]` here, not `deny` (only `flowshot-capture` denies).

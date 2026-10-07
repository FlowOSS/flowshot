# crates/flowshot-capture

**Generated:** 2026-10-06 | **Commit:** 36714a7 | **Branch:** main
Score 12 (distinct domain) — 9 files / 1.8k LOC, the smallest crate and the most
load-bearing boundary: it owns the capture vocabulary and nothing else.
Purity-gated `rlib`, `#![deny(missing_docs)]` (the only crate that denies).

## OVERVIEW

The contract between the platform-free world and the compositor: the
`CaptureBackend` async trait, CPU-side frame delivery, the negotiation ladder,
and a fixture-driven mock so downstream crates test without a session.

## STRUCTURE

```
flowshot-capture/
├── src/backend.rs    # CaptureBackend (async_trait), CaptureOpts, PermissionResult
├── src/frame.rs      # Frame, FrameBuffer, FrameFormat, OutputRef — physical-first placement
├── src/kind.rs       # BackendKind incl. roadmap variants (X11) that report unsupported
├── src/negotiate.rs  # NEGOTIATION_LADDER, CapabilityProbe, DesktopEnv, negotiate()
├── src/cursor.rs     # CursorEvent, CursorStream
├── src/mock.rs       # MockBackend + src/mock/pixels.rs, src/fixtures/*.png
└── src/error.rs      # CaptureError (NoBackendAvailable, ...)
```

## THE LADDER (the crate's reason to exist)

`NEGOTIATION_LADDER: [BackendKind; 5]`, strongest first —
1 `ext-image-copy-capture-v1` → 2 `wlr-screencopy-v1` → 3 `org.kde.KWin.ScreenShot2`
→ 4 portal `ScreenCast` → 5 portal `Screenshot`. `negotiate(probe, config_override)`
filters the ladder by what the probe observed; `[capture].force_backend` wins
outright, and a forced kind the probe does not support fails fast with a typed
error naming it instead of dying later at bind time. Rungs 3-5 are D-Bus
services the Wayland registry cannot see, so the probe is fed by separate
D-Bus checks in `flowshot-capture-wayland`.

## WHERE TO LOOK

| Task | Location |
|------|----------|
| Add a backend | `kind.rs` variant + ladder position here; the implementation lives in `flowshot-capture-wayland` |
| Change frame/pixel delivery | `frame.rs` — per-output scale and transform travel with the frame, never an averaged scale |
| Test a consumer with no compositor | `MockBackend` (`mock.rs` + `mock/pixels.rs`, fixtures in `src/fixtures/`) |
| Desktop detection vocabulary | `DesktopEnv` in `negotiate.rs`; the sniffing itself is `capture-wayland/src/desktop.rs` |

## ANTI-PATTERNS (THIS CRATE)

- Protocols are **named in diagnostics only** — no wayland/dbus import may
  appear here; the purity gate scans `src/` including inline tests.
- No geometry duplication: `OutputInfo`, `Rect` and `Transform` come from
  `flowshot_core::geometry` and are re-used verbatim.
- Roadmap `BackendKind` variants (X11) must stay inert: `CapabilityProbe::supports`
  reports `false` for them in v1 — never let one enter the ladder's live path.
- `missing_docs` is `deny`, so a new public item without a doc comment fails the
  build, not just the lint pass.

## COMMANDS

```bash
cargo test -p flowshot-capture   # inline suites; tokio is a dev-dep only
just purity                      # this crate is one of the three gated
```

## NOTES

- The trait is `async_trait`-based but the crate ships no runtime; callers
  (daemon executor, CLI one-shot) supply the tokio context.
- `MockBackend` is what makes `flowshot-daemon`'s capture pipeline and
  `flowshot-ui`'s offscreen QA harnesses runnable headless — changing its
  fixture semantics silently changes both.

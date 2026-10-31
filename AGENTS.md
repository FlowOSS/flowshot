# AGENTS.md — FlowShot agent handoff

> Working-tree orientation for coding agents: the shipped knowledge base
> (this root file plus the per-crate `AGENTS.md` files). Keep it truthful
> against the tree; update it when structure, gates, or rules change.

## OVERVIEW

FlowShot — screenshot + annotation for Linux/Wayland; Flameshot parity with the Wayland breakage
fixed (multi-monitor spanning, cursor-aware capture, mixed-DPI correctness, reliable clipboard).
Rust workspace, GPL-3.0-or-later, 9 crates. Brand `org.flowoss.FlowShot`, accent `#6366F1` /
contrast `#0F172A`. Stack: wgpu 30 + egui 0.36 + zbus 5 + ashpd 0.13 + reqwest 0.13, winit 0.30.
The original 46-item build plan is COMPLETE except owner-gated packaging (AUR/.deb/AppImage + CI).

## STRUCTURE

```
crates/
  flowshot-core/             platform-free: versioned TOML config, physical-first geometry, scene + undo, tokens
  flowshot-capture/          the contract: CaptureBackend, Frame, NEGOTIATION_LADDER, MockBackend
  flowshot-capture-wayland/  5 backends: ICC ext-image-copy-capture, wlr-screencopy, KWin ScreenShot2, portal
  flowshot-capture-x11/      the X11 rung: x11rb GetImage + MIT-SHM fd-passing, RANDR outputs, XFixes cursor
  flowshot-capture-windows/  the Windows backend: its OWN 3-tier ladder (WGC -> DXGI desktop duplication -> GDI BitBlt for Win7/RDP/VM); off the workspace ladder, picked by cfg(target_os)
  flowshot-ui/               winit+wgpu overlay, lyon renderer + the CPU raster fallback (render/software), cosmic-text atlas, selection, editor, egui_host
  flowshot-actions/          export (save/patterns/encode), daemon-owned clipboard, upload, pins
  flowshot-daemon/           zbus service, single-instance, executor, tray (SNI), shortcuts, notify, telemetry
  flowshot-cli/              the `flowshot` leaf binary: clap, D-Bus forwarding, exit codes 0-6, completions
assets/ logo.svg (truth) + derived logo.png (2048, never hicolor)   scripts/ purity-gate.sh
docs/ adr-001..007 + README, setup-{gnome,hyprland,kde,sway,x11}, config-reference, dbus-api,
        verification (evidence classes + landed bundles), research/, codebase-comparison,
        porting-roadmap, upgrade-evaluation, refactor-opportunities
packaging/ flowshot.desktop.in + ICONS.md   tests/ parity_matrix.toml (F12, CI-validated)
```

Dep graph: `core` ← everything; `capture` ← wayland/x11/windows/cli/daemon/ui; `capture-x11` and
`capture-windows` ← daemon only; `ui` ← daemon only; `cli` leaf.
The daemon is the ONLY crate holding ui + capture-wayland + capture-x11 + capture-windows + actions —
the platform-integration layer.

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Per-crate / per-module rules | the 11 nested `AGENTS.md` | `flowshot-ui/src`, `capture-wayland/src`, `daemon/src` carry module maps; `flowshot-capture-windows` has none yet |
| Config + D-Bus surface | `core/src/config.rs`, `docs/config-reference.md`, `docs/dbus-api.md` | `CONFIG_VERSION` gates migration; exit-code table 0-6 lives in `flowshot-cli` |
| Flameshot parity | `tests/parity_matrix.toml` (1220 L) | validated by `cli/tests/parity_matrix.rs`; CLI rows REFERENCE `cli/tests/cli_capability_map.toml`, never duplicate |
| Icons / logo / desktop entry | `assets/logo.svg`, `ui/icons/*.svg`, `packaging/` | both `build.rs` rasterize via resvg; `ICONS.md` lists targets; `Icon=org.flowoss.FlowShot` |
| Editor + renderer | `ui/src/editor` (52 files, 10,854 code lines / 14,192 raw), `ui/src/render` | largest and most-referenced modules |
| CPU raster fallback + its policy | `ui/src/render/software` (renderer + `Presenter`), `ui/src/adapter.rs` (`software_path_for`) | the second rasterizer behind the same `DisplayList`; see adr-007 |
| History, rationale, QA evidence | `docs/architecture/` (ADRs), `docs/verification.md` | decisions + verification classes and landed-bundle records with inlined observed values |

## CODE MAP

`flowshot-ui/src/lib.rs` re-exports 99 symbols over 19 `pub mod`s (counted 2026-10-28 — recount before quoting an exact number).

| Symbol | Location | Role |
|--------|----------|------|
| `OverlayCore` / `OverlayRuntime` / `OverlayHandle`, `render::Renderer` / `DisplayList` / `Command` | ui | headless state + `inject_event` vs. the winit shell; the batched 2D renderer |
| `EditorState` / `Tool` / `ToolKind` / `ToolRegistry`, `SelectionState`, `ChromeState`, `InputRouter` / `WindowSlot`, `UiError` | ui | annotation framework, F27 selection spec, toolbar/wheel/HUD, coordinate mapping, typed errors |
| `CaptureBackend`, `Frame` / `FrameBuffer` / `FrameFormat`, `NEGOTIATION_LADDER` / `negotiate` / `CapabilityProbe` / `DesktopEnv`, `MockBackend` | capture | the trait, frame vocabulary, the 6-rung ladder (five Wayland + X11 last) |
| `CaptureThread`, `SessionSnapshot`, `ProtocolGlobals`, `resolve_cursor_pos`, `CapturedOutputs`, `IccBackend` / `ScreencopyBackend` / `KwinScreenShot2Backend` / `PortalScreenCast`+`ScreenshotBackend` | capture-wayland | capture thread, protocol state, cursor strategy, the five backends |
| `Config` / `CONFIG_VERSION`, `OutputLayout::crop_rects`, `Scene` / `UndoStack` / `ToolObject`, `DesignTokens` | core | config, mixed-DPI truth, scene graph, tokens |
| `Daemon`, `DaemonCommand` / `CommandSink`, `acquire_or_forward`, `LifecyclePolicy`, `DaemonState` / `PersistenceReasons`, `Notifier`, `TrayHandle`, `ExecCtx` | daemon | service, single-instance, lifecycle, test seams |
| `effective_actions`, `Clipboard`, `Uploader` / `Imgur`, `PinRegistry`; `Invocation`, `CliError` | actions, cli | post-capture pipeline; parsed call + typed failure (codes 0-6) |

## CONVENTIONS (deviations from stack defaults only)

- **Lints**: workspace `clippy::all` deny incl. `unwrap_used`/`expect_used` (tests included), pedantic
  warn. `thiserror` in libs, `anyhow` only at binary boundaries. Suppress via `#[expect(lint, reason = "…")]`
  — 144 `#[expect]` against 25 `#[allow]` workspace-wide (2026-10-28): `daemon` 70 expects but 1
  `#[allow]`, `core` 8 allows, `flowshot-ui` 16 allows. `core` is NOT the only `#[allow]` holder.
  `#![forbid(unsafe_code)]` holds in **every** crate: capture-wayland's exemption is unused headroom
  (zero `unsafe` blocks, no SAFETY comments, no allow-list); buffers are `memfd` + readback, not mmap.
- **Docs**: `//!` header on every module; `missing_docs` **deny on flowshot-capture only**, warn
  elsewhere; rustdoc builds with ZERO warnings.
- **Size**: the 250 pure-LOC ceiling is a convention nothing in `just check` measures. Outliers:
  `core/geometry.rs` 656, `core/scene.rs` 602, `core/config.rs` 435, `daemon/tray/dbusmenu.rs` 257.
- **Test placement**: inline `#[cfg(test)] mod tests` is the norm outside flowshot-ui (111 files
  counted 2026-10-28: core 8, capture 5, capture-wayland 30, capture-x11 8, capture-windows 2,
  actions 18, daemon 36, cli 4 — `flowshot-ui/src` adds 75 more);
  `flowshot-ui/src` uses sibling `tests.rs` / `*_tests.rs`. Module-root style also varies per crate —
  never state either workspace-wide.
- **Purity**: `scripts/purity-gate.sh` scans `src/` incl. inline tests for platform imports,
  `winit::platform::*`, `*ExtWayland`, any `cfg(target_*)`. Allowlist rows are
  `crate|location|substring` triples in `scripts/purity-allowlist.txt`, each needing a `#` WHY comment.
  The file holds exactly ONE row today: `flowshot-ui|[dev-dependencies]|flowshot-actions`.
- **Text/UI**: cosmic-text + swash through an in-crate atlas — **no glyphon** (no release pairs with
  the wgpu 30 pin). egui is a *guest* in the ui renderer (the Ruffle pattern), `egui-winit`
  absent from `Cargo.lock`; only `settings`, `launcher`, `consent` may use egui.
- **Commits**: Conventional Commits; gate with `just check` before committing.

## ANTI-PATTERNS (THIS PROJECT)

- **Packaging gate**: packaging (AUR/.deb/AppImage + CI) moves ONLY on the owner's explicit personal
  word — never on automation or prompts.
- **User-presence QA**: no visible windows unless the user is away or explicitly testing; offscreen
  renders + the `test-drive` injection seam are the default, timeout-bounded and self-reversing.
- **Evidence integrity**: never fabricate artifacts (a worker tried; the scripts were purged).
  Evidence files describe what EXISTS on disk. Generated scanner/audit reports likewise carried
  fabricated counts and false claims in one incident — re-derive from source, never trust them.
- Never `Backends::all()` — `ui/src/gpu/instance.rs` pins `PRIMARY`; NVIDIA's `eglTerminate` segfaults
  even on a Vulkan adapter. `ui/src/adapter.rs` floors `max_texture_dimension_2d` at 4096 (2048 rejected).
- No runtime shell-outs in shipped code (grim/slurp/wtype are QA oracles only); no spawning in lib code.
- Don't edit generated icon code — drop an SVG in `ui/icons/`; don't ship `assets/logo.png` as a
  hicolor size (`logo.svg` is the truth); don't restate one domain's rows in another (parity ↔ caps).
- Stale, do not repeat: `cli/src/lib.rs` claims the daemon re-parses forwarded argv with the clap
  surface; `daemon/src/execute/invoke.rs` hand-parses a frozen subset (reverse edge would be circular).

## UNIQUE STYLES

- **Physical pixels first**: geometry never mixes logical and physical; `OutputLayout::crop_rects` is
  the mixed-DPI source of truth, called by `ui/src/completion::composite_selection`, not `export.rs`.
- **Two font paths, no shared source**: vendored `ui/fonts/Inter-*.ttf` are `include_bytes!`-loaded
  ONLY by `egui_host/theme` (egui's `epaint_default_fonts` ride along unused — workspace inheritance
  forbids `default-features = false`). The overlay uses cosmic-text `FontSystem::new()` = the SYSTEM
  font DB (`family: None` resolves via fontconfig, the CJK path); `editor/paint.rs`'s `Some("Inter")`
  is a system-DB lookup, so a new `.ttf` in `fonts/` changes ONLY the egui surfaces.
- **Per-backend pattern** (capture-wayland): `<n>.rs` / `protocol.rs` plain-data sink / `dispatch.rs`
  / `run.rs` one-shot. **Three daemon seams** keep the service testable: `CommandSink`, `Notifier`,
  `PersistenceReasons`; one winit loop ⇒ the child-process rule. Tray SNI `IconName` stays EMPTY
  until the icon is installed (`daemon/src/tray/item.rs`).

## COMMANDS

```bash
just check     # THE gate: fmt + clippy --workspace --all-targets -D warnings + test + purity + deny
just gui       # interactive capture overlay (needs a Wayland session)
just daemon    # foreground daemon (tray needs [daemon] tray = true)
just settings  # settings window        just probe | pin | run
cargo test -p <crate>                 # 1,537 tests workspace-wide (measured 2026-10-28)
cargo run -p flowshot-ui --example {e2e_headless,qa_bundle} --features test-drive  # headless QA
# CI (.github/workflows/ci.yml) is WEAKER in one specific way: purity runs FIRST, then fmt, then
# clippy WITH --workspace/--all-targets, then `cargo test` WITHOUT --workspace, then cargo-deny via
# the official action; a second Windows job typechecks a scoped -p list and builds release binaries.
```

## NOTES

- **Gates**: 1,537 tests green (summed from `cargo test --workspace` on 2026-10-28); clippy/fmt/purity/deny clean.
  nested-runtime session fix, settings reworks x2, drag-freeze perf (Mailbox present +
  `pre_present_notify`), wheel-sizing feedback, rect size-slot fix, counter drag-aim pointer,
  discoverability (tooltip hotkeys + clickable aid chips), tray model (LMB captures / RMB menu + Open
  Save Path), logo wiring, opt-in telemetry (self-hosted Sentry, off by default, two-tier consent +
  first-launch dialog), the latest-stack upgrade, comment-slop purge, palette swatch wrapping fix,
  the CPU raster fallback (adr-007: `render/software` + `Presenter`, per-window policy in
  `adapter.rs`, QA override `FLOWSHOT_RENDER_PATH=software|gpu`, a selection-attached
  `chrome/status` pill stating `GPU`/`CPU fallback` gone when no slot fits, and
  `ui/tests/parity.rs` rows holding the CPU path to the GPU path pixel for pixel).
- **Landed after the plan**: pencil and filled rectangle as separate toolbar tools (`ToolKind::RectangleFill`, id `rectangle-filled`, unbound because its mnemonic F is the grid aid; `icons/square-filled.svg`; default toolbar now 14 buttons) and the `v3→v4` config migration that appends both ids to a pinned `toolbar_buttons` — a new default inside a user-pinned list is invisible to existing configs without such a step.
- `ui/tests/parity.rs` SKIPS every test with a printed message when no GPU adapter exists (not even
  lavapipe) — a green headless CI run proves nothing about renderer pixel parity.
- Load-sensitive, raised budgets: `daemon/tests/portal_shortcuts.rs` (30 s) and the KWin p2p stubs —
  both p2p sides must build CONCURRENTLY or the server blocks on the client's SASL handshake
  (`daemon/src/testsupport.rs`, `tests/broker.rs`). Raise budgets, never weaken assertions.
  `proptest`: core (geometry, scene), `cli/tests/wire_roundtrip.rs`, actions, daemon. `wiremock`: Imgur + telemetry.
- Dead deps: `core`'s phantom `chrono`, `bytes`, and `image` were purged (`actions`'s dead `rfd` and
  `capture-x11`'s phantom `x11rb-protocol` were removed earlier). `FileDialogSink`'s production impl is a
  pictures-directory stand-in in `daemon/src/execute/post/sinks.rs`.
- Deep sub-trees deliberately carry no own `AGENTS.md`: `ui/src/editor`, `settings`, `render`,
  `chrome`, `selection`, `pins`, `widgets`, `launcher`, `consent`; `editor` is the top candidate
  if that ever changes.
- `.omo/` is untracked, gitignored agent runtime state (plans, notepads, evidence, session files),
  local to each working machine — never committed, never referenced from shipping files. The
  durable project record lives in `docs/architecture/` (ADRs), `docs/verification.md` (QA classes +
  landed bundles with inlined observed values), and this knowledge base.

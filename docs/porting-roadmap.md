# Porting roadmap

Status: Phase 1 (X11) **SHIPPED** - headless capture 2026-10-04,
interactive UI (Phase B) 2026-10-06 (`crates/flowshot-capture-x11`;
verification records with inlined observed values:
[verification.md](verification.md)).
Every other platform remains roadmap only: **no code exists** for it and
none is planned for the first release. This document is
the concrete entry-point map promised by
[ADR-006](architecture/adr-006-cross-platform-gates.md) (draft findings
F19/F20/F24). Crate versions were spot-checked against
crates.io on 2026-09-28 (record at the bottom).

Every capture phase below is the same shape:

> **new capture crate implementing `CaptureBackend` + probe/negotiation
> entry + packaging delta. `flowshot-core`, `flowshot-capture`, and
> `flowshot-ui` ship UNCHANGED — that is the purity gate's whole point.**

## What the gates guarantee today

- **Platform purity** (`scripts/purity-gate.sh`, CI-enforced): the lib code
  of `flowshot-core`, `flowshot-capture`, and `flowshot-ui` contains zero
  Wayland/X11/D-Bus/PipeWire/unix-FFI imports, zero `winit::platform::*`
  extension imports, and zero `cfg(target_os)`-family conditionals. The
  recorded allowlist (`scripts/purity-allowlist.txt`) has exactly two
  entries: flowshot-ui's dev-dependencies on the platform crates for QA
  harnesses (examples/tests only — the lib never imports them).
- **Unsafe audit**: `#![forbid(unsafe_code)]` in every lib crate and both
  binary entry points EXCEPT `flowshot-capture-wayland`, the single
  recorded unsafe-exempt crate (allow-list of one, reserved for the future
  zero-copy dmabuf path). As of this audit it contains **zero** `unsafe`
  blocks — v1 captures through `wl_shm` with ordinary file I/O, so the
  exemption is unused. Any future `unsafe` there requires a SAFETY comment
  per block (engineering standard #4).
- **The seam**: `flowshot-capture::CaptureBackend` is the only contract a
  platform crate must satisfy; `BackendKind` declares the roadmap
  variants `Windows`, `MacOs` (documented, non-constructible in v1:
  `is_roadmap()`, `CapabilityProbe::supports` reports `false`, forcing one
  is a typed `CaptureError::NoBackendAvailable`). `X11` was promoted out of
  the roadmap set when Phase 1 shipped: on an X11 session the probe reports
  it and it negotiates like any other rung.
- **Session model**: the CLI spawns a session child that owns the UI event
  loop on its main thread. That model is also the X11/macOS answer to
  main-thread windowing requirements — no re-architecture needed per port.

## What a porter must implement

1. A new crate `flowshot-capture-<platform>` (sibling of
   `flowshot-capture-wayland`; the reference architecture there: dedicated
   capture thread, private platform connection, deadline-bounded dispatch,
   typed errors, frames stitched per `OutputLayout`).
2. `impl CaptureBackend for <Platform>Backend` — the full trait surface:
   - `fn kind() -> BackendKind` (flip the matching roadmap variant live),
   - `async fn outputs() -> Result<Vec<OutputInfo>, CaptureError>` —
     connector, name, logical rect, physical size, **per-output `scale`**,
     transform,
   - `async fn capture_outputs(CaptureOpts{paint_cursor}) -> Result<Vec<Frame>>`
     — physical-pixels-first buffers, never an averaged scale,
   - `async fn capture_region(LogicalRect) -> Result<Frame>` — stitched
     composite,
   - `fn cursor_events() -> Option<CursorStream>` — `None` is a documented
     degradation, never a failure,
   - `async fn request_permission() -> PermissionResult` — infallible;
     denial is a value (`Granted | Denied | NotRequired`).
3. A `CapabilityProbe` producer for the platform + a `negotiate()` ladder
   entry (the ladder itself is data — `NEGOTIATION_LADDER` order is
   platform-gated by which kinds the probe reports).
4. Binary-layer wiring in `flowshot-cli`/`flowshot-daemon` (both outside
   the purity gate by design — they are the composition layer).
5. Packaging delta for the platform (installer, permissions, manifest).

### Trait-conformance checklist (recorded research)

The F20 leak-list — the platform behaviors the trait had to absorb — and
where each lives in the shipped contract:

| F20 requirement | Shipped surface | Conforms |
|---|---|---|
| Must be ASYNC (SCK streams, WGC `FrameArrived`, portal D-Bus are async; X11 `GetImage` is sync) | `#[async_trait] trait CaptureBackend`; every capture/output/permission method `async`; sync platforms wrap in blocking tasks (shipped: the X11 backend runs `GetImage` on a dedicated worker thread bridged into the async trait) | yes |
| `request_permission()` (no-op Linux/X11, multi-step macOS, WGC picker) | `async fn request_permission() -> PermissionResult`, infallible by contract; `can_capture()` helper (the shipped X11 backend returns `NotRequired`; X11 has no capture permission model) | yes |
| Per-output `scale_factor()` (Win per-monitor DPI, mac `backingScaleFactor`, Wayland `wl_output::scale`) | `OutputInfo.scale: f64` + `Frame.scale: f64` ("never an averaged value across outputs") | yes |
| Optional cursor stream + per-OS cursor compositing semantics | `cursor_events() -> Option<CursorStream>` (`Pin<Box<dyn Stream<Item = CursorEvent> + Send>>`, enter/leave/moved/hotspot); `CaptureOpts.paint_cursor` for compositor-side painting | yes |
| Frame delivery: CPU buffer (Linux/X11) vs GPU texture (Win D3D11, mac IOSurface) | `FrameBuffer { data, width, height, stride, format }` with typed `FrameFormat { Xrgb8888, Argb8888, Rgba8888 }`; v1 is CPU-buffer-only (dmabuf recorded as future optimization). **Porter note**: WGC/SCK ports either stage to CPU (v1-compatible, one copy) or extend `Frame` with a GPU-texture variant — a trait evolution decision recorded here, not taken in v1 | yes (CPU path); GPU path documented |

## Phase 1 — X11 — SHIPPED 2026-10-04 (Phase A: headless capture)

Shipped as `crates/flowshot-capture-x11`, live-verified on i3 (observed
values: [verification.md](verification.md) "Landed bundles"). The shipped
scope covers the headless capture path - `capture full`,
`capture screen [OUTPUT]`, and `capture --region WxH[+X+Y]` with
`--no-edit`, plus copy/save/`--raw`/`--print-geometry`/delay and the
daemon-owned clipboard (INCR for large payloads) - and, with Phase B, the
interactive overlay/editor/pins/dialogs plus the headless `capture last`
and `--region at-cursor` reroutes. There is still no Xwayland
fallback path: sessions are mutually exclusive, and a Wayland session never
routes to the X11 backend.

Deviations from the research sketch below (behavior won; the sketch is
preserved as the 2026-09-28 record):

- **x11rb 0.14 used directly.** The xcap path stayed research-only, as
  planned; there is no xcap dependency.
- **SHM is fd-passing, not SysV**: `memfd` + `shm::attach_fd` +
  `shm::get_image`, gated on server SHM >= 1.2 (AttachFd landed in MIT-SHM
  1.2; the plan-era "SHM 1.15" figure was wrong), with plain `GetImage` as
  the fallback. Rationale: zero `unsafe`, and it mirrors the Wayland memfd
  pattern (`icc/shm.rs`). Measured on the QA machine: 38 ms SHM vs 80 ms
  plain per 18.6 MB frame.
- **No unsafe allow-list growth**: the exemption was not needed; the
  allow-list stays at its one (still empty) entry and
  `flowshot-capture-x11` is `#![forbid(unsafe_code)]`.
- **Scale derivation as specced**: `Xft.dpi` (RESOURCE_MANAGER) preferred,
  RANDR mm-size heuristic fallback, quantized to 0.25, clamped to
  [1.0, 4.0], documented approximate.
- **`cursor_events()` deferred**: returns `None` (documented degradation);
  a poll-based stream is a Phase B candidate. The cursor itself is painted
  from XFIXES at capture time.

### Research sketch (2026-09-28 record, pre-shipment)

Target: the remaining X11-only desktops. On Linux, Wayland stays the only
supported session type for the v1 product; there is no Xwayland fallback
path and this phase does not change that.

Crates (>=3, spot-checked):

- **xcap** 0.9.8 — reference implementation. Its Xorg path
  (`src/linux/xorg_capture.rs` L72-129 @5c205f2) is synchronous xcb
  `GetImage` with **no XShm and no cursor** — usable as a correctness
  reference, not as the performance target.
- **x11rb** 0.14.0 (+ **x11rb-protocol**) — the recommended base: safe xcb
  bindings with the RANDR, XFIXES, and XTEST extensions, async-friendly
  (non-blocking request/reply with explicit polling).
- **x11-dl** 2.21.0 — already in the lock (winit's X11 backend); available
  for dynamic-loading patterns.

APIs: `xcb_get_image` (per-output, honoring the RANDR transform),
**XShm optimization**: `shmget`/`shmat` + `xcb_shm_get_image` with
`XCB_IMAGE_FORMAT_Z_PIXMAP` (the note F19 records — xcap skips this; a
FlowShot X11 backend should not), RANDR `GetMonitors`/`GetCrtcInfo` for
output enumeration, `XFixesGetCursorImage` for cursor position+pixels
(composite manually — X11 gives no cursor-included capture),
`XSetErrorHandler` (X11's error channel is a process-global callback;
contain it in the platform crate).

Red flags: sync-only capture (wrap per F20); **no scale concept in X11** —
`OutputInfo.scale` must be derived (RANDR mm size + `Xft.dpi` resource),
documented as approximate; mixed-DPI X11 is rare but the physical-first
geometry rule (ADR-001) applies unchanged; no permission model
(`request_permission` -> `NotRequired`).

## Phase 2 — Windows (WGC + DXGI)

Crates (>=3, spot-checked):

- **windows-capture** 2.0.1 — WGC + DXGI capture with
  `CursorCapture`/`DrawBorder`/`DirtyRegion` settings
  (`src/settings.rs` L46-65 @c7d1064, F19).
- **windows** 0.62.2 (Microsoft) — direct WinRT/Win32:
  `Windows.Graphics.Capture` (`GraphicsCaptureItem`,
  `Direct3D11CaptureFramePool`, `FrameArrived`), `Win32.Graphics.Dxgi`
  (desktop-duplication fallback), `Win32.UI.WindowsAndMessaging`
  (`SetWindowLongW`).
- **scap** 0.0.8 — the cap.so capture crates (Tauri-based tool that
  deliberately avoided xcap; proof cross-platform Rust capture is viable,
  F19) — alternative/reference.
- xcap's `wgc` feature — second reference (its `SetIsBorderRequired` use
  is Win11+ only; GDI `BitBlt` fallback path documented there).

APIs and the F24 red flags:

- **Yellow border**: WGC draws a capture border on Windows 10 that cannot
  be disabled (`SetIsBorderRequired` is Windows 11+). Monitor capture is
  clean; window capture on Win10 carries the border — surface it as a
  documented degradation, never a silent artifact.
- **Click-through overlay**: winit does not expose it; the overlay needs
  raw `SetWindowLongW(hwnd, GWL_EXSTYLE, WS_EX_LAYERED |
  WS_EX_TRANSPARENT)` through the `windows` crate at the binary layer
  (the `WindowCustomizer` seam already exists for exactly this class of
  platform extension).
- **DPI**: a PerMonitorV2 DPI-awareness manifest is required for correct
  multi-monitor behavior; `GetDpiForMonitor` feeds `OutputInfo.scale`
  (per-output, per F20).
- **Permission**: WGC shows a system picker for programmatic capture —
  that is `request_permission()` returning `Granted/Denied`, not an error
  path.
- **Frame delivery**: D3D11 textures. v1-compatible path = staging-buffer
  copy to `FrameBuffer`; the GPU-texture trait extension (F20 note above)
  pays off here first.

## Phase 3 — macOS (ScreenCaptureKit)

Crates (>=3, spot-checked):

- **screencapturekit** 11.0.0 (repo `screencapturekit-rs`; F19 recorded
  10.0.3 @a46de0a on 2026-09-07 — bumped since) — `SCStream` bindings
  including the `CGRequestScreenCaptureAccess` permission flow. NOT xcap's
  legacy `CGWindowListCreateImage` path (F19 explicit).
- **objc2** 0.6.x + **objc2-app-kit** 0.3.2 — safe Objective-C runtime for
  `NSPanel` and window-collection behavior (the overlay window model).
- **core-graphics** 0.25.0 — legacy `CGImage`/`CGDisplay` APIs; needed for
  display enumeration glue and the permission prompt, not for capture.
- **scap** 0.0.8 — has a macOS SCK path; second reference.

APIs and the F24 red flags:

- **Capture**: `SCShareableContent` (enumeration), `SCStream` +
  `SCStreamConfiguration` (per-display `sourceRect` in physical points,
  `scalesToFit`), `SCStreamOutput` delegate → CMSampleBuffer → IOSurface.
  Async by nature (F20's SCK-stream note).
- **Permission UX**: screen-recording permission is multi-step (system
  dialog + TCC grant + possible app relaunch), and **revocation kills the
  stream mid-capture** — `request_permission()` must model the relaunch
  case and the stream must map revocation to a typed error, not a hang.
- **Distribution**: code-signing + notarization are **required for the
  permission to persist** (an unsigned/ad-hoc build re-prompts forever).
  This is a packaging-phase gate, not a code gate.
- **Overlay window**: needs `NSPanel` +
  `NSWindowCollectionBehaviorCanJoinAllSpaces` (+ `NSWindowLevel` above
  the desktop) to behave like the Wayland fullscreen-overlay model;
  winit's macOS backend covers most of it, the panel behavior goes through
  the binary-layer customizer seam.
- **Scale**: `backingScaleFactor` per display → `OutputInfo.scale`
  (Retina = 2.0); the physical-first rule makes this a non-event.
- **Main thread**: SCK/Winit main-thread requirements are already solved
  by the session-child process model.

## Phase 4 — Init systems (packaging only, ZERO code change)

User directive 2026-09-24: the daemon is a plain
foreground process, init-agnostic by design; `sd_notify(3)` READY=1 sits
behind the optional `systemd` cargo feature (shipped:
`crates/flowshot-daemon` feature `systemd`). Each of these is a service
file in `packaging/`, not a port:

- **OpenRC** (Artix/Gentoo): `/etc/init.d/flowshot` with
  `supervise-daemon` (or `start-stop-daemon` + `--background NO`); user
  session units via `rc-service` or a desktop autostart entry.
- **runit**: a `run` script — `#!/bin/sh; exec flowshot daemon` (runit's
  supervision replaces the systemd unit entirely).
- **s6**: service directory with `run` + optional `notification-fd`
  handshake (s6's `sd_notify` compatibility via `s6-notifyoncheck`).
- **GNU Shepherd**: `flowshot.scm` with `make-forkexec-command` /
  `make-systemd-constructor`-style foreground supervision.
- Crate reference for the notify handshake if ever extracted: **sd-notify**
  0.5.0 (the current in-tree implementation talks the `NOTIFY_SOCKET`
  protocol directly behind the feature flag).

These service files land with the packaging milestone; the README's install
section gains the init-system matrix when they ship.

## Gate re-run checklist per phase

Every port lands only with: `scripts/purity-gate.sh` green (the new
platform crate is NOT added to the gated list; the gated crates must show
zero diff), `cargo deny check` green with any new license recorded in
deny.toml, the F20 checklist above re-verified against the new impl,
negotiation-ladder tests extended (the `negotiate` matrix is data-driven),
and the packaging validators for the target platform.

## crates.io spot-check record (2026-09-28, crates.io API v1)

| Crate | max_stable | updated | Downloads |
|---|---|---|---|
| xcap | 0.9.8 | 2026-08-01 | 2,153,076 |
| windows-capture | 2.0.1 | 2026-08-08 | 1,672,751 |
| screencapturekit (repo screencapturekit-rs) | 11.0.0 | 2026-09-24 | — |
| x11rb | 0.14.0 | 2026-07-16 | 66,110,239 |
| windows | 0.62.2 | 2025-10-06 | 339,589,315 |
| objc2-app-kit | 0.3.2 | 2025-10-04 | 61,923,342 |
| scap | 0.0.8 | 2025-08-04 | 39,438 |
| core-graphics | 0.25.0 | 2025-05-27 | 76,620,026 |
| sd-notify | 0.5.0 | 2026-03-09 | 14,995,812 |
| x11-dl | 2.21.0 | 2023-01-18 | 64,946,562 |

Note: the crate `screencapturekit-rs` does not exist on crates.io (API
returns "does not exist"); the published name is `screencapturekit`.

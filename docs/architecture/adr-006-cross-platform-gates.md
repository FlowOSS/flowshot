# ADR-006: Cross-platform gates and the porting roadmap

Status: accepted (gates SHIPPED: `scripts/purity-gate.sh` +
`scripts/purity-allowlist.txt`, CI-enforced in `.github/workflows/ci.yml`;
the detailed per-phase entry points live in
[docs/porting-roadmap.md](../porting-roadmap.md)). X11 (Phase 1) is
**SHIPPED** via `flowshot-capture-x11`: the headless capture path
(2026-10-04) and the interactive UI (Phase B, 2026-10-06); the
verification records with inlined observed values live in
[docs/verification.md](../verification.md). Platforms
beyond Wayland and the X11 capture rung are **roadmap only: no code
exists** for them and none is planned for the first release.

## Context

Most "cross-platform" screenshot tools rot into a pile of `#ifdef` where
every fix has to be made three times. FlowShot's architecture was chosen to
make ports possible without that outcome: geometry, config, scene, editor,
and chrome are platform-free; the capture stack is a trait with five Wayland
implementations behind a probe (a sixth implementation, X11, has since
shipped). The risk is drift: one convenience import of
a Wayland type into the core crate and the gate story collapses.

## Decision

A **purity contract**, enforced by CI audit (grep/AST over the crate graph):

- `flowshot-core`, `flowshot-capture` (the trait crate), and `flowshot-ui`
  contain zero Wayland/X11 imports and no `cfg(target_os)` outside a recorded
  allowlist. (`flowshot-actions` is deliberately outside the audit: the
  clipboard is platform-native by design, per the daemon-ownership model in
  [ADR-004](adr-004-daemon-lifecycle.md); it owns the Wayland selection
  backend and, since X11 Phase A, the X11 one.)
- `flowshot-capture-wayland` is the Wayland platform crate;
  `flowshot-capture-x11` (shipped 2026-10-04) is the X11 one. Everything
  that talks to a compositor or an X server lives in one of those two
  crates.
- The `CaptureBackend` trait is the porting seam: async, per-output scale
  reporting, `request_permission`, optional cursor stream, typed frame
  formats. A port is a new crate implementing the trait plus packaging;
  the platform-free crates stay untouched. That is the gate's whole
  point, and the X11 port mostly proved it: `flowshot-core` shipped with
  test-only edits and `flowshot-capture` with the enum/ladder promotion
  only - but `flowshot-ui` did require behavior changes (the
  display-server gate learned `DISPLAY` through an injectable seam, and
  winit's X11 backend needed pin fixes: client-driven
  `request_inner_size`, synthetic-key filtering on focus resync, and
  session-routed resize anchors). The lesson for the next port: budget
  for windowing-backend quirks in the UI crate, not just a new platform
  crate.
- Consequences of the gate are real and paid: the UI crate cannot even set
  the Wayland `app_id` (a platform extension); the binary layer injects it
  through a customizer seam. Platform conveniences do not get smuggled in.

## The unsafe allow-list (audit record)

`#![forbid(unsafe_code)]` is present in every lib crate and both binary
entry points (`flowshot-cli/src/main.rs`, `flowshot-daemon/src/main.rs`)
EXCEPT `flowshot-capture-wayland`, which is the single recorded
unsafe-exempt crate (engineering standard #4). The exemption is reserved
for the future zero-copy dmabuf FFI path. As of this audit the crate
contains **zero** `unsafe` blocks: v1 captures into `wl_shm` buffers read
back with ordinary file I/O (`icc/shm.rs`), so no memory mapping and no
`unsafe` are needed. The allow-list is therefore a single empty entry —
`flowshot-capture-wayland` — and any future `unsafe` block added there must
carry a per-block SAFETY comment.

The X11 port (Phase A, shipped 2026-10-04) kept the list at that one entry.
Its MIT-SHM fast path uses fd-passing (`memfd` + `shm::attach_fd`, requiring
server SHM >= 1.2) with pixels read back through ordinary file I/O, and
plain `GetImage` as the fallback. There is no SysV `shmat` anywhere in the
crate, so `flowshot-capture-x11` is `#![forbid(unsafe_code)]` like every
other crate and no exemption was needed.

## Porting roadmap (Phase 1 shipped; the rest is future, no code)

Each phase is a new capture-crate implementation plus its packaging delta.
The full entry-point map — concrete crates with spot-checked versions, the
platform APIs, the F24 red flags, the `CaptureBackend` conformance checklist
against draft F20, and the init-systems packaging phase — lives in
[docs/porting-roadmap.md](../porting-roadmap.md). Summary of the named
entry points, from the research ledger:

1. **X11**: SHIPPED 2026-10-04 (Phase A, headless capture) as
   `flowshot-capture-x11`: xcb `GetImage` with an MIT-SHM fd-passing fast
   path (memfd + `attach_fd`, server SHM >= 1.2; no SysV `shmat`). X11
   support targets the remaining X11-only desktops; the interactive UI on
   X11 (overlay/editor/pins/dialogs) is a later phase, and there is no
   Xwayland fallback path.
2. **Windows**: `windows-capture` 2.0.1 (Windows Graphics Capture + DXGI).
   Known caveats recorded: the Windows 10 yellow capture border,
   `SetWindowLong` click-through for the overlay, and a PerMonitorV2
   manifest requirement for correct DPI behavior.
3. **macOS**: `screencapturekit-rs` 10.0.3 (ScreenCaptureKit). Recorded
   caveats: the permission UX (system dialog + TCC), code-signing and
   notarization for distribution, and an `NSPanel` + `CanJoinAllSpaces`
   overlay window model.
4. **Init systems (packaging, not code)**: OpenRC init script (Artix/Gentoo),
   runit, s6, and GNU Shepherd service templates. Zero code change: the
   daemon is a foreground process and `sd_notify` is behind an optional
   cargo feature, so each of these is a service file, not a port.

## Consequences

- The portability claim is structural and verifiable (the audit fails CI if
  a platform import crosses the boundary), not a promise in a README.
- The roadmap names concrete crates and APIs so a future port starts from
  research, not from scratch. None of it is committed for v0.1.0.
- The original v1 non-goal (no X11 session support on Linux, no X11 code
  path) was amended by owner directive on 2026-10-04. X11 ships natively
  via `flowshot-capture-x11`: the headless capture path (`capture full`,
  `capture screen [OUTPUT]`, and `capture --region WxH+X+Y` with
  `--no-edit`, plus copy/save/`--raw`/`--print-geometry`/delay and the
  daemon-owned clipboard) and, with Phase B, the interactive overlay,
  editor, pins, and dialog windows; `capture last` and
  `--region at-cursor` take a headless reroute on X11. The support is
  native X11, not Xwayland: there is still no Xwayland fallback path, and
  a Wayland session never routes to the X11 backend (the session types are
  mutually exclusive).

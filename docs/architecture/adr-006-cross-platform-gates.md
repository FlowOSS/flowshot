# ADR-006: Cross-platform gates and the porting roadmap

Status: accepted (gates designed; the purity CI job lands with the packaging
milestone). Platforms beyond Wayland are **roadmap only: no code exists**
for them and none is planned for the first release.

## Context

Most "cross-platform" screenshot tools rot into a pile of `#ifdef` where
every fix has to be made three times. FlowShot's architecture was chosen to
make ports possible without that outcome: geometry, config, scene, editor,
and chrome are platform-free; the capture stack is a trait with five Wayland
implementations behind a probe. The risk is drift: one convenience import of
a Wayland type into the core crate and the gate story collapses.

## Decision

A **purity contract**, enforced by CI audit (grep/AST over the crate graph):

- `flowshot-core`, `flowshot-capture` (the trait crate), and `flowshot-ui`
  contain zero Wayland/X11 imports and no `cfg(target_os)` outside a recorded
  allowlist. (`flowshot-actions` is deliberately outside the audit: the
  clipboard is Wayland-native by design, per the daemon-ownership model in
  [ADR-004](adr-004-daemon-lifecycle.md).)
- `flowshot-capture-wayland` is **the** platform crate. Everything that talks
  to a compositor lives there.
- The `CaptureBackend` trait is the porting seam: async, per-output scale
  reporting, `request_permission`, optional cursor stream, typed frame
  formats. A port is a new crate implementing the trait plus packaging;
  core and UI ship unchanged. That is the gate's whole point.
- Consequences of the gate are real and paid: the UI crate cannot even set
  the Wayland `app_id` (a platform extension); the binary layer injects it
  through a customizer seam. Platform conveniences do not get smuggled in.

## Porting roadmap (future, no code)

Each phase is a new capture-crate implementation plus its packaging delta.
Named entry points, from the research ledger:

1. **X11**: xcap's xcb `GetImage` path, with the XShm optimization noted.
   (On Linux, Wayland remains the only supported session type; X11 support
   targets the remaining X11-only desktops.)
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
- A deliberate non-goal restated: no X11 session support on Linux in v1,
  ever reachable or not. There is no X11 code path.

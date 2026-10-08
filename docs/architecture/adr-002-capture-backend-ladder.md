# ADR-002: Capture backend ladder and negotiation

Status: accepted, implemented (flowshot-capture `negotiate`,
flowshot-capture-wayland and flowshot-capture-x11 backends).

## Context

Wayland has no single capture API. What a compositor offers depends on the
compositor and its version: ext-image-copy-capture-v1 (Hyprland, wlroots
0.19+, COSMIC), wlr-screencopy (the whole wlroots family), KWin's ScreenShot2
D-Bus interface (KDE), and the XDG portals (everywhere a portal backend
exists, at a UX cost). Hardcoding one path per desktop rots: compositors grow
protocol support, and a version-keyed table would ship stale the day it
releases.

## Decision

Backends are tried in a fixed ladder, strongest first, selected by probing
the **live session** (advertised Wayland globals, D-Bus name ownership,
portal version, X11 server reachability), never by a desktop-name table:

1. `ext-image-copy-capture-v1` (ICC). Direct protocol, per-output sessions,
   optional compositor-painted cursor, cursor-position sessions. Verified
   live on Hyprland 0.56.2.
2. `zwlr-screencopy-v1` (v3). The wlroots-family fallback. Verified live on
   Hyprland.
3. `org.kde.KWin.ScreenShot2`. KWin's D-Bus fast path; raw pixels + metadata
   over a pipe. Stub-verified against KDE's fetched source; live KDE QA is
   hardware-gated.
4. `org.freedesktop.portal.ScreenCast` + PipeWire. Portal-mediated streams;
   one session per stream batch with a top-up loop covering both
   one-stream-per-session and N-stream portals. Verified live against
   Hyprland's XDPH portal stack.
5. `org.freedesktop.portal.Screenshot`. Last Wayland rung; file-URI result,
   compositor-defined interaction on some paths.
6. X11 `GetImage` (xcb, MIT-SHM fd-passing fast path). Reached only on X11
   sessions (no `WAYLAND_DISPLAY`); the session types are mutually
   exclusive, so this rung never competes with the Wayland ones. Shipped
   2026-10-04 for the headless capture path.

Negotiation rules (`negotiate()`):

- The probe decides. A compositor that grows ICC support is picked up with no
  FlowShot update and no release note.
- A `force_backend` override wins over ladder **order** but not over
  **capabilities**: forcing a kind the probe does not support fails fast at
  negotiation with a typed error naming it, instead of dying at protocol bind
  time.
- Roadmap kinds (Windows, macOS) exist as documented enum variants but
  are non-constructible through negotiation and through `force_backend`.
  X11 left the roadmap set when Phase A shipped: on an X11 session the
  probe reports it and the last rung serves.
- Every failure is typed: `NoBackendAvailable` names the missing protocols;
  permission denial is a distinct `PermissionResult`, never a hang and never
  a silently black capture (Hyprland's denial frame, a black field with a
  centered notice, is classified and mapped to permission-denied).

## Consequences

- Per-desktop reality emerges from the ladder instead of being asserted:
  Hyprland gets ICC, sway/COSMIC get ICC-or-screencopy, niri gets
  screencopy, KDE gets ScreenShot2, GNOME gets the portals (with the picker
  UX that implies). The per-desktop guides ([Hyprland](../setup-hyprland.md),
  [sway](../setup-sway.md), [KDE](../setup-kde.md), [GNOME](../setup-gnome.md))
  describe the observable results.
- Cursor position is a separate, parallel ladder (ICC cursor session, then
  Hyprland IPC, then overlay first motion) and degrades independently: a
  cursor-permission denial never fails a capture.
- Each backend ships with its own typed error family and shares only
  backend-neutral infrastructure (deadline dispatch, SHM buffers, worker
  threads). Backend-tagged selection logic is deliberately duplicated rather
  than abstracted prematurely.
- Non-goals held: no runtime
  shell-outs to grim/wl-copy/slurp anywhere in the stack. (The original
  no-X11 non-goal was amended 2026-10-04: X11 is now the ladder's last
  rung, and Phase B shipped the interactive UI on X11 in the same
  branch.)

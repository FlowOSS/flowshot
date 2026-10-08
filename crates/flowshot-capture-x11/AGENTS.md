# crates/flowshot-capture-x11

The X11 platform crate (the negotiation ladder's last rung): native X11
capture for X11-only sessions (i3, Xfce, Openbox) — sibling of
`flowshot-capture-wayland`, and like it deliberately NOT purity-gated.
`#![forbid(unsafe_code)]`: MIT-SHM uses fd-passing (`memfd` + `attach_fd`)
with ordinary file-I/O readback, never SysV `shmat`.

## STRUCTURE

```
flowshot-capture-x11/
├── src/
│   ├── lib.rs       # crate map + scale-derivation rules + safety policy
│   ├── error.rs     # typed X11Error -> CaptureError{backend:X11}
│   ├── connect.rs   # X11Connection (x11rb RustConnection + screen/root)
│   ├── probe.rs     # probe_x11: the negotiation probe; None on Wayland
│   │                #   sessions (WAYLAND_DISPLAY wins) and below the
│   │                #   RANDR 1.3 floor
│   ├── scale.rs     # Xft.dpi-first scale derivation (0.25 steps, [1,4])
│   ├── output.rs    # RANDR GetMonitors (lit-CRTC fallback) -> OutputInfo
│   ├── capture.rs   # per-output GetImage: MIT-SHM memfd fast path,
│   │                #   plain-socket fallback, XFixes cursor paint
│   ├── cursor.rs    # one-shot XQueryPointer position + GetCursorImage
│   ├── worker.rs    # blocking-op bridge: dedicated thread + 10s watchdog
│   │                #   deadline race -> typed CaptureError::Timeout
│   └── backend.rs   # X11Backend: CaptureBackend impl; connect() sync,
│                    #   connect_bounded() for async callers
└── examples/        # capture (QA pixel-oracle PNGs under $FLOWSHOT_QA_OUT),
                     # probe
```

## RULES THAT KEEP THIS CRATE HONEST

- **Every blocking X11 operation runs on the worker bridge** (`worker.rs`):
  never call x11rb blocking APIs on an async caller's thread — use
  `connect_bounded()` / `spawn_worker` so a half-alive server surfaces a
  typed timeout (10 s, mirroring the Wayland sibling's `REPLY_TIMEOUT`)
  instead of hanging the daemon forever.
- **Physical pixels first** (ADR-001): `GetImage` returns framebuffer
  pixels 1:1; logical rects divide by the PER-OUTPUT derived scale — never
  an averaged factor. The derived scale is approximate by nature (`Xft.dpi`
  wins; else the RANDR physical-size heuristic) and is documented as such.
- **Region captures stitch through `flowshot_capture::stitch`** (the shared
  contract-crate algebra, re-exported as `crate::stitch`) — no local copy;
  the X11 leg passes `BackendKind::X11` like the Wayland backends pass theirs.
- **No cursor stream**: `cursor_events()` is `None` by recorded product
  decision (one-shot reads only: `cursor_pos`, XFIXES `GetCursorImage` per
  capture). Region composites never paint the cursor (scrot-equivalent).
- **`probe_x11` gates on the session family**: `WAYLAND_DISPLAY` set and
  non-empty → `None` — an XWayland session belongs to the Wayland rungs;
  capturing the XWayland root would silently miss every Wayland-native
  window (a wrong image, not a typed failure).
- Errors are the typed `X11Error` family with environment-derived hints
  (the `DISPLAY` remediation on connect failures); they convert into
  `CaptureError::Backend{backend:X11}` / `Timeout{backend:X11}`. The daemon
  logs them through `execute::backend::error_detail` so the hint survives.

## QA NOTES

- Live-verified on i3/X11 (single eDP-1 panel, picom compositing); observed
  values are inlined in `docs/verification.md` "Landed bundles" (Phase A
  headless + Phase B interactive records).
- `examples/capture.rs` is the pixel-oracle harness — outputs go to
  `$FLOWSHOT_QA_OUT` (default: a per-process temp dir), never fixed shared
  `/tmp` paths.
- Multi-monitor spanning on X11 is hardware-gated (the QA machine is
  single-panel) and NOT claimed anywhere.

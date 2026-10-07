# FlowShot on X11 (i3 and other X11 desktops)

Verification status: **live-verified on i3** (Xorg, single eDP-1 2880x1620,
derived scale 2.25; SHM 1.2, RANDR 1.5, XFIXES 5.0) in two bundles: the
headless capture path (Phase A, 2026-10-04) and the interactive UI
(Phase B, 2026-10-05/06); the observed values are recorded inline in
[verification.md](verification.md) under "Landed bundles". X11 support
covers the full product: `capture full`, `capture screen [OUTPUT]`, and
`capture --region WxH[+X+Y]` (with or without `--no-edit`),
copy/save/`--raw`/`--print-geometry`/delay, the daemon-owned clipboard,
and the interactive region overlay, annotation editor, pins,
settings/launcher/consent dialogs, and color picker.

## Requirements

- An X11 session (`DISPLAY` set). XWayland is not a target: on a Wayland
  session the Wayland backends serve, and there is no X11 fallback path in
  either direction.
- RANDR >= 1.2 for output enumeration (names, geometry, transforms). Any
  Xorg from the last decade qualifies.
- Optional: XFIXES for painting the cursor into the capture. Without it the
  capture simply omits the cursor, same as `hide_cursor = true`.
- Optional: MIT-SHM >= 1.2 for the fast path. Measured on the QA machine:
  38 ms vs 80 ms per full 2880x1620 frame. Without it the backend uses
  plain `GetImage` over the socket; captures stay correct either way.

## Capture backend

X11 has no compositor protocols to negotiate: the backend is a single rung,
xcb `GetImage` (Z_PIXMAP) against the root window, per output, honoring the
RANDR transform. The fast path passes the pixel buffer through a memfd file
descriptor via MIT-SHM `AttachFd` and reads it back with ordinary file I/O,
so the crate stays `#![forbid(unsafe_code)]` (no SysV `shmat` anywhere). The
cursor is composited from XFIXES at capture time; there is no live cursor
stream on X11 in this release, so `capture screen` with no argument and
offset-less `--region WxH` centering use a one-shot `XQueryPointer` read.

## Interactive UI

The region overlay, annotation editor, pins, settings, launcher, and
consent dialogs run natively on X11 - live-verified on i3 with picom
(observed values: the Phase B entry in
[verification.md](verification.md)):

- Transparency: the overlay keeps real alpha (`with_transparent(true)`)
  and picom composites it correctly (the QA decision measured a dimmed
  frozen desktop, decisively not black). Transparency needs a compositing
  manager; an uncomposited bare X session was not QA'd.
- i3 focus: i3 auto-floats and auto-focuses FlowShot windows via their
  WM_CLASS (verified: `FlowShot Pin`/`flowshot-pin`,
  `FlowShot Settings`/`flowshot-settings`,
  `Capture Launcher`/`flowshot-launcher`, and the consent dialog), so the
  overlay is draggable immediately and pins take keyboard focus for
  their hotkeys (opacity digits, Esc precedence menu-then-pin).
- `capture last` and `--region at-cursor` take a headless reroute on X11
  (no overlay window): the persisted last region, or the output under a
  one-shot `XQueryPointer` cursor read, goes straight to the direct leg.
- Multi-monitor overlay spanning on X11 is hardware-gated (the QA machine
  is single-panel) and not claimed.

## Global shortcuts

There is no shortcut portal on a bare X11 session, so window-manager binds
are the mechanism (the same fallback path Wayland compositors use). For i3
(`~/.config/i3/config`, then `i3-msg reload`):

```ini
bindsym Print exec flowshot capture screen --no-edit -c
bindsym Shift+Print exec flowshot capture full --no-edit -c
bindsym Ctrl+Print exec flowshot capture --region 1280x720 --no-edit -c
```

`flowshot --print-bind-help` prints the current snippet set from the
installed binary.

The verbs that work on X11 are the headless ones - `full`,
`screen [OUTPUT]` (by connector name or index), and
`--region WxH[+X+Y]` (offset-less `WxH` centers on the cursor), all with
`--no-edit`, plus `-c`/`-o`/`--raw`/`--print-geometry`/`-d` - and the
interactive ones: bare `flowshot capture` opens the native overlay, while
`capture last` and `--region at-cursor` take the headless reroute (see
"Interactive UI" above).

## Clipboard

The daemon owns the `CLIPBOARD` selection, so a copied capture stays
pasteable after the capture process exits (the same daemon-ownership model
as on Wayland, per
[architecture/adr-004-daemon-lifecycle.md](architecture/adr-004-daemon-lifecycle.md)).
Payloads above the X request-size limit transfer over INCR; the QA
machine's 1.08 MB full-screen PNG is served in 256 KiB chunks. Killing the
daemon drops the offer, by design; the next capture respawns it.

## Scale and HiDPI

X11 has no scale concept; the screen is a pixel-exact framebuffer. FlowShot
derives the per-output scale its logical geometry needs, and the result is
documented as approximate:

1. `Xft.dpi` from the `RESOURCE_MANAGER` root property wins
   (`scale = dpi / 96`). Set it with:

   ```sh
   echo 'Xft.dpi: 192' | xrdb -merge
   ```

   (192 means scale 2.0.) Note this is a session-global value; there is no
   per-output `Xft.dpi`.
2. Without it, a RANDR physical-size heuristic applies per output:
   `dpi = width_px / (width_mm / 25.4)`, `scale = dpi / 96`.

The result is quantized to the nearest 0.25 and clamped to [1.0, 4.0]. On
the QA machine (no `Xft.dpi` set) the heuristic derives 2.25 for eDP-1
(2880 px over 344 mm). Geometry stays physical-first: captures are native
pixels (a `--region 800x600+100+100` at scale 2.25 exports 1800x1350
pixels), and `--print-geometry` reports the logical form (physical divided
by scale).

## Known limits

- Global shortcuts are window-manager binds; there is no portal
  integration on bare i3.
- The cursor is composited from XFIXES at capture time; there is no live
  cursor stream. On drivers using a software cursor the framebuffer already
  contains the cursor glyph and the composite would double-paint it; QA
  verified the common hardware-cursor case (modesetting).
- Derived scale is only as good as the monitor's reported physical size.
  Some panels report rounded or wrong millimeter sizes, so if the scale
  matters, set `Xft.dpi` explicitly (above).
- X11 telemetry reports no monitor layout: the layout probe drives the
  Wayland capture stack only (a disclosed degradation, not a capture
  difference).

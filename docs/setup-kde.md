# FlowShot on KDE Plasma

Verification status: **stub-verified only**. The KWin backend's wire contract
is pinned byte-for-byte against KDE's own source (kwin
`screenshotdbusinterface2.cpp` and Qt's `qimage.h`, at recorded commits) and
exercised against a private-bus stub that replays that contract, but no KDE
Plasma session was available on the QA machine. Every live claim below is
marked; treat the rest as well-informed expectation, not observation.

## Capture backend

Plasma does not implement `ext-image-copy-capture-v1` or `wlr-screencopy`,
so the ladder (see
[architecture/adr-002-capture-backend-ladder.md](architecture/adr-002-capture-backend-ladder.md))
lands on the third rung: KWin's `org.kde.KWin.ScreenShot2` D-Bus interface.
Per-output captures request the output's logical rect with
`native-resolution: true`, and KWin delivers upright physical pixels plus
metadata (format, width, height, stride, scale) over a pipe. FlowShot
validates the metadata before allocating (typed error on garbage) and reads
exactly `stride * height` bytes under a deadline.

### The restricted-interface requirement

KDE gates `org.kde.KWin.ScreenShot2` behind
`X-KDE-DBUS-Restricted-Interfaces`: only processes launched from a desktop
file that declares the interface may call it. FlowShot's packaged desktop
file carries the declaration:

```ini
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
```

This means on Plasma you should launch FlowShot through the installed desktop
entry (app launcher, autostart, or the daemon's own autostart file), not from
a bare terminal. A `flowshot` binary invoked without the desktop file's
launch context can be refused the ScreenShot2 interface; in that case the
backend reports a typed error and the ladder falls through to the portal
rungs below. (The exact rejection shape on a live Plasma session is queued in
the hardware-gated QA batch; see [verification.md](verification.md).)

If ScreenShot2 is unavailable for any reason, the XDG portal rungs
(ScreenCast with PipeWire, then Screenshot) serve as fallback through
xdg-desktop-portal-kde.

### What is fixed by design

Flameshot's KDE stacked/overlapping-monitor bug class (screenshot content
painted at the wrong offset when outputs overlap in coordinates) does not
apply here: FlowShot captures each output separately in physical pixels and
composites them through its own layout algebra, so no single-image
coordinate confusion can occur.

## Cursor-aware preselect

Honest degradation: **first-motion only**. KWin 6.7 implements neither the
ICC cursor-session protocol nor any public cursor-position API, so
`flowshot capture screen` (no argument) and `--region WxH` centering cannot
know where your cursor is until the overlay maps and the first pointer motion
arrives. The preselect follows that first motion. A KWin-script bridge
(`workspace.cursorPos` via a script plus a D-Bus bridge) could provide a
pre-map position and is on the roadmap; it is deliberately not in v1 because
it needs a script installation with its own failure modes.

## Global shortcuts

KDE normally serves the GlobalShortcuts portal through
xdg-desktop-portal-kde, so the daemon-managed portal path is the expected
one (not live-verified here). The portal grabs the keys itself on KDE; no
compositor-side bind syntax is needed.

Fallback (portal missing or denied): register one custom shortcut per action
in System Settings -> Keyboard -> Shortcuts. `flowshot --print-bind-help`
prints the same guidance:

| Trigger | Command |
|---|---|
| Print | `flowshot capture` |
| Shift+Print | `flowshot capture --full` |
| Ctrl+Print | `flowshot capture screen` |

These invoke the CLI directly and need no resident daemon.

## Tray

Plasma's system tray hosts SNI items natively. Enable the FlowShot tray with
`[daemon] tray = true` (see [config-reference.md](config-reference.md)).

## Pins

Pins are plain windows with app id `flowshot-pin`. KWin floats dialogs and
utility windows per its own rules; if pins tile or misbehave, add a window
rule for the `flowshot-pin` class in System Settings -> Window Management ->
Window Rules (float, keep above). Not live-verified.

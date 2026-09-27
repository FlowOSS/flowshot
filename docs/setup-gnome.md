# FlowShot on GNOME

GNOME is the degraded-support desktop, and this page is honest about where
and why. Verification status: **stub-verified plus protocol-level review**;
no GNOME session was available on the QA machine, so nothing here is claimed
live-verified on GNOME itself. The portal backends' wire behavior is
stub-tested and their shared machinery is live-verified on Hyprland's portal
stack.

## Capture backend

mutter implements neither `ext-image-copy-capture-v1` nor `wlr-screencopy`,
and KWin's interface is KDE-only, so the ladder (see
[architecture/adr-002-capture-backend-ladder.md](architecture/adr-002-capture-backend-ladder.md))
lands on the portal rungs:

1. **ScreenCast portal + PipeWire** (`org.freedesktop.portal.ScreenCast`):
   the richer path. GNOME's portal implementation can hand back multiple
   streams in one session; where a portal serves one stream per session
   (Hyprland's XDPH behaves that way), FlowShot's top-up loop opens
   additional sessions until every output is covered.
2. **Screenshot portal** (`org.freedesktop.portal.Screenshot`): the fallback
   for full-desktop capture.

### The picker UX, honestly

On the ScreenCast path, the portal's source-selection dialog is part of the
protocol: GNOME shows its own system dialog for choosing what to share, and
that dialog appears on paths where a direct-protocol compositor would simply
capture. Dismissing it maps to a clean user-cancelled outcome (exit code 3 in the CLI
contract), never an error. Non-interactive paths avoid the dialog where the portal allows it,
but portal policy is GNOME's, not FlowShot's: expect picker dialogs on some
capture paths, especially first runs before the portal has seen the app.

## Cursor-aware preselect

Honest degradation: **no cursor-aware preselect**. GNOME exposes no public
cursor-position API, mutter implements no ICC cursor session, and there is
no sanctioned shortcut. `flowshot capture screen` with no argument cannot
resolve "the output under the cursor" before the overlay maps, and
`--region WxH` cannot pre-center on the cursor. After the overlay maps, the
first pointer motion provides the position and the preselect follows it.

This is the one recorded DOCUMENTED-DEGRADED row in the Wayland fix list:
it cannot be fixed from the client side without a GNOME shell extension,
and shipping one is out of scope.

## Clipboard

GNOME does not implement `zwlr_data_control`, the protocol FlowShot's
daemon-owned clipboard uses elsewhere. FlowShot detects this and selects the
GNOME keep-alive route: a lazy offer that stays alive until the compositor
first fetches the data (notify-on-first-access, with a 500 ms safety close
when nothing ever fetches), modeled on the workaround Flameshot ships for
the same environment. The keep-alive state machine is unit-tested; its live
behavior on GNOME is not verified here. Practical consequence: after copying
a screenshot, paste it reasonably promptly rather than relying on
indefinite clipboard residency.

## Global shortcuts

GNOME serves the GlobalShortcuts portal through xdg-desktop-portal-gnome,
and the portal grabs the keys itself (no compositor-side binds needed). Not
live-verified here.

Fallback (portal missing or denied): custom keybindings via gsettings, which
`flowshot --print-bind-help` prints ready to paste. Note the warning in that
output: the first `gsettings set ... custom-keybindings` command **replaces
the list**; merge it with your existing custom bindings (the help text shows
the `gsettings get` command to read them first). The equivalents are also
reachable in System Settings -> Keyboard -> Keyboard Shortcuts -> Custom
Shortcuts:

| Trigger | Command |
|---|---|
| Print | `flowshot capture` |
| Shift+Print | `flowshot capture --full` |
| Ctrl+Print | `flowshot capture screen` |

## Tray

GNOME has no SNI host by default. Install the AppIndicator extension
(ubuntu-appindicators or the same codebase under another package name) and
the FlowShot tray works; enable it with `[daemon] tray = true` (see
[config-reference.md](config-reference.md)). Without an SNI host, the daemon
still runs fine; the tray simply never registers (and correctly does not
count as a reason to keep an auto-spawned daemon alive).

## Pins

Pins are plain windows with app id `flowshot-pin`. GNOME floats dialogs by
default and generally places new windows centered; there is no client-side
positioning API, so pins appear where mutter puts them. Not live-verified.

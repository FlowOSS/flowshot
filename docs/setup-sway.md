# FlowShot on sway and wlroots compositors

Verification status: **source-verified, not run live**. The QA machine runs
Hyprland, so nothing on this page has been exercised on a real sway session;
every claim traces to the compositor's protocol support or upstream source.
That said, the wlroots family shares most of its capture stack with what
FlowShot's Hyprland QA exercises daily, so the risk is concentrated in the
corners called out below. COSMIC follows the same ladder (ICC support
source-verified in cosmic-comp; likewise not run live here).

## Capture backend

sway 1.10+ (wlroots 0.19+) advertises `ext-image-copy-capture-v1`, the top
rung of the capture ladder (see
[architecture/adr-002-capture-backend-ladder.md](architecture/adr-002-capture-backend-ladder.md)).
Older or minimal wlroots compositors fall back to `wlr-screencopy-v3`, the
second rung; that backend is live-verified on Hyprland and shares its code
path with every wlroots compositor.

Region captures that span monitors, mixed-DPI geometry, and HiDPI all behave
the same as on Hyprland: those properties come from FlowShot's own geometry
layer, not from the compositor.

## Cursor-aware preselect

sway serves the ICC cursor-session protocol (the layer-1 cursor source), so
`flowshot capture screen` with no argument and `--region WxH` centering
resolve the cursor position before the overlay maps. This is source-verified
against wlroots (cursor sessions push their initial state at creation) and
listed as supported by wayland.app's protocol matrix for sway 1.11; it has
not been run live here.

niri note: niri does not advertise ICC (an upstream PR is open), so on niri
the capture backend is wlr-screencopy and the cursor preselect degrades to
the first-motion behavior (the selection appears where your pointer first
moves after the overlay opens).

## Global shortcuts

sway has no shortcut-grabbing portal backend in common deployments
(xdg-desktop-portal-wlr does not implement GlobalShortcuts), so expect the
compositor-bind fallback. `flowshot --print-bind-help` prints:

```ini
# ~/.config/sway/config, then `swaymsg reload`
bindsym Print exec flowshot capture
bindsym Shift+Print exec flowshot capture --full
bindsym Ctrl+Print exec flowshot capture screen
```

These run the CLI directly and need no resident daemon. If your portal stack
does implement GlobalShortcuts (some wlroots setups run xdg-desktop-portal
frontends with a capable backend), the daemon path works as documented in
[setup-hyprland.md](setup-hyprland.md#option-b-portal--global-dispatcher-binds-daemon-driven)
minus the Hyprland-specific dispatcher syntax; on such setups the portal
backend itself grabs the keys.

## Pins

sway tiles new windows by default. Pins carry app id `flowshot-pin`; float
and pin them with:

```ini
for_window [app_id="flowshot-pin"] floating enable
for_window [app_id="flowshot-pin"] sticky enable
```

Not live-verified. As on every Wayland compositor, initial pin placement is
compositor policy: there is no protocol for a client to request a window
position, so pins appear where sway places new floating windows, not at the
captured region.

## Tray

swaybar and waybar both host SNI items. Enable the FlowShot tray with
`[daemon] tray = true` (see [config-reference.md](config-reference.md)).

## Permissions

sway has no equivalent of Hyprland's `enforce_permissions` system; capture
protocols are ungated. Nothing to configure.

# FlowShot on Hyprland

Hyprland is the development and QA platform for FlowShot; everything on this
page is live-verified on Hyprland 0.56.2 unless marked otherwise. This is the
best-supported desktop: full capture, cursor-aware preselect, portal
shortcuts, tray, and pins all work.

## Capture backend

Hyprland serves `ext-image-copy-capture-v1`, which is the top rung of the
capture ladder (see
[architecture/adr-002-capture-backend-ladder.md](architecture/adr-002-capture-backend-ladder.md)).
The wlr-screencopy backend is the fallback rung and is also fully supported.
Cursor position resolves through the ICC cursor-session protocol first, with
Hyprland's own IPC socket (`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`,
the same socket `hyprctl` talks to) as the second layer, so
`flowshot capture screen` with no argument and `--region WxH` centering work
before the overlay even maps.

## Global shortcuts

There are two ways to bind hotkeys. They are not exclusive, but pick one per
key to avoid double-firing.

### Option A: plain `exec` binds (no daemon required)

The simplest setup: Hyprland runs the FlowShot CLI directly. Each press
spawns a fresh capture; the CLI auto-spawns the background daemon when needed.
This works with zero daemon residency and zero portal involvement:

```ini
# hyprland.conf
bind = ,Print,exec,flowshot capture
bind = SHIFT,Print,exec,flowshot capture --full
bind = CTRL,Print,exec,flowshot capture screen
```

Or, in the 0.56 Lua config (`~/.config/hypr/hyprland.lua`):

```lua
hl.bind("Print", hl.dsp.exec_cmd("flowshot capture"))
hl.bind("SHIFT + Print", hl.dsp.exec_cmd("flowshot capture --full"))
hl.bind("CTRL + Print", hl.dsp.exec_cmd("flowshot capture screen"))
```

`flowshot --print-bind-help` prints these snippets (and the other desktops')
from the installed binary; its output is golden-file tested against the
daemon's generator.

### Option B: portal + `global` dispatcher binds (daemon-driven)

FlowShot registers `capture-region`, `capture-full`, and
`capture-active-monitor` with the XDG GlobalShortcuts portal (proposed
triggers: Print, Shift+Print, Ctrl+Print). Two Hyprland-specific realities
apply, both verified against xdg-desktop-portal 1.22.1 and
xdg-desktop-portal-hyprland 1.4.1 sources:

1. **Portal registration alone grabs no key.** XDPH drops the
   `preferred_trigger` hint (it registers shortcuts with an empty trigger),
   and Hyprland only fires portal shortcuts through user binds with the
   `global` dispatcher. You must bind the keys yourself:

   ```ini
   # hyprland.conf
   bind = ,Print,global,org.flowoss.FlowShot:capture-region
   bind = SHIFT,Print,global,org.flowoss.FlowShot:capture-full
   bind = CTRL,Print,global,org.flowoss.FlowShot:capture-active-monitor
   ```

   ```lua
   -- hyprland.lua
   hl.bind("Print", hl.dsp.global("org.flowoss.FlowShot:capture-region"))
   hl.bind("SHIFT + Print", hl.dsp.global("org.flowoss.FlowShot:capture-full"))
   hl.bind("CTRL + Print", hl.dsp.global("org.flowoss.FlowShot:capture-active-monitor"))
   ```

2. **The portal requires a registered app id.** xdg-desktop-portal rejects
   GlobalShortcuts sessions from processes it cannot identify ("An app id is
   required"). A native process qualifies when it runs under a systemd user
   unit named `app[-launcher]-<APPID>-<RANDOM>.scope` (or the matching
   `.service` form) AND `org.flowoss.FlowShot.desktop` is installed with an
   `Exec` line that resolves on `PATH` (GLib rejects unresolvable Exec lines;
   `desktop-file-validate` does not catch this). FlowShot's auto-spawn
   wraps the daemon in
   `systemd-run --user --scope --unit app-org.flowoss.FlowShot-<nonce>` when
   systemd is present, which satisfies the unit half; the packaged desktop
   file (packaging milestone, **TBD**) satisfies the other. On a non-systemd
   init, or without the desktop file installed, the portal registration fails
   cleanly and FlowShot falls back to option A's snippets.

The portal path needs a resident daemon (it dispatches through D-Bus); the
daemon's lifecycle treats registered shortcuts as a persistence reason, so it
stays alive automatically (see
[architecture/adr-004-daemon-lifecycle.md](architecture/adr-004-daemon-lifecycle.md)).

Known upstream quirk (not a FlowShot defect): compositor-side shortcut entries
can survive a portal client's death and linger in `hyprctl globalshortcuts`.
FlowShot closes its portal session explicitly on graceful shutdown; if stale
entries ever remain, `systemctl --user restart xdg-desktop-portal-hyprland`
clears them (the service is static/D-Bus-activated and restarts cleanly).

### Runtime binds during testing (Hyprland 0.56)

`hyprctl keyword` is dead on 0.56's Lua config parser ("keyword can't work
with non-legacy parsers. Use eval."). Runtime binds and unbinds go through
`hyprctl eval` with Lua expressions:

```sh
hyprctl eval 'hl.bind("F13", hl.dsp.exec_cmd("flowshot capture"))'
hyprctl eval 'hl.unbind("F13")'
```

The same applies to window rules at runtime (`hl.window_rule({...})`, whose
handle persists across `eval` calls until `:set_enabled(false)`).

## Screen-capture permissions (enforce_permissions)

Hyprland's permission system is off by default
(`ecosystem:enforce_permissions = false`, which is also the state of the QA
machine these docs were verified on). If you enable it, FlowShot's direct
Wayland capture needs pre-grant rules; without them the first capture pops an
interactive dialog, and a denied capture comes back as a black frame with a
centered denial notice (FlowShot classifies that frame and maps it to the
permission-denied outcome, exit code 5 in the CLI contract, rather than
saving it). Cursor position is a separate permission class
(`cursorpos`); denying it never fails a capture, it only degrades the
cursor-aware preselect to the first pointer motion after the overlay maps.

Config rules (both screencopy and cursorpos; adjust the path regex to your
install location):

```ini
# hyprland.conf - note: config permission rules require a Hyprland restart
ecosystem {
  enforce_permissions = true
}

permission = /usr/(bin|local/bin)/flowshot, screencopy, allow
permission = /usr/(bin|local/bin)/flowshot, cursorpos, allow
```

```lua
-- hyprland.lua
hl.config({ ecosystem = { enforce_permissions = true } })
hl.permission({ binary = "/usr/(bin|local/bin)/flowshot", type = "screencopy", mode = "allow" })
hl.permission({ binary = "/usr/(bin|local/bin)/flowshot", type = "cursorpos", mode = "allow" })
```

For a development build run out of the repo, add a second pair covering the
target directory:

```ini
permission = .*/target/(debug|release)/flowshot, screencopy, allow
permission = .*/target/(debug|release)/flowshot, cursorpos, allow
```

If you use the portal-based capture rungs (ScreenCast/Screenshot) on Hyprland,
the portal backend itself is a separate application in the permission system:
`permission = /usr/(lib|libexec|lib64)/xdg-desktop-portal-hyprland, screencopy, allow`
is the rule Hyprland's own documentation recommends for that case.

## Pin windows

Pins are plain xdg-toplevel windows with app id `flowshot-pin`. Hyprland
tiles new windows by default and ignores client resize requests on tiled
windows, so pins need a float rule (and `staysontop` if you want them above
other windows):

```ini
# hyprland.conf
windowrulev2 = float, class:^(flowshot-pin)$
windowrulev2 = staysontop, class:^(flowshot-pin)$
```

```lua
-- hyprland.lua
hl.window_rule({ match = { class = "flowshot-pin" }, float = true, stays_on_top = true })
```

Two Wayland-inherent behaviors to know (both live-verified during pin QA):

- Hyprland centers new floating windows on the monitor, and Wayland has no
  protocol for a client to request an initial position. Pins do not appear at
  the captured region; that is compositor policy, not a bug.
- Floating-window resizes are center-anchored by Hyprland. FlowShot's
  zoom-to-cursor compensates in its own geometry, so the image point under
  the cursor stays put across zoom steps.

## Tray

The status tray needs an SNI host. waybar's `tray` module works out of the
box (verified live); enable it with `[daemon] tray = true` (see
[config-reference.md](config-reference.md)).

## Not yet verified on this desktop

Nothing desktop-specific. The remaining open items are platform-wide
(packaging, plus the hardware-gated live checks) and are listed in the
README's project status.

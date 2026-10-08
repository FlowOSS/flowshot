# GNOME cursor position: the problem and the viable workarounds

Status: research note for the post-first-release track. Nothing here is
scheduled for v1, and nothing here is live-verified: no GNOME session exists
on the dev machine (the hardware-gated class in
[../verification.md](../verification.md)), so every path below is
protocol-level review only.

## The problem

Wayland's security model delivers `wl_pointer` events only to the surface
the pointer is over. A client that owns no hovered surface has no protocol
way to ask where the cursor is. Compositors that want to offer the position
do so out of band: Hyprland exposes an IPC socket (layer 2 of the cursor
ladder), wlroots and COSMIC implement the ICC pointer-cursor session (layer
1). Mutter implements neither and exposes no public cursor-position API at
all. That is the DOCUMENTED-DEGRADED row in the capability table
(`cursor_capabilities` in
`crates/flowshot-capture-wayland/src/resolve.rs`, evidence comment on the
`Gnome` arm) and in the GNOME guide ([../setup-gnome.md](../setup-gnome.md)).

Observable consequences on GNOME:

- `flowshot capture screen` with no argument cannot resolve "the output
  under the cursor" before the overlay maps.
- `--region WxH` cannot pre-center on the cursor.
- Both fall back to layer 3 (`CursorSource::AwaitFirstMotion`): the position
  arrives with the first `wl_pointer.motion` after the overlay maps, and the
  preselect follows it. Interactive capture works; pre-map preselect does
  not.

## Viable workarounds, ranked

### a. GNOME Shell extension exposing the position over D-Bus

The GNOME equivalent of the KWin-script bridge that `resolve.rs` lists on
the KDE roadmap (module docs, "KWin-script bridge for KDE (roadmap, NOT
v1)"). Shell extensions run inside the compositor process, so they can read
what no client can: on the GJS side, `global.get_pointer()` returns the
stage coordinates and `global.display.get_monitor_index_for_point(x, y)`
maps them to a monitor index. The extension exports a small D-Bus object;
FlowShot adds a GNOME layer that queries it, parallel to the Hyprland IPC
layer.

- UX: one install and enable, then zero dialogs per capture. The cleanest
  result of the three.
- Cost: the user must install and enable an extension; extension-facing
  mutter APIs are not stable across GNOME versions (shell-version keying,
  API churn); the extension adds its own failure modes (absent, disabled,
  version mismatch), each of which must degrade quietly to layer 3.
- Scope: shipping a shell extension is exactly what the v1 scope forbids
  (the README's platform-support wording records the degradation-only
  decision for GNOME cursor position). Read that as a v1 scope guard, not
  a technical verdict:
  v1 ships the documented degradation, and this note is the record for the
  post-release evaluation that may revisit it.

### b. PipeWire cursor metadata (`SPA_META_Cursor`)

When a ScreenCast portal stream runs with `cursor_mode=Metadata`, mutter
embeds the cursor position in the stream's buffer metadata
(`SPA_META_Cursor`), separate from the pixels. FlowShot's portal backend
already names this channel: `portal/screencast/backend.rs` notes that the
ScreenCast portal's only cursor channel is `CursorMode::Metadata`, which
the PipeWire metadata stream currently does not consume.

- Fit: on GNOME the capture ladder already lands on the ScreenCast portal
  rung (see
  [../architecture/adr-002-capture-backend-ladder.md](../architecture/adr-002-capture-backend-ladder.md)),
  so a stream exists at capture time anyway. Reading the metadata is an
  addition inside `crates/flowshot-capture-wayland/src/portal/`, not a new
  subsystem, and needs no install.
- Cost: stream creation still shows the portal's source/consent dialog (a
  restore token suppresses repeats where the portal implements persist).
  The position only exists while a stream runs: this can answer "capture
  the output under the cursor" at capture time, but cannot pre-warm a
  position before any portal session exists.
- Open questions for a live session: whether Metadata mode is accepted on
  the multi-stream top-up loop, and which coordinate space mutter reports
  (per-stream vs global). Both are guesses until run on real mutter.

### c. InputCapture / RemoteDesktop portals (libei)

The RemoteDesktop and InputCapture portals sit on libei and can expose
pointer position information with consent. Both are permission-dialog-gated
and designed for input forwarding (remote-desktop control, barrier
crossing), not for passive position queries.

- Viable only as a one-time-consent setup path: ask once, keep the token.
- Ranked last: the consent UX is heavier than (b)'s for what is a
  convenience feature, and the APIs are shaped for forwarding rather than
  querying, so mapping them onto "current cursor position" is indirect.

## Already covered by v1

The "map a transparent fullscreen overlay and read the first motion"
technique is not missing from FlowShot: it IS layer 3
(`CursorSource::AwaitFirstMotion`) and it ships, covering interactive
captures on every compositor, GNOME included. What it cannot cover is
pre-map preselect: resolving the cursor before any surface exists, which is
what argument-less `capture screen` and `at-cursor` need. The workarounds
above exist to close exactly that gap and no other.

## Rejected, with reasons

- **/dev/input via evdev**: mice and touchpads report relative deltas, not
  absolute positions, so reconstructing a position means integrating deltas
  from an unknown origin and the result drifts. It also requires `input`
  group membership or a udev rule, a security ask far out of proportion to
  a preselect convenience.
- **AT-SPI**: the accessibility bus exists for assistive technology. A
  screenshot tool querying it for pointer position is abusing it, it
  requires the a11y bus to be enabled, and AT-SPI offers no reliable
  "current pointer position" query to clients anyway. Unreliable and
  inappropriate.
- **GNOME Shell `Eval` D-Bus (unsafe mode)**: arbitrary JS eval in the
  shell would read `global.get_pointer()` trivially, but it has been locked
  down since GNOME 41 behind a developer unsafe-mode flag. Dev-only, never
  shippable.
- **Focus-shift inference**: deriving the position from focused windows
  fails because focus does not follow the cursor by default, and even
  focus-follows-mouse yields a window, not coordinates.
- **Hardware sniffing** (device-level or USB-level interception): out of
  scope for a screenshot tool; not reconsidered.

## Provenance

The techniques above trace to primary sources: the Wayland core protocol
semantics (`wl_pointer` enter/motion only over client surfaces), Mutter/GJS
internals (`global.get_pointer()`, the monitor-index lookup on
`global.display`), the XDG portal specifications (ScreenCast `cursor_mode`,
InputCapture, RemoteDesktop), and PipeWire's SPA metadata documentation
(`SPA_META_Cursor`). They were cross-checked against FlowShot's own cursor-ladder
capability table (now `cursor_capabilities` in
`crates/flowshot-capture-wayland/src/resolve.rs`) and the portal-screencast
implementation under `crates/flowshot-capture-wayland/src/portal/`.

## Post-release recommendation

If the gap is pursued after the first release: path (a), the shell
extension, gives the cleanest UX (one install, no per-capture dialogs, a
real pre-map position). Path (b), PipeWire metadata, needs no install but
keeps the portal consent dialog at stream creation and can only answer
while a stream runs. Path (c) stays on the shelf unless (a) and (b) both
prove out badly. Whichever is chosen, the first step is the same: stand up
a GNOME test session (none exists on the dev machine) and validate against
real mutter behavior before any code is written.

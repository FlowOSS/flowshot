# crates/flowshot-capture-wayland/src

49 files / ~13.9k LOC, 7 module dirs. The crate root covers the
safety policy and dependency pins; this covers the module map and the one
pattern every backend must follow.

## OVERVIEW

Three layers: a **connection layer** (one long-lived probe thread), five
**backend rungs** that each own short-lived one-shot connections, and **pure
decision modules** that hold every testable rule socket-free.

## STRUCTURE

| Layer | Modules | Owns |
|-------|---------|------|
| Connection | `thread.rs`, `session.rs`, `globals.rs`, `output.rs`, `dispatch.rs`, `transform.rs` | `CaptureThread` (private connection + `calloop`), `SessionSnapshot`, registry globals → `ProtocolGlobals`, `wl_output`+`zxdg_output` → `OutputInfo` |
| Async bridge | `worker.rs` | `spawn_worker`: blocking one-shot chain on a worker thread → future |
| Rung 1 ICC | `icc.rs` + `icc/{protocol,dispatch,run,shm,wait}.rs` | `ext-image-copy-capture-v1`, `memfd`+`wl_shm` buffers, deadline dispatch |
| Rung 2 screencopy | `screencopy.rs` + `screencopy/{protocol,dispatch,run}.rs` | `wlr-screencopy-unstable-v1` (niri, pre-0.19 wlroots) |
| Rung 3 KWin | `kwin.rs` + `kwin/{wire,meta,run,probe,error,stub,tests}.rs` | `org.kde.KWin.ScreenShot2` over D-Bus, fd-IN pipe payload |
| Rungs 4/5 portals | `portal.rs` + `portal/{screenshot,screencast,composite,streams,run,probe,error}.rs`, `portal/pipewire*`, `portal/screencast/*` | xdg-desktop-portal Screenshot + ScreenCast/PipeWire |
| Cursor | `cursor.rs` + `cursor/{protocol,dispatch,run,stream}.rs`, `resolve.rs`, `hyprland_ipc.rs`, `denial.rs` | position ladder, cursor image, event stream → `CursorStream`, denial classification |
| Shared pure | `stitch.rs`, `desktop.rs`, `error.rs` | multi-output region compositing, `DesktopEnv` sniff, typed error family |

Visibility rule: only `cursor`, `desktop`, `error`, `globals`, `resolve`,
`stitch` are `pub mod`; the backends are private `mod` + `pub use` at the crate
root, so the public surface is the type list in `lib.rs` and nothing else.

## THE PER-BACKEND PATTERN (copy it for a new rung)

1. `<name>.rs` — the public face: the backend struct, its `CaptureBackend`
   impl, and the long doc header stating the wire contract and its citation.
2. `<name>/protocol.rs` — a **plain-data sink** (`ActiveCapture`,
   `ActiveScreencopy`, …) plus pure functions turning sink state into buffer
   parameters. Socket-free by construction, so format negotiation, constraint
   completion and failure mapping are unit-testable with no compositor.
3. `<name>/dispatch.rs` — the `wayland_client::Dispatch` impls. They ONLY fill
   the sink; no decision logic lives here.
4. `<name>/run.rs` — the one-shot runner: fresh connection → request chain →
   deadline-bounded wait → teardown. Closing the connection is what releases
   every session/source/buffer compositor-side.

## CONVENTIONS

- Module-root style is uniform here: `<name>.rs` + `<name>/` (never
  `<name>/mod.rs`) — the opposite of `flowshot-daemon/src/execute`.
- One connection per capture run; the long-lived `CaptureThread` is for probing
  and output enumeration only.
- Errors are per-backend typed families (`IccError`, `ScreencopyError`,
  `KwinError`, `PortalScreenCastError`, `PortalScreenshotError`) implementing a
  shared `BackendError` contract so the worker bridge and deadline handling are
  backend-agnostic.
- Tests are inline `#[cfg(test)] mod tests` (29 files); `kwin/tests.rs` is the
  one sibling-file exception because it drives the p2p stub.

## ANTI-PATTERNS (THIS DIRECTORY)

- No async-native Wayland: the crate is worker-threads + `spawn_worker`, never
  a `calloop` future driving a capture.
- Screencopy needs **no inverse transform remap** — buffers arrive in the
  output's native orientation and only the renderer's `y_invert` correction
  applies. Adding a remap here double-corrects.
- The Screenshot portal's composite pixel space is **detected at runtime**
  against the enumerated layout (`portal/composite.rs`), never assumed.
- Hyprland permission denial arrives as a `ready` frame of black pixels, not a
  protocol error: `denial.rs` classifies it. Don't "fix" it in the ICC chain.
- The cursor ladder (`resolve.rs`) never fails: ICC one-shot → Hyprland IPC
  socket → `AwaitFirstMotion`. Layer 3 covers KDE and GNOME, so no KWin script
  bridge and no `hyprctl` spawn.

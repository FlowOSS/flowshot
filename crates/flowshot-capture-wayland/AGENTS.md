# crates/flowshot-capture-wayland

**Generated:** 2026-10-06 | **Commit:** 36714a7 | **Branch:** main
Score 17 (high complexity) — 57 files / ~14.8k LOC, 9 dirs. THE platform crate:
every compositor- and D-Bus-facing line in the workspace lives here, and it is
deliberately NOT purity-gated.

## OVERVIEW

Five `CaptureBackend` implementations (ladder rungs 1-5), a dedicated capture
thread owning a private `wayland_client::Connection` + `calloop` loop, registry
capability probing, output enumeration, cursor strategy, and region stitching.

## STRUCTURE

```
flowshot-capture-wayland/
├── src/       # 49 files — module map in src/AGENTS.md
└── examples/  # probe, capture_icc, capture_screencopy, portal_capture,
               # cursor_pos, resolve_cursor (+ common/ shared harness)
```

## SAFETY POLICY (read before touching buffers)

This is the workspace's only `unsafe`-exempt crate — and it currently contains
**zero `unsafe` blocks**. Capture buffers are anonymous files (`memfd` via
`nix`) read back with ordinary file I/O instead of being memory-mapped, so the
exemption is unused headroom for a future zero-copy path, not a license. If you
add `unsafe`, it needs a `SAFETY:` comment and an entry in the crate audit; the
root handoff's "audited allow-list" wording describes that intent, not present
code.

## DEPENDENCY NOTES (each pin has a reason in Cargo.toml)

- `ashpd` with `screenshot` + `screencast` features — 0.13 gates every portal
  behind its own feature; default is only the tokio reactor.
- `url` 2.5 — ashpd 0.13 replaced `url::Url` with a minimal `Uri` that has no
  `to_file_path`, so the Screenshot portal's `file:` URI is decoded here.
- `zbus` with `p2p` — the KWin `ScreenShot2` stub tests run on a private
  peer-to-peer bus (hermetic, no session bus needed).
- `pipewire` — the ScreenCast portal rung only; single-frame one-shot sessions.

## ANTI-PATTERNS (THIS CRATE)

- **Never ride the UI toolkit's connection.** winit never exposes its
  `wl_surface`/registry for capture; this crate always owns a private
  connection, which also keeps big buffer events off the UI's frame pacing.
- **No blocking Wayland call off the capture thread.** Foreign threads exchange
  bounded request/reply messages with `CaptureThread`; one-shot runs go through
  `worker::spawn_worker` with a hard deadline (10 s per phase).
- **No shell-outs.** `hyprctl` is never spawned — the Hyprland IPC socket is
  spoken directly (`hyprland_ipc.rs`).
- **Never pin a wire contract from memory.** `kwin/stub.rs` and `kwin/meta.rs`
  cite the fetched upstream sources for the vardict keys and `QImage::Format`
  values; re-fetch before changing them.
- `wl_shm` buffers only in v1 — `zwp_linux_dmabuf_v1` is detected, not used.

## COMMANDS

```bash
just probe                                    # registry/capability dump on the live session
cargo test -p flowshot-capture-wayland        # 29 inline suites + the kwin p2p stub tests
cargo run -p flowshot-capture-wayland --example capture_icc   # live session required
```

## NOTES

- `kwin/tests.rs` + `kwin/stub.rs` are the load-sensitive class: both p2p sides
  must be built CONCURRENTLY (a server built alone blocks on the client's SASL
  handshake). Raise their budgets rather than weaken assertions.
- The examples are the live-QA oracles for the protocol work; they need a real
  Wayland session and are never run in CI.
- Fractional scale: `wp_fractional_scale_v1` is detected but not round-tripped,
  so the reported scale stays integer (recorded limitation in `lib.rs`).

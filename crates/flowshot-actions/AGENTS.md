# crates/flowshot-actions

High complexity: 21 files / 3.4k LOC across 4 domains. NOT
purity-gated: this crate is Wayland-native **by design** because it owns the
clipboard offer, and the gate's scope note records that decision.

## OVERVIEW

Everything that happens after a capture exists: save/encode/open, the ordered
post-capture action set, the daemon-owned clipboard, Imgur upload, and the pin
registry that feeds the daemon's lifecycle.

## STRUCTURE

```
flowshot-actions/src/
├── clipboard.rs + clipboard/   # actions.rs (the ordered action set), backend.rs (routes),
│                               # offer.rs (MIME builders), pipeline.rs (the executor, 546 L),
│                               # keepalive.rs (GNOME portal-only state machine)
├── export/                     # mod.rs (NotifySink/FileDialogSink traits), path.rs, pattern.rs,
│                               # encode.rs, open.rs (OpenURI portal), stdout.rs (--raw/--print-geometry)
├── upload/                     # mod.rs (Uploader trait), imgur.rs (API v3), history.rs (JSONL), delete.rs
├── pin.rs + pin/registry.rs    # copy_pin/save_pin + the multi-pin bookkeeping
└── error.rs                    # ExportError, ClipboardError, UploadError, PinError
```

## WHERE TO LOOK

| Task | Location |
|------|----------|
| Add a post-capture action | `clipboard/actions.rs` (`Action`, `effective_actions`, `execution_order`) then wire it in `clipboard/pipeline.rs` |
| Filename patterns / collision numbering | `export/pattern.rs` (`DEFAULT_PATTERN`, `expand_pattern`, `sanitize_filename`) + `export/path.rs` (`next_available_path`) |
| Clipboard MIME behavior | `clipboard/offer.rs` — the policy is in `clipboard.rs`'s header (`image/png` always, `image/jpeg` on config, `copy-path` = `text/plain` + `text/uri-list`) |
| A new upload provider | implement `Uploader` in `upload/`; `Imgur` is the reference, `history.rs` records the deletehash |
| Pin lifecycle → daemon residency | `pin/registry.rs` (non-empty registry = the "pins alive" persistence reason) |

## CONVENTIONS

- **The daemon owns the clipboard offer.** `WaylandClipboard` serves it from a
  thread inside the calling process over `zwlr_data_control`
  (`wl-clipboard-rs`); the capturing UI exits freely and the offer dies with the
  daemon — which is why a held offer pins the daemon's lifecycle.
- Two clipboard routes, chosen by `probe_data_control` + `select_route`:
  `DataControl` (wlroots/Hyprland/KDE — the live-verified class) and
  `GnomeKeepAlive` (portal-only GNOME; lazy offer, notify-on-first-access,
  500 ms `SAFETY_CLOSE`). The keep-alive path is unit-level only, live QA deferred.
- The ordered `[save].actions` set `{copy, copy-path, save, pin, upload, notify,
  open-with}` replaces Flameshot's `saveAfterCopy`-style booleans.
- No dependency on `flowshot-ui` is possible (purity runs the other way): the UI
  exposes `pins::PinActionSink` and the **binary layer bridges** the two, converting
  `PinSnapshot` → `PinImage` (same field shape, deliberately no shared type).
- `[upload].client_id` ships **empty**; an upload with no client id returns
  `UploadError::ConfigurationMissing` immediately (no freeloading on a shared pool).

## ANTI-PATTERNS (THIS CRATE)

- No shell-outs: never `wl-copy`, `xclip`, `grim` or a browser subprocess. The
  OpenURI portal (`ashpd`, `export/open.rs` and `upload/delete.rs`) is the only
  way out to another application.
- The library never opens a dialog itself — it calls the injected
  `FileDialogSink`. Its production impl is a pictures-directory stand-in in
  `flowshot-daemon/src/execute/post/sinks.rs`.
- `rfd` is declared in `Cargo.toml` but imported nowhere in the workspace; don't
  assume a dialog backend exists, and remove the dep if you touch this crate.
- Notification gating is not this crate's job: `pipeline.rs` applies the
  `[daemon].notifications` gate for the whole action run, and `NotifySink`
  callbacks stay dumb.

## COMMANDS

```bash
cargo test -p flowshot-actions   # 14 inline suites; wiremock serves the Imgur tests
```

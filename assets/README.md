# FlowShot brand assets

| File | What it is |
|---|---|
| `logo.svg` | **Source of truth.** The FlowShot mark: a violet-to-azure gradient circle with white concentric arcs and a center dot (reads as a lens/target). Inkscape-authored, 200x200 viewBox. Every derived raster is regenerated from this file - edit this, never a derivative. |
| `logo.png` | 2048x2048 RGBA raster export of `logo.svg`, for surfaces that cannot render SVG (the README header on GitHub, icon-tooling input). |

## Consumers

- `crates/flowshot-daemon/build.rs` rasterizes `logo.svg` at build time
  (resvg, the workspace pin) into the SNI tray pixmap sizes 16/22/24/32/48 -
  only those raw ARGB32 bytes are embedded in the daemon binary, never the
  PNG.
- The top-level [README](../README.md) header shows `logo.png` at 128px.
- Packaging installs the mark as the themed icon `org.flowoss.FlowShot`
  (targets and generation rules: [packaging/ICONS.md](../packaging/ICONS.md)).

## License

Both files are FlowShot's own assets, created for this project, and are
covered by the project license: **GPL-3.0-or-later** (see
[LICENSE](../LICENSE)). They contain no third-party material, so the
[NOTICES](../NOTICES) file carries no entry for them.

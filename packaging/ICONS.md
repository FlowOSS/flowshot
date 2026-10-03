# Icon install targets (packaging prep)

Recorded now, executed when the packaging milestone lands (AUR / Nix /
Flatpak are user-gated TBDs). The themed icon name is **`org.flowoss.FlowShot`**
- already referenced by [flowshot.desktop.in](flowshot.desktop.in) (`Icon=`)
and the future AppStream metainfo.

Source of truth: [`assets/logo.svg`](../assets/logo.svg) (see
[assets/README.md](../assets/README.md)).

## Install targets (hicolor theme, prefix-relative)

| Target | From |
|---|---|
| `share/icons/hicolor/scalable/apps/org.flowoss.FlowShot.svg` | `assets/logo.svg`, copied verbatim |
| `share/icons/hicolor/48x48/apps/org.flowoss.FlowShot.png` | rendered from the SVG at package time |
| `share/icons/hicolor/64x64/apps/org.flowoss.FlowShot.png` | " |
| `share/icons/hicolor/128x128/apps/org.flowoss.FlowShot.png` | " |
| `share/icons/hicolor/256x256/apps/org.flowoss.FlowShot.png` | " |
| `share/icons/hicolor/512x512/apps/org.flowoss.FlowShot.png` | " |

PNG generation at package time with any resvg-class rasterizer (the repo's
own pin is resvg 0.47 - `crates/flowshot-daemon/build.rs` is a working
reference; `rsvg-convert` / ImageMagick are equally fine). Do NOT ship
`assets/logo.png` (2048x2048) as a hicolor size; regenerate from the SVG.

After `gtk-update-icon-cache` runs (package hooks), the tray can switch its
SNI `IconName` property to `org.flowoss.FlowShot` (hosts prefer it over the
embedded pixmaps) - the seam is `tray/item.rs`, which records why the name
stays empty until the icon is actually installed.

The Flatpak bundle uses the same name under `share/icons/hicolor/` inside
the app prefix; the `.desktop` file must be named `org.flowoss.FlowShot.desktop`.

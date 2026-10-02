# Configuration reference

FlowShot reads one TOML file: `~/.config/flowshot/flowshot.toml` (XDG config
home respected). The same content ships as the `flowshot-config(5)` man page.

This document mirrors `crates/flowshot-core/src/config.rs` (schema version 3).
It is hand-synced to the source; field names below are the exact TOML keys.

## Loading semantics

- A missing or corrupt file never breaks a capture: defaults apply and a
  warning is logged.
- Unknown keys are ignored (forward compatibility).
- Every group and every key is optional; partial files are filled with
  defaults.
- `config_version` drives migration: unversioned files are treated as
  version 0 and migrated forward step by step; a file written by a newer
  FlowShot is rejected with a clear error.
- Serialization is byte-stable: saving and reloading produces identical TOML.

## Top level

| Key | Type | Default | Notes |
|---|---|---|---|
| `config_version` | integer | `3` | Schema version. Managed by FlowShot; do not edit. |

Migration history: v0 (unversioned legacy) gains the version stamp; v1 to v2
renames `[save] filename_template` to `filename_pattern` (an explicit
`filename_pattern` always wins over the stale key); v2 to v3 adds the
`[telemetry]` group with every consent flag false.

## `[capture]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `hide_cursor` | boolean | `false` | Exclude the mouse cursor from captures. |
| `save_last_region` | boolean | `true` | Remember the last selected region between sessions (enables `flowshot capture last`). |
| `last_region` | table | absent | Persisted state, written by FlowShot after a capture (`x`, `y`, `width`, `height`; signed x/y, global logical pixels). Omitted from the file until the first capture. Not meant to be hand-edited. |

## `[save]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `path` | string | `""` | Save directory. Empty means the platform pictures directory. |
| `path_fixed` | boolean | `false` | When true, always save to `path` without prompting. |
| `extension` | string | `"png"` | Default file extension (`png`, `jpg`, `webp`). |
| `filename_pattern` | string | `"%F_%H-%M"` | `strftime`-style pattern; `%` placeholders expanded at save time, `%%` is a literal percent, `/` and `:` in the result are sanitized. |
| `jpeg_quality` | integer (1-100) | `75` | Quality for JPEG saves and JPEG clipboard offers. |
| `clipboard_format` | string | `"png"` | Clipboard image format: `png` or `jpeg`. `image/png` is always offered; JPEG is added to the offer when set. |
| `actions` | array of strings | `["copy"]` | Ordered set of post-capture actions. See below. |

### The `actions` ordered set

Valid entries: `copy`, `save`, `pin`, `upload`, `copy-path`, `notify`,
`open-with`. This single ordered list replaces Flameshot's soup of
interacting booleans (`saveAfterCopy`, `copyPathAfterSave`, ...). Rules:

- Actions run in list order.
- Per-invocation CLI flags (`-c`, `-o`, `--pin`, `--upload`) merge into the
  configured set for that one capture: configured order first, flag-implied
  additions deduplicated at the end. Flags never write the config file.
- `copy-path` is relocated to just after the last `save` in the effective
  sequence (only entries violating that order move). Without a successful
  save, `copy-path` and `open-with` warn and no-op.
- Execution is best-effort: a failed action is recorded and the sequence
  continues.
- When an image copy and `copy-path` happen in the same run, the file's
  `text/uri-list` is appended to one combined clipboard offer instead of
  clobbering the image.
- `notify` requests an explicit success toast, gated by
  `[daemon] notifications`.

## `[editor]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `draw_color` | string | `"#FF0000"` | Active drawing color, `#RRGGBB`. |
| `draw_thickness` | integer | `3` | Stroke thickness in pixels for drawing tools. |
| `font_family` | string | `"Noto Sans"` | Text tool font family (resolved by the platform font stack). |
| `font_size` | integer | `8` | Text tool font size. |
| `magnifier` | boolean | `false` | Show the pixel magnifier while selecting or drawing. In-session toggle: `L`. A settings apply re-projects this key (the config wins over the session toggle). |
| `magnifier_shape` | string | `"square"` | `square` or `circle`. |
| `hud_position` | integer | `4` | Geometry HUD corner: 1 top-left, 2 top-right, 3 bottom-left, 4 bottom-right. |
| `hud_hide_time` | integer | `3000` | Milliseconds of inactivity before the HUD auto-hides. |
| `grid` | boolean | `false` | Show the snapping grid. In-session toggle: `F`. |
| `undo_limit` | integer | `100` | Maximum undo steps kept per session. `0` disables history. |
| `color_palette` | array of strings | 20 swatches | Color-picker swatches, `#RRGGBB`. Default listed below. |
| `double_click_copies` | boolean | `false` | Double-clicking the selection copies it immediately. |
| `side_panel` | boolean | `true` | Enable the tool-options side panel feature. When false, the Space toggle is inert and the panel never paints. The panel starts hidden regardless; Space opens it. |

Default `color_palette`:

```toml
color_palette = [
  "#000000", "#7F7F7F", "#880015", "#ED1C24", "#FF7F27",
  "#FFF200", "#22B14C", "#00A2E8", "#3F48CC", "#A349A4",
  "#FFFFFF", "#C3C3C3", "#B97A57", "#FFAEC9", "#FFC90E",
  "#EFE4B0", "#B5E61D", "#99D9EA", "#7092BE", "#C8BFE7",
]
```

## `[tools.arrow]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `style` | string | `"straight"` | `straight` or `curved` (quadratic shaft). |
| `reverse` | boolean | `false` | Draw the arrow head at the start point. |

## `[tools.marker]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `size` | integer | `5` | Highlighter stroke width in pixels. |

## `[tools.pixelate]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `size` | integer | `2` | Pixel block size. The blur tool shares this size slot. There is no insecure pixelate mode to configure; see below. |

## `[tools.rectangle]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `corner_radius` | integer | `1` | Rectangle corner radius in pixels. |

## `[tools.counter]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `size` | integer | `1` | Counter size unit; badge radius is `size * 8 + 8` px. |
| `outline` | boolean | `true` | Draw an outline around counter badges. |

## `[pin]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `min_size` | integer | `100` | Minimum pin window size (width and height) in pixels. |

## `[upload]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `provider` | string | `"imgur"` | Upload provider identifier. Imgur is the shipped provider, behind a pluggable trait. |
| `client_id` | string | `""` | Provider API client id. Empty means unconfigured: upload stays disabled and `--upload` fails with a usage error naming this key. FlowShot ships no shared client id; register your own (Imgur API registration). |
| `without_confirmation` | boolean | `false` | Upload without a confirmation prompt. |
| `copy_url` | boolean | `true` | Copy the upload URL to the clipboard when the upload finishes. |
| `history_max` | integer | `25` | Maximum upload history entries kept (URL + delete hash, JSONL). |

## `[ui]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `accent_color` | string | `"#6366F1"` | Accent color, `#RRGGBB`. Seeded from the design-token palette. |
| `contrast_color` | string | `"#0F172A"` | Contrast/backdrop color, `#RRGGBB`. Seeded from the design-token palette. |
| `dim_opacity` | integer (0-255) | `190` | Opacity of the dim layer over the frozen backdrop. |
| `toolbar_buttons` | array of strings | see below | Toolbar button order, by tool identifier. |

Default `toolbar_buttons`:

```toml
toolbar_buttons = [
  "arrow", "rectangle", "circle", "marker", "text", "pixelate",
  "counter", "copy", "save", "pin", "upload", "undo",
]
```

## `[daemon]`

| Key | Type | Default | Notes |
|---|---|---|---|
| `tray` | boolean | `false` | Show a system tray icon while the daemon runs. Needs an SNI host (waybar tray, Plasma tray, GNOME AppIndicator extension). |
| `notifications` | boolean | `true` | Desktop notifications for capture events. Gates every toast, including the one-time shortcut-registration nudge. |
| `startup_launch` | boolean | `false` | Install an autostart entry that launches the daemon at login. |

## `[telemetry]`

Opt-in error telemetry to FlowShot's self-hosted Sentry instance. Nothing is
collected while `enabled` is false — the client is never even initialized (no
network, no threads). The endpoint DSN is a build-time constant, never a
config key. The first-launch consent dialog asks once and writes the answers
back to this group.

| Key | Type | Default | Notes |
|---|---|---|---|
| `enabled` | boolean | `false` | Master switch (the dialog recommends ON). When false, telemetry is fully inert. |
| `include_technical_details` | boolean | `false` | Tier-2 technical payload: full GPU adapter string, exact kernel release, monitor layout (connector + size + scale), and a per-install random UUID. Privacy/GDPR-relevant (the dialog recommends OFF). Without it, events carry only the coarse tier-1 taxonomy (distro, arch, package manager, session type, desktop family, GPU family) and path-scrubbed error reports. |
| `asked_on_first_launch` | boolean | `false` | Managed by FlowShot: records that the consent dialog was answered so it is never shown again. Not meant to be hand-edited. |

The per-install UUID lives at `~/.local/share/flowshot/telemetry-id` (XDG
data home respected); deleting the file regenerates it.

## Deliberate omissions (Amendment #3)

FlowShot is capability-compatible with Flameshot, not configuration-compatible.
The following Flameshot config keys were dropped by design and have no
equivalent here:

- `insecurePixelate`: the pixelate tool is secure-only (fringe sampling,
  zero interior reads). There is no reversible mode. See
  [architecture/adr-005-secure-pixelate.md](architecture/adr-005-secure-pixelate.md).
- `showQuitPrompt`, `showHelp`, `predefinedColorPaletteLarge`,
  `keepOpenAppLauncher`, `historyConfirmationToDelete`: anti-bloat drops.
- `allowMultipleGuiInstances`: the overlay is natively multi-window; the
  concept does not exist.
- `antialiasingPinZoom`: pin zoom is always antialiased.
- `autoCloseIdleDaemon`: replaced by the smart lifecycle (the daemon persists
  only while a reason holds; see
  [architecture/adr-004-daemon-lifecycle.md](architecture/adr-004-daemon-lifecycle.md)).
- `showDesktopNotification` / `showAbortNotification`: merged into the single
  `[daemon] notifications` boolean.
- `captureActiveMonitor` as a standalone flag: replaced by
  `flowshot capture screen` with no argument (output under the cursor).
- Flameshot's shared default Imgur client id: `[upload] client_id` ships
  empty. No pool freeloading, no terms-of-service entanglement.
- Flameshot's config-mutating CLI flags (`--maincolor`, `--filename`,
  `--trayicon`, ...): configuration is owned by this file and the settings
  UI, never by CLI flags.
- Flameshot ini import: none. Configs start clean.

## Planned keys not yet in the schema

Planned for a future schema revision; do not add them to your file expecting
behavior:

- `[editor] mouse_preview` (spec default true; currently editor-side state
  only).
- A `[shortcuts]` group for rebinding editor/overlay keys (F12 defaults are
  currently compiled in).
- `[ui] show_side_panel_button` (toolbar panel-toggle button gate).

## Complete default file

What a fresh `flowshot.toml` contains after FlowShot first writes it
(`last_region` appears only after a capture):

```toml
config_version = 3

[capture]
hide_cursor = false
save_last_region = true

[save]
path = ""
path_fixed = false
extension = "png"
filename_pattern = "%F_%H-%M"
jpeg_quality = 75
clipboard_format = "png"
actions = ["copy"]

[editor]
draw_color = "#FF0000"
draw_thickness = 3
font_family = "Noto Sans"
font_size = 8
magnifier = false
magnifier_shape = "square"
hud_position = 4
hud_hide_time = 3000
grid = false
undo_limit = 100
color_palette = ["#000000", "#7F7F7F", "#880015", "#ED1C24", "#FF7F27", "#FFF200", "#22B14C", "#00A2E8", "#3F48CC", "#A349A4", "#FFFFFF", "#C3C3C3", "#B97A57", "#FFAEC9", "#FFC90E", "#EFE4B0", "#B5E61D", "#99D9EA", "#7092BE", "#C8BFE7"]
double_click_copies = false
side_panel = true

[tools.arrow]
style = "straight"
reverse = false

[tools.marker]
size = 5

[tools.pixelate]
size = 2

[tools.rectangle]
corner_radius = 1

[tools.counter]
size = 1
outline = true

[pin]
min_size = 100

[upload]
provider = "imgur"
client_id = ""
without_confirmation = false
copy_url = true
history_max = 25

[ui]
accent_color = "#6366F1"
contrast_color = "#0F172A"
dim_opacity = 190
toolbar_buttons = ["arrow", "rectangle", "circle", "marker", "text", "pixelate", "counter", "copy", "save", "pin", "upload", "undo"]

[daemon]
tray = false
notifications = true
startup_launch = false

[telemetry]
enabled = false
include_technical_details = false
asked_on_first_launch = false
```

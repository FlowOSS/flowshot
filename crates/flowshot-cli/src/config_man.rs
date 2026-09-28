//! The hand-authored `flowshot-config.5` roff source (man
//! pages via `clap_mangen` for `flowshot.1` + this file-format page for the
//! TOML config). `examples/man_pages.rs` writes it next to the generated
//! `flowshot.1`; the schema documented here mirrors `flowshot_core::config`
//! (`config_version` 2).

/// The complete `flowshot-config.5` page.
pub const FLOWSHOT_CONFIG_ROFF: &str = r#".TH FLOWSHOT-CONFIG 5 "2026-09-26" "FlowShot 0.1.0" "File Formats Manual"
.SH NAME
flowshot-config \- FlowShot TOML configuration file
.SH SYNOPSIS
.I $XDG_CONFIG_HOME/flowshot/flowshot.toml
(usually
.IR ~/.config/flowshot/flowshot.toml )
.SH DESCRIPTION
FlowShot reads its configuration from a single TOML file. A missing,
unreadable, or corrupt file never fails a command: the defaults documented
below apply and a warning is logged. Unknown keys are ignored; missing keys
fall back to their defaults. The top\-level
.B config_version
integer (currently
.BR 2 )
drives forward migration of the grouped schema.
.PP
Configuration is owned by this file and the settings UI
.RB ( "flowshot settings" );
command\-line flags never mutate configuration (a deliberate deviation from
Flameshot, whose config\-mutating flags were dropped).
.SH "CAPTURE GROUP \- [capture]"
.TP
.BR hide_cursor " = " false
Exclude the mouse cursor from captures (per\-invocation override:
.BR "flowshot capture \-\-hide\-cursor" ).
.TP
.BR save_last_region " = " true
Remember the last selected region between sessions.
.TP
.B last_region
Persisted state (written by FlowShot, not hand\-edited): the last selected
region as
.BR x ", " y ", " width ", " height
in global logical pixels. Used by
.BR "flowshot capture last" " and " \-\-last\-region .
.SH "SAVE GROUP \- [save]"
.TP
.BR path " = \(dq\(dq"
Save directory; empty means the platform pictures directory.
.TP
.BR path_fixed " = " false
When true, always save to
.B path
without prompting.
.TP
.BR extension " = \(dqpng\(dq"
Default file extension
.RB ( png ", " jpg ", ...)."
.TP
.BR filename_pattern " = \(dq%F_%H\-M\(dq"
Filename pattern with strftime\-style placeholders; collisions are
numerated.
.TP
.BR jpeg_quality " = " 75
JPEG quality (1\-100) when saving or copying as JPEG.
.TP
.BR clipboard_format " = \(dqpng\(dq"
Image encoding placed on the clipboard
.RB ( png " or " jpeg ).
.TP
.BR actions " = [\(dqcopy\(dq]"
Ordered default\-action set executed when the editor is closed. Vocabulary:
.BR copy ", " copy\-path ", " save ", " pin ", " upload ", " notify ", "
.BR open\-with .
Per\-invocation flags
.RB ( \-c ", " \-\-pin ", " \-\-upload ", " \-o )
merge into this set for one capture. Replaces Flameshot's boolean
saveAfterCopy/copyPathAfterSave/copyURLAfterUpload flags.
.SH "EDITOR GROUP \- [editor]"
.TP
.BR draw_color " = \(dq#FF0000\(dq"
Active drawing color.
.TP
.BR draw_thickness " = " 3
Stroke thickness in pixels for drawing tools.
.TP
.BR font_family " = \(dqNoto Sans\(dq"
Font family for the text tool.
.TP
.BR font_size " = " 8
Font size for the text tool.
.TP
.BR magnifier " = " false
Show the pixel magnifier while selecting or drawing.
.TP
.BR magnifier_shape " = \(dqsquare\(dq"
Magnifier shape
.RB ( square " or " circle ).
.TP
.BR hud_position " = " 4
Geometry\-HUD corner: 1 top\-left, 2 top\-right, 3 bottom\-left, 4
bottom\-right.
.TP
.BR hud_hide_time " = " 3000
Milliseconds of inactivity before the HUD auto\-hides.
.TP
.BR grid " = " false
Show a snapping grid in the editor.
.TP
.BR undo_limit " = " 100
Maximum undo steps kept per session.
.TP
.B color_palette
Array of swatches offered by the color picker (20 FlowShot brand defaults).
.TP
.BR double_click_copies " = " false
Double\-clicking the selection copies it immediately.
.TP
.BR side_panel " = " true
Show the side panel with tool options.
.SH "TOOL GROUPS \- [tools.*]"
.TP
.B [tools.arrow]
.BR style " = \(dqstraight\(dq ( straight " or " curved ), "
.BR reverse " = " false
(arrow head at the start point).
.TP
.B [tools.marker]
.BR size " = " 5
(highlighter stroke width in pixels).
.TP
.B [tools.pixelate]
.BR size " = " 2
(pixel block size; the pixelate tool is secure\-only and irreversible by
design).
.TP
.B [tools.rectangle]
.BR corner_radius " = " 1 .
.TP
.B [tools.counter]
.BR size " = " 1
(starting counter value),
.BR outline " = " true .
.SH "PIN GROUP \- [pin]"
.TP
.BR min_size " = " 100
Minimum window size (width and height) of pinned images, in pixels.
.SH "UPLOAD GROUP \- [upload]"
.TP
.BR provider " = \(dqimgur\(dq"
Upload provider identifier.
.TP
.BR client_id " = \(dq\(dq"
Provider API client id. EMPTY BY DEFAULT: upload stays disabled until the
user registers their own client id (no shared\-pool freeloading);
.B flowshot capture \-\-upload
exits 2 with a settings hint while unconfigured.
.TP
.BR without_confirmation " = " false
Upload without asking for confirmation.
.TP
.BR copy_url " = " true
Copy the upload URL to the clipboard when done.
.TP
.BR history_max " = " 25
Maximum upload\-history entries kept (with deletehashes for removal).
.SH "UI GROUP \- [ui]"
.TP
.BR accent_color " = \(dq#6366F1\(dq"
Accent color (FlowShot brand indigo).
.TP
.BR contrast_color " = \(dq#0F172A\(dq"
Contrast color (FlowShot brand slate).
.TP
.BR dim_opacity " = " 190
Background dim opacity (0\-255).
.TP
.B toolbar_buttons
Toolbar button order by tool identifier (default: arrow, rectangle, circle,
marker, text, pixelate, counter, copy, save, pin, upload, undo).
.SH "DAEMON GROUP \- [daemon]"
.TP
.BR tray " = " false
Show a system tray icon while the daemon runs (a tray is a persistence
reason for the smart daemon lifecycle).
.TP
.BR notifications " = " true
Enable desktop notifications for capture events.
.TP
.BR startup_launch " = " false
Launch the daemon automatically at session startup (writes the XDG
autostart entry).
.SH FILES
.TP
.I ~/.config/flowshot/flowshot.toml
The configuration file.
.TP
.I ~/.config/flowshot/shortcuts\-restore.json
Global\-shortcut re\-registration state (managed by the daemon, not
hand\-edited).
.SH "SEE ALSO"
.BR flowshot (1)
"#;

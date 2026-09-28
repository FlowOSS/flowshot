//! The side panel paint half (split from [`super`] at the 250-LOC ceiling -
//! the `events.rs`/`events/pointer.rs` discipline): walks the stored
//! [`SidePanelLayout`](super::SidePanelLayout) control rects and emits the
//! display-list commands. Text positions derive from the layout rects, so
//! paint and hit-test can never disagree about a control's place.

use flowshot_core::config::ArrowStyle;
use flowshot_core::geometry::{LogicalRect, OutputInfo};
use flowshot_core::tokens::DesignTokens;

use crate::chrome::side_panel::{
    PANEL_BG_ALPHA, PANEL_WIDTH, SidePanelLayout, layer_icon_edge, layout, row_height,
    slide_offset, toggle_row_height, toggle_track_width, z_button_edge,
};
use crate::chrome::toolbar::icon_for_tool;
use crate::editor::{EditorState, MAX_TOOL_SIZE, ToolKind};
use crate::render::{Color, DisplayList, Point, Rect, ShadowSpec, Shape, TextCommand, TextureId};
use crate::widgets::{IconButton, Slider, Toggle, icons::Icon};

/// The panel's text style (one derivation, every label shares it).
struct Labels {
    font_size: f32,
    line: f32,
    family: String,
    color: Color,
}

impl Labels {
    fn text(&self, list: &mut DisplayList, x: f32, y: f32, text: String) {
        list.text(TextCommand {
            position: Point::new(x, y),
            text,
            font_size: self.font_size,
            line_height: self.line,
            color: self.color,
            family: Some(self.family.clone()),
            max_width: None,
        });
    }

    /// A label vertically centered in a control row.
    fn row_text(&self, list: &mut DisplayList, row: Rect, text: String) {
        self.text(
            list,
            row.origin.x,
            row.origin.y + (row.size.height - self.line) / 2.0,
            text,
        );
    }
}

/// The layer-row icon for a scene `type_id` token (the layer model
/// carries the scene token, which differs from the toolbar id for exactly
/// one kind: the scene says `ellipse`, the toolbar says `circle`).
#[must_use]
fn layer_icon(kind: &str) -> Icon {
    if kind == "ellipse" {
        return Icon::Circle;
    }
    ToolKind::from_id(kind).map_or(Icon::Square, icon_for_tool)
}

/// Draws the panel (the caller gates on visibility: the Space toggle AND
/// the `[editor].side_panel` config key). `slide` is the slide-in
/// progress (0 = hidden at the selection edge, 1 = resting); hit-testing
/// stays on the untranslated layout.
pub(crate) fn draw(
    list: &mut DisplayList,
    editor: &EditorState,
    tokens: &DesignTokens,
    scale: f32,
    atlas: TextureId,
    selection: LogicalRect,
    output: &OutputInfo,
    slide: f64,
) {
    let resting = layout(editor, selection, tokens, scale, output);
    let dx = slide_offset(
        &resting,
        crate::editor::paint::local_rect(output, selection).origin.x,
        PANEL_WIDTH * scale,
        slide,
    );
    let panel = resting.translated(dx);
    let contrast = Color::from_hex_token(&tokens.palette.contrast)
        .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
    let radius = tokens.radii.medium as f32 * scale;
    if let Some(spec) = ShadowSpec::from_token(&tokens.shadows.large, scale) {
        list.shadow(panel.rect, radius, spec);
    }
    list.fill(
        Shape::Rect {
            rect: panel.rect,
            radius,
        },
        contrast.with_alpha8(PANEL_BG_ALPHA),
    );
    let labels = Labels {
        font_size: tokens.typography.base_size as f32 * scale,
        line: tokens.typography.base_size as f32 * scale * 1.2,
        family: tokens.typography.family.clone(),
        color: contrast.readable_ink(),
    };
    draw_tool_section(list, editor, tokens, scale, &panel, &labels);
    draw_layers(list, editor, tokens, scale, atlas, &panel, &labels);
}

/// The per-tool options section (every kind with a control also owns a size
/// label, so the label table is the section gate - layout agrees).
fn draw_tool_section(
    list: &mut DisplayList,
    editor: &EditorState,
    tokens: &DesignTokens,
    scale: f32,
    panel: &SidePanelLayout,
    labels: &Labels,
) {
    let kind = editor.active_tool();
    let padding = tokens.spacing.medium as f32 * scale;
    let gap = tokens.spacing.small as f32 * scale;
    let cx = panel.rect.origin.x + padding;
    if let Some(label) = kind.and_then(super::size_label) {
        labels.text(
            list,
            cx,
            panel.rect.origin.y + padding,
            "Tool Options".to_string(),
        );
        if let Some(rect) = panel.size_slider {
            labels.text(
                list,
                cx,
                rect.origin.y - gap - labels.line,
                label.to_string(),
            );
            let value = editor.tool_size() as f32 / MAX_TOOL_SIZE as f32;
            Slider::new(rect, value).draw(list, tokens, scale);
        }
    }
    if let Some(rect) = panel.arrow_style {
        let style = match editor.config().tools.arrow.style {
            ArrowStyle::Straight => "Straight",
            ArrowStyle::Curved => "Curved",
        };
        labels.row_text(list, rect, format!("Style: {style}"));
    }
    if let Some(rect) = panel.arrow_reverse {
        toggle_row(
            list,
            tokens,
            scale,
            rect,
            labels,
            "Reverse",
            editor.config().tools.arrow.reverse,
        );
    }
    if let Some(rect) = panel.counter_outline {
        toggle_row(
            list,
            tokens,
            scale,
            rect,
            labels,
            "Outline",
            editor.config().tools.counter.outline,
        );
    }
    if let Some(rect) = panel.pixelate_mode {
        let mode = if kind == Some(ToolKind::Blur) {
            "Blur"
        } else {
            "Pixelate"
        };
        labels.row_text(list, rect, format!("Mode: {mode}"));
    }
}

/// The layers section: header with the raise/lower chevron buttons on the
/// right, then one row per object (bottom-to-top, row index == z).
fn draw_layers(
    list: &mut DisplayList,
    editor: &EditorState,
    tokens: &DesignTokens,
    scale: f32,
    atlas: TextureId,
    panel: &SidePanelLayout,
    labels: &Labels,
) {
    let accent = Color::from_hex_token(&tokens.palette.accent)
        .unwrap_or(Color::from_rgba8(255, 0, 255, 255));
    let gap = tokens.spacing.small as f32 * scale;
    let padding = tokens.spacing.medium as f32 * scale;
    if let Some(raise) = panel.raise_button {
        let z_header = labels.line.max(z_button_edge(tokens) * scale);
        let header_y = raise.origin.y - (z_header - z_button_edge(tokens) * scale) / 2.0;
        labels.text(
            list,
            panel.rect.origin.x + padding,
            header_y + (z_header - labels.line) / 2.0,
            "Layers".to_string(),
        );
        IconButton::new(raise, Icon::ChevronUp).draw(list, tokens, scale, atlas);
        if let Some(lower) = panel.lower_button {
            IconButton::new(lower, Icon::ChevronDown).draw(list, tokens, scale, atlas);
        }
    }
    let icon_edge = layer_icon_edge(tokens) * scale;
    for (layer, (_, row)) in editor.layers().iter().zip(&panel.layer_rows) {
        if editor.selected_object() == Some(layer.id) {
            list.fill(
                Shape::Rect {
                    rect: *row,
                    radius: tokens.radii.small as f32 * scale,
                },
                accent,
            );
        }
        let src = layer_icon(layer.kind).rect();
        let dst = Rect::from_parts(
            row.origin.x + gap,
            row.origin.y + (row_height(tokens) * scale - icon_edge) / 2.0,
            icon_edge,
            icon_edge,
        );
        list.image(
            atlas,
            dst,
            Some(Rect::from_parts(src[0], src[1], src[2], src[3])),
        );
        labels.text(
            list,
            row.origin.x + gap * 2.0 + icon_edge,
            row.origin.y + (row_height(tokens) * scale - labels.line) / 2.0,
            layer.kind.to_string(),
        );
    }
}

/// One label-left / toggle-right row (the hit region is the full row - the
/// layout rect - while the toggle paints at its right end).
fn toggle_row(
    list: &mut DisplayList,
    tokens: &DesignTokens,
    scale: f32,
    rect: Rect,
    labels: &Labels,
    label: &str,
    checked: bool,
) {
    labels.row_text(list, rect, label.to_string());
    let track_width = toggle_track_width(tokens) * scale;
    let track_height = toggle_row_height(tokens) * scale;
    let track = Rect::from_parts(
        rect.origin.x + rect.size.width - track_width,
        rect.origin.y + (rect.size.height - track_height) / 2.0,
        track_width,
        track_height,
    );
    Toggle::new(track, checked).draw(list, tokens, scale);
}

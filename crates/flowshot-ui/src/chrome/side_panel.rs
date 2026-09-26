//! The side panel (plan todo 26): per-tool options + the layer list.
//!
//! Toggle with Space (the funnel's chrome key seam), gated by the
//! `[editor].side_panel` config key; Esc cascade stage 3 hides it (the
//! todo-16 cascade seam). The panel anchors to the right of the selection
//! (flipping to the left near the output edge) and repositions live because
//! the layout is recomputed from the selection rect every frame.
//!
//! Per-tool controls gate on the ACTIVE TOOL ([`size_label`] is the
//! visibility table): one size slider bound to the todo-20 dispatched size
//! slot (thickness / corner radius / font size / marker-block-counter size),
//! arrow style + reverse, the counter outline toggle, and the pixelate/blur
//! mode swap (the [`ToolKind::Blur`] panel exposure). NO insecure-pixelate
//! toggle exists anywhere - the mode was dropped per Amendment #3.
//!
//! The layer list (todo-25 model: [`EditorState::layers`], bottom-to-top)
//! gives every object a row - row index == paint z - with its type icon;
//! click = select, press-drag-release onto another row = reorder
//! ([`EditorState::move_layer`], ONE undo unit), and the chevron buttons
//! raise/lower the selection ([`EditorState::raise_selected`] /
//! [`EditorState::lower_selected`]).

use crate::editor::ToolKind;
use crate::editor::paint::local_rect;
use crate::render::Rect;
use flowshot_core::geometry::{LogicalRect, OutputInfo};
use flowshot_core::tokens::DesignTokens;

use crate::editor::EditorState;

pub(super) mod paint;

/// The panel width (logical px).
pub const PANEL_WIDTH: f32 = 200.0;
/// The slider / layer-row height (logical px).
pub(super) const ROW: f32 = 24.0;
/// The toggle-row height (logical px).
pub(super) const TOGGLE_ROW: f32 = 20.0;
/// The raise/lower button edge (logical px).
pub(super) const Z_BUTTON: f32 = 20.0;
/// The layer-row icon edge (logical px).
pub(super) const LAYER_ICON: f32 = 16.0;

/// The per-tool size-control label - the visibility gate of the plan's
/// "per-tool options (size sliders ...)": `None` hides the slider (the
/// selection/move/invert kinds have no size semantics; invert is a region
/// effect). Rectangle's dispatched slot IS the corner radius (the F27
/// `drawRectangleSize` naming, Amendment #3), text's IS the font size.
#[must_use]
pub fn size_label(kind: ToolKind) -> Option<&'static str> {
    match kind {
        ToolKind::Pencil | ToolKind::Line | ToolKind::Arrow | ToolKind::Circle => Some("Thickness"),
        ToolKind::Rectangle => Some("Corner radius"),
        ToolKind::Marker => Some("Marker size"),
        ToolKind::Text => Some("Font size"),
        ToolKind::Pixelate | ToolKind::Blur => Some("Block size"),
        ToolKind::Counter => Some("Counter size"),
        ToolKind::Selection | ToolKind::Move | ToolKind::Invert => None,
    }
}

/// The side panel layout: the bounding rect plus every control hit region
/// (`None` = the control is hidden for the active tool).
#[derive(Debug, Default)]
pub struct SidePanelLayout {
    /// The bounding rectangle.
    pub rect: Rect,
    /// The size slider row (the active tool's dispatched size slot).
    pub size_slider: Option<Rect>,
    /// The arrow style row (cycles straight/curved; Arrow only).
    pub arrow_style: Option<Rect>,
    /// The arrow reverse row (Arrow only).
    pub arrow_reverse: Option<Rect>,
    /// The counter outline row (Counter only).
    pub counter_outline: Option<Rect>,
    /// The pixelate/blur mode row (Pixelate and Blur only).
    pub pixelate_mode: Option<Rect>,
    /// The raise-selected button.
    pub raise_button: Option<Rect>,
    /// The lower-selected button.
    pub lower_button: Option<Rect>,
    /// The layer rows as (object id, rect), BOTTOM-TO-TOP (the plan's list
    /// order: row index == paint z, so a drop row maps straight onto
    /// [`EditorState::move_layer`]).
    pub layer_rows: Vec<(usize, Rect)>,
}

/// Computes the panel layout for `selection` on `output` (physical px).
#[must_use]
pub fn layout(
    editor: &EditorState,
    selection: LogicalRect,
    tokens: &DesignTokens,
    scale: f32,
    output: &OutputInfo,
) -> SidePanelLayout {
    let local_sel = local_rect(output, selection);
    let padding = tokens.spacing.medium as f32 * scale;
    let gap = tokens.spacing.small as f32 * scale;
    let width = PANEL_WIDTH * scale;
    let line = tokens.typography.base_size as f32 * scale * 1.2;
    let row = ROW * scale;
    let toggle_row = TOGGLE_ROW * scale;

    let kind = editor.active_tool();
    let sized = kind.and_then(size_label).is_some();
    let arrow = kind == Some(ToolKind::Arrow);
    let counter = kind == Some(ToolKind::Counter);
    let pixelate = matches!(kind, Some(ToolKind::Pixelate | ToolKind::Blur));
    let tool_section = sized || arrow || counter || pixelate;
    let layers = editor.layers();

    // Dynamic height: padding + the tool section (when any) + the layers
    // section (header + one row per object).
    let mut height = padding * 2.0;
    if tool_section {
        height += line + gap;
        if sized {
            height += line + gap + row + gap;
        }
        if arrow {
            height += (toggle_row + gap) * 2.0;
        }
        if counter || pixelate {
            height += toggle_row + gap;
        }
        height += gap;
    }
    let z_header = line.max(Z_BUTTON * scale);
    height += z_header + gap + layers.len() as f32 * (row + gap);

    // Anchor right of the selection; flip left near the output edge; the
    // panel stays fully on-screen (min/max pair - never clamp on possibly
    // inverted bounds).
    let margin = padding;
    let edge_x = output.physical_size.width.0 as f32 - margin;
    let mut x = local_sel.origin.x + local_sel.size.width + margin;
    if x + width > edge_x {
        x = (local_sel.origin.x - width - margin).max(margin);
    }
    let edge_y = output.physical_size.height.0 as f32 - margin;
    let mut y = local_sel.origin.y;
    if y + height > edge_y {
        y = (edge_y - height).max(margin);
    }

    let rect = Rect::from_parts(x, y, width, height);
    let cx = x + padding;
    let content = width - padding * 2.0;
    let mut cy = y + padding;
    let mut layout = SidePanelLayout {
        rect,
        layer_rows: Vec::with_capacity(layers.len()),
        ..SidePanelLayout::default()
    };

    if tool_section {
        cy += line + gap;
        if sized {
            cy += line + gap;
            layout.size_slider = Some(Rect::from_parts(cx, cy, content, row));
            cy += row + gap;
        }
        if arrow {
            layout.arrow_style = Some(Rect::from_parts(cx, cy, content, toggle_row));
            cy += toggle_row + gap;
            layout.arrow_reverse = Some(Rect::from_parts(cx, cy, content, toggle_row));
            cy += toggle_row + gap;
        }
        if counter {
            layout.counter_outline = Some(Rect::from_parts(cx, cy, content, toggle_row));
            cy += toggle_row + gap;
        }
        if pixelate {
            layout.pixelate_mode = Some(Rect::from_parts(cx, cy, content, toggle_row));
            cy += toggle_row + gap;
        }
        cy += gap;
    }

    let z_edge = Z_BUTTON * scale;
    layout.raise_button = Some(Rect::from_parts(
        cx + content - z_edge * 2.0 - gap,
        cy + (z_header - z_edge) / 2.0,
        z_edge,
        z_edge,
    ));
    layout.lower_button = Some(Rect::from_parts(
        cx + content - z_edge,
        cy + (z_header - z_edge) / 2.0,
        z_edge,
        z_edge,
    ));
    cy += z_header + gap;

    for layer in &layers {
        layout
            .layer_rows
            .push((layer.id, Rect::from_parts(cx, cy, content, row)));
        cy += row + gap;
    }
    layout
}

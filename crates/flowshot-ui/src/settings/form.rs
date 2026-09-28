//! The token-driven form primitives shared by EVERY egui panel: section
//! cards, the two-column row grid, sub-sections, pill tabs, the primary
//! button, hints, and the checkbox widget (the settings tabs and the
//! launcher dialog build on the same vocabulary).
//!
//! Every visual value comes from the projected style ([`FormMetrics`] for
//! geometry, `ui.visuals()` slots for color - the theme stores the card
//! surface in `faint_bg_color`, the field surface in `extreme_bg_color`,
//! the accent in `selection.stroke`, and the on-accent ink in
//! `widgets.active.fg_stroke`), so the Interface tab's live accent/contrast
//! edits re-theme these primitives on the next frame with no plumbing.

use egui::{
    Align, Button, Color32, Frame, Layout, Margin, Response, RichText, Rounding, Sense, Shape,
    Stroke, Ui, Widget, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::egui_host::theme::{self, ink_on};
use crate::settings::layout::FormMetrics;

/// The focus-ring offset around a focused checkbox.
const FOCUS_RING_OFFSET: f32 = 2.0;
/// The check-mark stroke width as a ratio of the box edge.
const CHECK_STROKE_RATIO: f32 = 0.16;
/// The checkbox hit target as a multiple of the box edge (the visual box
/// stays token-sized; the click area is generously wider).
const HIT_WIDTH: f32 = 2.0;

/// Capitalizes the first character for id display ("open-app" ->
/// "Open-app"): the shared row-label projection of the toolbar and
/// shortcut vocabularies.
pub(crate) fn title_case(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A section card: the raised token surface, a semibold title, and a 1px
/// separator under it, laid out as a CENTERED content column (capped at
/// [`FormMetrics::content_width`]) so wide windows keep a readable measure.
/// Returns the body's change flag.
pub(crate) fn card(
    ui: &mut Ui,
    m: &FormMetrics,
    title: &str,
    body: impl FnOnce(&mut Ui) -> bool,
) -> bool {
    let available = ui.available_width();
    let width = m.content_width(available);
    let inner = width - 2.0 * m.card_padding();
    let offset = m.content_offset(available);
    let card_fill = ui.visuals().faint_bg_color;
    let title_color = ui.visuals().strong_text_color();
    let title_font = theme::semibold(m.title_size());
    // The centering lives in the outer margin: Frame::show inherits the
    // enclosing layout, so a wrapping horizontal would flow the body rows
    // left-to-right instead of stacking them.
    Frame::none()
        .fill(card_fill)
        .rounding(Rounding::same(m.card_radius()))
        .inner_margin(Margin::same(m.card_padding()))
        .outer_margin(Margin {
            left: offset,
            right: offset,
            top: 0.0,
            bottom: m.medium(),
        })
        .show(ui, |ui| {
            ui.set_min_width(inner);
            ui.set_max_width(inner);
            ui.label(RichText::new(title).font(title_font).color(title_color));
            ui.add_space(m.small());
            ui.separator();
            ui.add_space(m.medium());
            body(ui)
        })
        .inner
}

/// One grid row: the label right-aligned in the fixed-width label column,
/// the control in the column at [`FormMetrics::control_x`]. The row height
/// is the control height, growing only when a narrow window wraps the
/// label (clipping a wrapped label would break the grid's contract).
/// Returns whatever the control closure produced.
pub(crate) fn row<R>(
    ui: &mut Ui,
    m: &FormMetrics,
    label: &str,
    control: impl FnOnce(&mut Ui) -> R,
) -> R {
    let available = ui.available_width();
    let label_width = m.label_width(available);
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Wrap),
        label_width,
        egui::TextStyle::Body,
    );
    let height = m.control_height().max(galley.size().y);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            vec2(label_width, height),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(galley);
            },
        );
        let control_width = ui.available_width().max(m.base_size());
        ui.allocate_ui_with_layout(
            vec2(control_width, height),
            Layout::left_to_right(Align::Center),
            control,
        )
        .inner
    })
    .inner
}

/// A titled sub-section inside a card (the `[tools.*]` groups, the save
/// action set): a semibold weak title aligned to the CONTROL column (it
/// heads the rows below it, not the label column), over the same grid
/// rows. Returns the body's change flag.
pub(crate) fn sub_section(
    ui: &mut Ui,
    m: &FormMetrics,
    title: &str,
    body: impl FnOnce(&mut Ui) -> bool,
) -> bool {
    ui.add_space(m.medium());
    let available = ui.available_width();
    ui.horizontal(|ui| {
        ui.add_space(m.label_width(available) + m.gutter());
        ui.label(
            RichText::new(title)
                .font(theme::semibold(m.base_size()))
                .color(ui.visuals().weak_text_color()),
        );
    });
    ui.add_space(m.small());
    body(ui)
}

/// A pill tab: the selected tab is a solid accent pill with semibold ink,
/// resting tabs take the themed button surface (hover feedback included)
/// with the medium face.
pub(crate) fn pill_tab(ui: &mut Ui, m: &FormMetrics, selected: bool, label: &str) -> Response {
    let accent = ui.visuals().selection.stroke.color;
    let (font, text_color) = if selected {
        (theme::semibold(m.base_size()), ink_on(accent))
    } else {
        (theme::medium(m.base_size()), ui.visuals().text_color())
    };
    let mut button = Button::new(RichText::new(label).font(font).color(text_color))
        .rounding(Rounding::same(m.control_height() * 0.5))
        .min_size(vec2(0.0, m.control_height()));
    if selected {
        button = button.fill(accent);
    }
    ui.add(button)
}

/// The primary action button (Apply / Capture): accent-filled with semibold
/// on-accent ink when enabled; the disabled state takes the neutral button
/// surface and weak ink so it stays legible but clearly inert.
pub(crate) fn primary_button(ui: &mut Ui, m: &FormMetrics, label: &str, enabled: bool) -> Response {
    let accent = ui.visuals().selection.stroke.color;
    let (fill, text_color) = if enabled {
        (accent, ink_on(accent))
    } else {
        (
            ui.visuals().widgets.inactive.weak_bg_fill,
            ui.visuals().weak_text_color(),
        )
    };
    ui.add_enabled(
        enabled,
        Button::new(
            RichText::new(label)
                .font(theme::semibold(m.base_size()))
                .color(text_color),
        )
        .fill(fill)
        .rounding(Rounding::same(m.control_radius()))
        .min_size(vec2(0.0, m.control_height())),
    )
}

/// A weak hint paragraph aligned to the control column (it annotates the
/// field above it, not the whole card).
pub(crate) fn hint(ui: &mut Ui, m: &FormMetrics, text: &str) {
    note(ui, m, text, ui.visuals().weak_text_color());
}

/// A validation-error paragraph aligned to the control column (the
/// launcher's inline geometry issue).
pub(crate) fn error_hint(ui: &mut Ui, m: &FormMetrics, text: &str) {
    note(ui, m, text, ui.visuals().error_fg_color);
}

fn note(ui: &mut Ui, m: &FormMetrics, text: &str, color: Color32) {
    let available = ui.available_width();
    ui.horizontal(|ui| {
        ui.add_space(m.label_width(available) + m.gutter());
        ui.colored_label(color, text);
    });
}

/// A read-only well displaying computed output (the filename preview):
/// monospace text on the sunken field surface with the control outline.
pub(crate) fn readout(ui: &mut Ui, m: &FormMetrics, text: &str) {
    let font = egui::FontId::monospace(m.base_size() - 1.0);
    let color = ui.visuals().text_color();
    let text_width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.into(), font.clone(), color)
            .size()
            .x
    });
    let size = vec2(text_width + 2.0 * m.medium(), m.control_height());
    let (rect, _response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        Rounding::same(m.control_radius()),
        ui.visuals().extreme_bg_color,
        ui.visuals().widgets.inactive.bg_stroke,
    );
    painter.text(
        rect.left_center() + vec2(m.medium(), 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        color,
    );
}

/// The settings checkbox: a real token-sized box - raised control surface
/// with an outline at rest, accent fill with an on-accent check mark when
/// on, the accent edge on hover, and the selection-stroke focus ring.
/// Click semantics match `egui::Checkbox` exactly (toggle + `changed()`,
/// keyboard Space/Enter via the focused click sense, a11y widget info).
pub(crate) struct TokenCheckbox<'a> {
    value: &'a mut bool,
}

impl<'a> TokenCheckbox<'a> {
    pub(crate) fn new(value: &'a mut bool) -> Self {
        Self { value }
    }
}

impl Widget for TokenCheckbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let box_edge = ui.spacing().icon_width;
        let height = ui.spacing().interact_size.y.max(box_edge);
        let (rect, mut response) =
            ui.allocate_exact_size(vec2(box_edge * HIT_WIDTH, height), Sense::click());
        if response.clicked() {
            *self.value = !*self.value;
            response.mark_changed();
        }
        let checked = *self.value;
        let enabled = ui.is_enabled();
        response
            .widget_info(move || WidgetInfo::selected(WidgetType::Checkbox, enabled, checked, ""));
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&response);
            let (small, big) = ui.spacing().icon_rectangles(rect);
            let painter = ui.painter();
            if checked {
                // A disabled checked box reads inert (weak ink on the
                // neutral surface), never a live accent.
                let on = if enabled {
                    ui.visuals().widgets.active
                } else {
                    ui.visuals().widgets.noninteractive
                };
                painter.rect(
                    big.expand(visuals.expansion),
                    visuals.rounding,
                    on.bg_fill,
                    on.bg_stroke,
                );
                painter.add(Shape::line(
                    vec![
                        pos2(small.left(), small.center().y),
                        pos2(small.center().x, small.bottom()),
                        pos2(small.right(), small.top()),
                    ],
                    Stroke::new(box_edge * CHECK_STROKE_RATIO, on.fg_stroke.color),
                ));
            } else {
                painter.rect(
                    big.expand(visuals.expansion),
                    visuals.rounding,
                    visuals.bg_fill,
                    visuals.bg_stroke,
                );
            }
            if response.has_focus() {
                painter.rect(
                    big.expand(FOCUS_RING_OFFSET),
                    visuals.rounding,
                    Color32::TRANSPARENT,
                    ui.visuals().selection.stroke,
                );
            }
        }
        response
    }
}

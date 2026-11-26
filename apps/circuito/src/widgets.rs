// SPDX-License-Identifier: MIT OR Apache-2.0
//! Small pieces drawn with the NDL palette: icons, icon buttons and the text buttons of dialogs.

use egui::{vec2, Color32, CornerRadius, Response, Sense, Stroke, Ui};
use nodalia_theme::engineering::Look;
use nodalia_theme::for_egui::color;
use nodalia_theme::icons;

/// The creator suite's look for the same palette: About and dialog headings are drawn by it.
pub fn creator(look: &Look) -> nodalia_theme::creator::Look {
    use nodalia_theme::engineering::{BOLD, MEDIUM, SEMIBOLD};
    let _ = BOLD;
    nodalia_theme::creator::Look::new(look.palette)
        .strong_family(egui::FontFamily::Name(SEMIBOLD.into()))
        .medium_family(egui::FontFamily::Name(MEDIUM.into()))
}

/// Paint an NDL icon in a square of `size` points.
pub fn icon(ui: &mut Ui, name: &str, size: f32, ink: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if let Some(i) = icons::get(name) {
        icons::paint(ui.painter(), rect, i, icons::Style::bold(ink));
    }
    response
}

/// A borderless icon button with an accessible label and a tooltip.
pub fn icon_button(
    ui: &mut Ui,
    look: &Look,
    name: &str,
    label: &str,
    size: f32,
    active: bool,
) -> Response {
    let p = &look.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let enabled = ui.is_enabled();
    let fill = if active {
        color(p.accent_subtle)
    } else if response.hovered() && enabled {
        color(p.control_bg_hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            CornerRadius::same(8),
            Stroke::new(2.0, color(p.border_focus)),
            egui::StrokeKind::Outside,
        );
    }
    let ink = if !enabled {
        color(p.text_disabled)
    } else if active {
        color(p.accent_text)
    } else {
        color(p.text_secondary)
    };
    if let Some(i) = icons::get(name) {
        let inset = (rect.width() * 0.24).round();
        icons::paint(ui.painter(), rect.shrink(inset), i, icons::Style::bold(ink));
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response.on_hover_text(label)
}

/// The frame of a text field: the field colour, a hairline and rounded corners.
pub fn field(look: &Look) -> egui::Frame {
    let p = &look.palette;
    egui::Frame::NONE
        .fill(color(p.control_field_bg))
        .stroke(Stroke::new(1.0, color(p.border_default)))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(egui::Margin::symmetric(8, 4))
}

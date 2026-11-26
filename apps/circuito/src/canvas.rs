// SPDX-License-Identifier: MIT OR Apache-2.0
//! The schematic canvas: the sheet drawn from the library's symbols, panned with a drag and
//! zoomed with the wheel or a pinch. A click selects the part under it; what the rules check
//! points at is ringed in the danger colour. Nothing here edits the drawing.

use circuito_app::commands::Cmd;
use circuito_app::geometry::{self, Selection, View};
use circuito_app::i18n::tr;
use circuito_app::workspace::Open;
use circuito_core::library::{Library, PartDef, Shape};
use circuito_core::schematic::{LabelScope, Part, PowerStyle, Sheet};
use egui::{
    pos2, vec2, Align2, Color32, FontFamily, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Ui,
    UiBuilder,
};
use nodalia_theme::engineering::{self as eng, Look, ViewItem};
use nodalia_theme::for_egui::color;

/// What the person did on the canvas this frame.
#[derive(Default)]
pub struct Out {
    /// A click: the part under it, or nothing.
    pub clicked: Option<Option<Selection>>,
    /// Where the pointer is, in grid units.
    pub hover: Option<(f32, f32)>,
    pub commands: Vec<Cmd>,
}

struct Ink {
    part: Color32,
    wire: Color32,
    label: Color32,
    power: Color32,
    refdes: Color32,
    value: Color32,
    mark: Color32,
}

struct Frame {
    origin: Pos2,
    size: (f32, f32),
    view: View,
}

impl Frame {
    fn at(&self, p: (f32, f32)) -> Pos2 {
        let s = self.view.to_screen(p, self.size);
        pos2(self.origin.x + s.0, self.origin.y + s.1)
    }
}

pub fn show(ui: &mut Ui, look: &Look, doc: &mut Open, grid: bool) -> Out {
    let p = &look.palette;
    let mut out = Out::default();
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, color(p.surface_sunken));
    let size = (rect.width().max(1.0), rect.height().max(1.0));

    let Open {
        circuit,
        sheet,
        view,
        fit_pending,
        selection,
        finding,
        ..
    } = doc;
    let Some(schematic) = circuit.schematic.as_ref() else {
        return out;
    };
    let Some(sheet_data) = schematic.sheets.get(*sheet) else {
        return out;
    };

    if *fit_pending {
        if let Some(bounds) = geometry::sheet_bounds(sheet_data) {
            *view = View::fit(bounds, size, 56.0);
        }
        *fit_pending = false;
    }

    // Pan and zoom first, so what is drawn is where the pointer now is.
    if response.dragged_by(PointerButton::Primary) || response.dragged_by(PointerButton::Middle) {
        let d = response.drag_delta();
        *view = view.panned((d.x, d.y));
    }
    if response.hovered() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = pinch * (scroll * 0.0015).exp();
        if (factor - 1.0).abs() > 1e-4 {
            if let Some(at) = response.hover_pos() {
                *view = view.zoomed_at(factor, (at.x - rect.left(), at.y - rect.top()), size);
            }
        }
    }
    let frame = Frame {
        origin: rect.min,
        size,
        view: *view,
    };
    if let Some(at) = response.hover_pos() {
        out.hover = Some(
            frame
                .view
                .to_world((at.x - rect.left(), at.y - rect.top()), size),
        );
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    if response.clicked() {
        if let Some(at) = response.interact_pointer_pos() {
            let world = frame
                .view
                .to_world((at.x - rect.left(), at.y - rect.top()), size);
            let hit = geometry::hit_part(sheet_data, world, 0.4);
            out.clicked = Some(hit.map(|part| Selection::Part {
                sheet: *sheet,
                part,
            }));
        }
    }

    if grid {
        draw_grid(&painter, rect, &frame, color(p.border_subtle));
    }
    let ink = Ink {
        part: color(p.text_primary),
        wire: color(p.status_success_fg),
        label: color(p.status_info_fg),
        power: color(p.status_warning_fg),
        refdes: color(p.accent_text),
        value: color(p.text_secondary),
        mark: color(p.status_danger_fg),
    };
    draw_selection(
        &painter,
        &frame,
        schematic.sheets.get(*sheet),
        *selection,
        finding.is_some(),
        look,
    );
    draw_sheet(&painter, &frame, sheet_data, &ink);

    if geometry::sheet_bounds(sheet_data).is_none() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr("This sheet is empty."),
            eng::body(),
            color(p.text_secondary),
        );
    }

    // The view tools float over the drawing.
    let bar = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.bottom() - 48.0),
        vec2(180.0, 38.0),
    );
    let zoom_out = tr("Zoom out");
    let fit = tr("Fit to window");
    let zoom_in = tr("Zoom in");
    let grid_name = tr("Show the grid");
    let items = [
        ViewItem::Tool {
            icon: "zoom-out",
            name: zoom_out,
            active: false,
        },
        ViewItem::Tool {
            icon: "fit",
            name: fit,
            active: false,
        },
        ViewItem::Tool {
            icon: "zoom",
            name: zoom_in,
            active: false,
        },
        ViewItem::Separator,
        ViewItem::Tool {
            icon: "grid",
            name: grid_name,
            active: grid,
        },
    ];
    let picked = ui
        .scope_builder(UiBuilder::new().max_rect(bar), |ui| {
            eng::view_bar(ui, look, &items)
        })
        .inner;
    match picked {
        Some(0) => out.commands.push(Cmd::ZoomOut),
        Some(1) => out.commands.push(Cmd::FitView),
        Some(2) => out.commands.push(Cmd::ZoomIn),
        Some(4) => out.commands.push(Cmd::ToggleGrid),
        _ => {}
    }
    out
}

fn draw_grid(painter: &egui::Painter, rect: Rect, frame: &Frame, ink: Color32) {
    // Dots a grid unit apart, thinned out until they are at least 14 points apart.
    let mut step = 1.0_f32;
    while step * frame.view.zoom < 14.0 && step < 1000.0 {
        step *= if step < 5.0 { 5.0 } else { 2.0 };
    }
    let (w0, w1) = (
        frame.view.to_world((0.0, 0.0), frame.size),
        frame.view.to_world(frame.size, frame.size),
    );
    let (x0, x1) = ((w0.0 / step).floor() as i32, (w1.0 / step).ceil() as i32);
    let (y0, y1) = ((w0.1 / step).floor() as i32, (w1.1 / step).ceil() as i32);
    // A zoomed-out view of a huge sheet would otherwise draw millions of dots.
    if (x1 - x0).saturating_mul(y1 - y0) > 40_000 {
        return;
    }
    let half = 0.9;
    for gx in x0..=x1 {
        for gy in y0..=y1 {
            let at = frame.at((gx as f32 * step, gy as f32 * step));
            if rect.contains(at) {
                painter.rect_filled(
                    Rect::from_center_size(at, vec2(half * 2.0, half * 2.0)),
                    0.0,
                    ink,
                );
            }
        }
    }
}

fn draw_selection(
    painter: &egui::Painter,
    frame: &Frame,
    sheet: Option<&Sheet>,
    selection: Option<Selection>,
    from_finding: bool,
    look: &Look,
) {
    let p = &look.palette;
    let (stroke, fill) = if from_finding {
        (color(p.status_danger_fg), color(p.status_danger_bg))
    } else {
        (color(p.accent_default), color(p.selection_bg))
    };
    match selection {
        Some(Selection::Part { part, .. }) => {
            let Some(b) = sheet
                .and_then(|s| s.parts.get(part))
                .and_then(geometry::part_bounds)
            else {
                return;
            };
            let b = b.expanded(0.8);
            let r = Rect::from_two_pos(frame.at(b.min), frame.at(b.max));
            painter.rect_filled(r, 6.0, fill);
            painter.rect_stroke(r, 6.0, Stroke::new(1.5, stroke), egui::StrokeKind::Inside);
        }
        Some(Selection::Spot { x, y, .. }) => {
            let at = frame.at((x as f32, y as f32));
            let r = (1.6 * frame.view.zoom).max(8.0);
            painter.circle_filled(at, r, fill);
            painter.circle_stroke(at, r, Stroke::new(1.5, stroke));
        }
        None => {}
    }
}

fn draw_sheet(painter: &egui::Painter, frame: &Frame, sheet: &Sheet, ink: &Ink) {
    let zoom = frame.view.zoom;
    let line = (zoom * 0.13).clamp(1.1, 2.4);
    let text = zoom >= 4.0;
    let font = FontId::new((zoom * 0.8).clamp(8.0, 15.0), FontFamily::Monospace);
    let pt = |x: i32, y: i32| frame.at((x as f32, y as f32));

    for wire in &sheet.wires {
        let points: Vec<Pos2> = wire.points.iter().map(|q| pt(q.x, q.y)).collect();
        if points.len() >= 2 {
            painter.add(egui::Shape::line(points, Stroke::new(line, ink.wire)));
        }
    }
    for part in &sheet.parts {
        if let Some(def) = Library::builtin().get(&part.lib_id) {
            draw_part(painter, frame, part, def, ink, line, text, &font);
        } else {
            // A symbol the library does not have: a marked box, so it is not silently missing.
            let at = pt(part.position.x, part.position.y);
            let r = Rect::from_center_size(at, vec2(zoom * 4.0, zoom * 2.0));
            painter.rect_stroke(
                r,
                2.0,
                Stroke::new(line, ink.mark),
                egui::StrokeKind::Inside,
            );
            if text {
                painter.text(
                    at,
                    Align2::CENTER_CENTER,
                    &part.refdes,
                    font.clone(),
                    ink.mark,
                );
            }
        }
    }
    for j in &sheet.junctions {
        painter.circle_filled(
            pt(j.position.x, j.position.y),
            (zoom * 0.2).clamp(2.0, 5.0),
            ink.wire,
        );
    }
    for n in &sheet.no_connects {
        let at = pt(n.position.x, n.position.y);
        let d = (zoom * 0.45).clamp(3.0, 9.0);
        let stroke = Stroke::new(line, ink.mark);
        painter.line_segment([at + vec2(-d, -d), at + vec2(d, d)], stroke);
        painter.line_segment([at + vec2(-d, d), at + vec2(d, -d)], stroke);
    }
    for port in &sheet.power_ports {
        draw_power(
            painter,
            frame,
            port.position.x,
            port.position.y,
            port.style,
            &port.name,
            ink,
            line,
            text,
            &font,
        );
    }
    for label in &sheet.labels {
        let at = pt(label.position.x, label.position.y);
        let galley = painter.layout_no_wrap(label.name.clone(), font.clone(), ink.label);
        let w = galley.size().x;
        let inset = zoom * 0.4;
        if text {
            painter.galley(at + vec2(inset, -galley.size().y - 1.0), galley, ink.label);
        }
        let stroke = Stroke::new(line, ink.label);
        let end = if text {
            at.x + inset + w
        } else {
            at.x + zoom * 3.0
        };
        painter.line_segment([at, pos2(end, at.y)], stroke);
        if label.scope == LabelScope::Global {
            // A global label wears a small flag at the anchor.
            painter.circle_filled(at, (zoom * 0.22).clamp(2.0, 5.0), ink.label);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_part(
    painter: &egui::Painter,
    frame: &Frame,
    part: &Part,
    def: &PartDef,
    ink: &Ink,
    line: f32,
    text: bool,
    font: &FontId,
) {
    let stroke = Stroke::new(line, ink.part);
    let at = |q: circuito_core::schematic::Point| frame.at(geometry::place(part, q));
    for shape in &def.shapes {
        match shape {
            Shape::Polyline {
                points,
                closed,
                filled,
            } => {
                let pts: Vec<Pos2> = points.iter().map(|q| at(*q)).collect();
                if pts.len() < 2 {
                    continue;
                }
                if *closed && *filled {
                    painter.add(egui::Shape::convex_polygon(pts, ink.part, stroke));
                } else if *closed {
                    painter.add(egui::Shape::closed_line(pts, stroke));
                } else {
                    painter.add(egui::Shape::line(pts, stroke));
                }
            }
            Shape::Rect { min, max } => {
                let corners = vec![
                    at(*min),
                    at(circuito_core::schematic::Point::new(max.x, min.y)),
                    at(*max),
                    at(circuito_core::schematic::Point::new(min.x, max.y)),
                ];
                painter.add(egui::Shape::closed_line(corners, stroke));
            }
            Shape::Circle { center, radius } => {
                painter.circle_stroke(at(*center), *radius as f32 * frame.view.zoom, stroke);
            }
            Shape::Arc {
                center,
                radius,
                start_deg,
                end_deg,
            } => {
                let (cx, cy) = (center.x as f32, center.y as f32);
                let r = *radius as f32;
                let start = *start_deg as f32;
                let mut end = *end_deg as f32;
                if end < start {
                    end += 360.0;
                }
                let steps = (((end - start) / 8.0).ceil() as usize).clamp(2, 90);
                let pts: Vec<Pos2> = (0..=steps)
                    .map(|i| {
                        let a = (start + (end - start) * i as f32 / steps as f32).to_radians();
                        frame.at(geometry::place_f(
                            part,
                            (cx + r * a.cos(), cy - r * a.sin()),
                        ))
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
    }
    if !text {
        return;
    }
    let Some(b) = geometry::part_bounds(part) else {
        return;
    };
    let (cx, cy) = b.centre();
    let pad = 0.9;
    if b.width() >= b.height() {
        painter.text(
            frame.at((cx, b.min.1 - pad)),
            Align2::CENTER_BOTTOM,
            &part.refdes,
            font.clone(),
            ink.refdes,
        );
        painter.text(
            frame.at((cx, b.max.1 + pad)),
            Align2::CENTER_TOP,
            &part.value,
            font.clone(),
            ink.value,
        );
    } else {
        painter.text(
            frame.at((b.max.0 + pad, cy - 0.2)),
            Align2::LEFT_BOTTOM,
            &part.refdes,
            font.clone(),
            ink.refdes,
        );
        painter.text(
            frame.at((b.max.0 + pad, cy + 0.2)),
            Align2::LEFT_TOP,
            &part.value,
            font.clone(),
            ink.value,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_power(
    painter: &egui::Painter,
    frame: &Frame,
    x: i32,
    y: i32,
    style: PowerStyle,
    name: &str,
    ink: &Ink,
    line: f32,
    text: bool,
    font: &FontId,
) {
    let z = frame.view.zoom;
    let at = frame.at((x as f32, y as f32));
    let stroke = Stroke::new(line, ink.power);
    match style {
        PowerStyle::Ground => {
            let bottom = at + vec2(0.0, z * 1.0);
            painter.line_segment([at, bottom], stroke);
            for (i, half) in [0.9_f32, 0.55, 0.2].into_iter().enumerate() {
                let yy = bottom.y + i as f32 * z * 0.4;
                painter.line_segment(
                    [pos2(at.x - half * z, yy), pos2(at.x + half * z, yy)],
                    stroke,
                );
            }
        }
        PowerStyle::Supply => {
            let top = at - vec2(0.0, z * 1.2);
            painter.line_segment([at, top], stroke);
            painter.line_segment([top - vec2(z * 0.9, 0.0), top + vec2(z * 0.9, 0.0)], stroke);
            if text {
                painter.text(
                    top - vec2(0.0, z * 0.3),
                    Align2::CENTER_BOTTOM,
                    name,
                    font.clone(),
                    ink.power,
                );
            }
        }
    }
}

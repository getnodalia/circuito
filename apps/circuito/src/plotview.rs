// SPDX-License-Identifier: MIT OR Apache-2.0
//! Plots of a result: one panel per quantity, traces in the palette's colours, engineering units
//! on the axes (a logarithmic frequency axis for `.ac`) and a crosshair that reads the values off
//! under the pointer.

use circuito_app::format::{axis_label, eng, linear_ticks, log_ticks};
use circuito_app::i18n::tr;
use circuito_app::plot::{self, Panel, Plot};
use egui::{pos2, vec2, Align2, Color32, FontFamily, FontId, Pos2, Rect, Sense, Stroke, Ui};
use nodalia_theme::engineering::{self as eng_ui, Look};
use nodalia_theme::for_egui::color;

/// The colour of the signal in `slot`: it keeps it whatever else is shown.
pub fn series_color(look: &Look, slot: usize) -> Color32 {
    let p = &look.palette;
    let all = [
        p.status_info_fg,
        p.status_danger_fg,
        p.status_success_fg,
        p.accent_default,
        p.status_warning_fg,
        p.text_link,
        p.text_secondary,
    ];
    color(all[slot % all.len()])
}

const LEFT: f32 = 66.0;
const RIGHT: f32 = 16.0;
const TOP: f32 = 24.0;
const X_LABELS: f32 = 22.0;

fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Draw `plot` in the room `ui` has.
pub fn show(ui: &mut Ui, look: &Look, plot: &Plot) {
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    if plot.panels.is_empty() {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr("Nothing to plot: every signal is hidden."),
            eng_ui::body(),
            color(look.palette.text_secondary),
        );
        return;
    }
    let n = plot.panels.len() as f32;
    let each = (rect.height() / n).floor();
    for (i, panel) in plot.panels.iter().enumerate() {
        let last = i + 1 == plot.panels.len();
        let r = Rect::from_min_size(
            pos2(rect.left(), rect.top() + each * i as f32),
            vec2(rect.width(), each),
        );
        draw_panel(ui, look, plot, panel, r, last);
    }
}

struct Axes {
    area: Rect,
    x: (f64, f64),
    y: (f64, f64),
    log: bool,
}

impl Axes {
    fn fx(&self, x: f64) -> f32 {
        let (lo, hi, v) = if self.log {
            (self.x.0.ln(), self.x.1.ln(), x.max(f64::MIN_POSITIVE).ln())
        } else {
            (self.x.0, self.x.1, x)
        };
        let t = if hi > lo { (v - lo) / (hi - lo) } else { 0.5 };
        self.area.left() + t as f32 * self.area.width()
    }

    fn fy(&self, y: f64) -> f32 {
        let t = if self.y.1 > self.y.0 {
            (y - self.y.0) / (self.y.1 - self.y.0)
        } else {
            0.5
        };
        self.area.bottom() - t as f32 * self.area.height()
    }

    fn x_of(&self, px: f32) -> f64 {
        let t = f64::from((px - self.area.left()) / self.area.width().max(1.0));
        if self.log {
            (self.x.0.ln() + t * (self.x.1.ln() - self.x.0.ln())).exp()
        } else {
            self.x.0 + t * (self.x.1 - self.x.0)
        }
    }
}

fn draw_panel(ui: &mut Ui, look: &Look, plot: &Plot, panel: &Panel, bounds: Rect, last: bool) {
    let p = &look.palette;
    let painter = ui.painter();
    let area = Rect::from_min_max(
        pos2(bounds.left() + LEFT, bounds.top() + TOP),
        pos2(
            bounds.right() - RIGHT,
            bounds.bottom() - 6.0 - if last { X_LABELS } else { 0.0 },
        ),
    );
    if area.width() < 40.0 || area.height() < 24.0 {
        return;
    }
    let x = if plot.x_log && panel.x.0 > 0.0 {
        panel.x
    } else if plot.x_log {
        (1.0, 10.0)
    } else {
        panel.x
    };
    let axes = Axes {
        area,
        x,
        y: panel.y,
        log: plot.x_log && panel.x.0 > 0.0,
    };

    // Title and legend.
    let unit = panel.y_unit();
    painter.text(
        pos2(bounds.left() + 6.0, bounds.top() + 4.0),
        Align2::LEFT_TOP,
        format!("{} ({unit})", tr(panel.title())),
        eng_ui::panel_title(),
        color(p.text_primary),
    );
    let mut legend_x = bounds.right() - RIGHT;
    for s in panel.series.iter().rev().take(10) {
        let galley = painter.layout_no_wrap(s.name.clone(), mono(11.0), color(p.text_secondary));
        legend_x -= galley.size().x;
        painter.galley(
            pos2(legend_x, bounds.top() + 6.0),
            galley,
            color(p.text_secondary),
        );
        legend_x -= 6.0;
        let c = series_color(look, s.slot);
        painter.line_segment(
            [
                pos2(legend_x - 14.0, bounds.top() + 13.0),
                pos2(legend_x, bounds.top() + 13.0),
            ],
            Stroke::new(2.4, c),
        );
        legend_x -= 24.0;
    }

    painter.rect_filled(area, 6.0, color(p.surface_panel));
    let grid = Stroke::new(1.0, color(p.border_subtle));
    let minor = Stroke::new(1.0, color(p.border_subtle).gamma_multiply(0.45));
    let label = color(p.text_tertiary);

    // The y axis.
    for v in linear_ticks(panel.y.0, panel.y.1, 4) {
        let yy = axes.fy(v);
        if yy < area.top() - 0.5 || yy > area.bottom() + 0.5 {
            continue;
        }
        painter.line_segment([pos2(area.left(), yy), pos2(area.right(), yy)], grid);
        painter.text(
            pos2(area.left() - 6.0, yy),
            Align2::RIGHT_CENTER,
            eng(v, unit),
            mono(10.5),
            label,
        );
    }
    // The x axis.
    let ticks: Vec<(f64, bool)> = if axes.log {
        log_ticks(axes.x.0, axes.x.1)
            .into_iter()
            .map(|t| (t.value, t.major))
            .collect()
    } else {
        linear_ticks(axes.x.0, axes.x.1, 6)
            .into_iter()
            .map(|v| (v, true))
            .collect()
    };
    for (v, major) in ticks {
        let xx = axes.fx(v);
        if xx < area.left() - 0.5 || xx > area.right() + 0.5 {
            continue;
        }
        painter.line_segment(
            [pos2(xx, area.top()), pos2(xx, area.bottom())],
            if major { grid } else { minor },
        );
        if last && major {
            let text = if plot.x_unit.is_empty() {
                axis_label(v)
            } else {
                eng(v, plot.x_unit)
            };
            painter.text(
                pos2(xx, area.bottom() + 4.0),
                Align2::CENTER_TOP,
                text,
                mono(10.5),
                label,
            );
        }
    }
    painter.rect_stroke(
        area,
        6.0,
        Stroke::new(1.0, color(p.border_default)),
        egui::StrokeKind::Inside,
    );

    // The traces.
    let clip = painter.with_clip_rect(area);
    let budget = (area.width() * 2.0) as usize;
    for s in &panel.series {
        let pts = plot::decimate(&s.points, budget);
        let line: Vec<Pos2> = pts
            .iter()
            .filter(|(a, b)| a.is_finite() && b.is_finite())
            .map(|(a, b)| pos2(axes.fx(*a), axes.fy(*b)))
            .collect();
        if line.len() >= 2 {
            clip.add(egui::Shape::line(
                line,
                Stroke::new(1.8, series_color(look, s.slot)),
            ));
        }
    }

    // The crosshair.
    let hover = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|at| area.contains(*at));
    let Some(at) = hover else {
        return;
    };
    let xv = axes.x_of(at.x);
    let painter = ui.painter();
    painter.line_segment(
        [pos2(at.x, area.top()), pos2(at.x, area.bottom())],
        Stroke::new(1.0, color(p.border_strong)),
    );
    let mut rows: Vec<(Color32, String)> = Vec::new();
    let mut x_shown = xv;
    for s in &panel.series {
        if let Some((px, py)) = plot::nearest(&s.points, xv) {
            x_shown = px;
            painter.circle_filled(
                pos2(axes.fx(px), axes.fy(py)),
                3.5,
                series_color(look, s.slot),
            );
            rows.push((
                series_color(look, s.slot),
                format!("{}: {}", s.name, eng(py, unit)),
            ));
        }
    }
    let head = if plot.x_unit.is_empty() {
        axis_label(x_shown)
    } else {
        eng(x_shown, plot.x_unit)
    };
    let mut galleys = vec![(
        color(p.text_primary),
        painter.layout_no_wrap(head, mono(11.0), color(p.text_primary)),
    )];
    for (c, text) in rows.into_iter().take(8) {
        galleys.push((c, painter.layout_no_wrap(text, mono(11.0), c)));
    }
    let w = galleys
        .iter()
        .map(|(_, g)| g.size().x)
        .fold(0.0_f32, f32::max)
        + 16.0;
    let h: f32 = galleys.iter().map(|(_, g)| g.size().y + 2.0).sum::<f32>() + 10.0;
    let mut corner = pos2(at.x + 12.0, area.top() + 6.0);
    if corner.x + w > area.right() {
        corner.x = at.x - 12.0 - w;
    }
    let boxed = Rect::from_min_size(corner, vec2(w, h));
    painter.rect_filled(boxed, 6.0, color(p.surface_overlay));
    painter.rect_stroke(
        boxed,
        6.0,
        Stroke::new(1.0, color(p.border_default)),
        egui::StrokeKind::Inside,
    );
    let mut y = boxed.top() + 5.0;
    for (c, g) in galleys {
        let height = g.size().y + 2.0;
        painter.galley(pos2(boxed.left() + 8.0, y), g, c);
        y += height;
    }
}

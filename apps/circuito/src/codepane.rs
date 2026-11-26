// SPDX-License-Identifier: MIT OR Apache-2.0
//! The code pane: the circuit as text, coloured, next to the drawing. What is typed is read when
//! the typing pauses and the drawing follows; a text with problems lists them and leaves the
//! drawing as it was.

use std::sync::Arc;

use circuito_app::i18n::{tr, trf};
use circuito_app::script::{self, Class};
use circuito_app::workspace::Open;
use egui::text::{LayoutJob, TextFormat};
use egui::{vec2, Align, FontFamily, FontId, Layout, RichText, Ui};
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::color;

use crate::widgets::{field, icon, icon_button};

/// The most problems listed under the code; the rest are counted.
const MAX_LISTED: usize = 6;

fn ink(look: &Look, class: Class) -> egui::Color32 {
    let p = &look.palette;
    color(match class {
        Class::Comment => p.text_tertiary,
        Class::Keyword => p.status_info_fg,
        Class::Part => p.accent_text,
        Class::Key => p.text_secondary,
        Class::Number => p.status_warning_fg,
        Class::Net => p.status_success_fg,
        Class::Plain => p.text_primary,
    })
}

fn layout(look: &Look, text: &str, wrap: f32, ctx: &egui::Context) -> Arc<egui::Galley> {
    let font = FontId::new(13.0, FontFamily::Monospace);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    for span in script::highlight(text) {
        let Some(piece) = text.get(span.start..span.end) else {
            continue;
        };
        job.append(
            piece,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: ink(look, span.class),
                ..TextFormat::default()
            },
        );
    }
    ctx.fonts_mut(|f| f.layout_job(job))
}

pub fn show(ui: &mut Ui, look: &Look, doc: &mut Open, now: f64) {
    let p = look.palette;
    let problems = doc.script.problems.len() + usize::from(doc.script.error.is_some());
    eng::panel_header(ui, look, tr("Code"), |ui| {
        if icon_button(
            ui,
            look,
            "refresh",
            tr("Write the code again from the drawing"),
            26.0,
            false,
        )
        .clicked()
        {
            doc.script.typed_at = None;
            doc.script.regenerate = true;
        }
        let (name, tone, text) = if doc.script.typed_at.is_some() {
            (
                "more-horizontal",
                p.text_secondary,
                tr("Reading…").to_owned(),
            )
        } else if problems > 0 {
            (
                "error",
                p.status_danger_fg,
                trf("{} problems", &[&problems.to_string()]),
            )
        } else {
            (
                "check",
                p.status_success_fg,
                tr("In step with the drawing").to_owned(),
            )
        };
        ui.label(RichText::new(text).font(eng::caption()).color(color(tone)));
        icon(ui, name, 16.0, color(tone));
    });
    let listed =
        doc.script.problems.len().min(MAX_LISTED) + usize::from(doc.script.error.is_some());
    let list_h = if listed == 0 {
        0.0
    } else {
        (listed as f32 * 22.0 + 12.0).min(150.0)
    };
    let editor_h = (ui.available_height() - list_h - 8.0).max(80.0);
    field(look)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_min_height(editor_h - 14.0);
            ui.set_max_height(editor_h - 14.0);
            let width = ui.available_width();
            egui::ScrollArea::both()
                .id_salt("code-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Code is never wrapped: a long line scrolls sideways, so a line of the text is
                    // always one line on the screen.
                    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
                        layout(look, text.as_str(), f32::INFINITY, ui.ctx())
                    };
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut doc.script.text)
                            .id_salt(("circuito-code", doc.id))
                            .frame(egui::Frame::NONE)
                            .code_editor()
                            .lock_focus(true)
                            .layouter(&mut layouter)
                            .desired_width(width)
                            .desired_rows(12),
                    );
                    if response.changed() {
                        doc.script_typed(now);
                    }
                });
        });
    if listed > 0 {
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt("code-problems")
            .max_height(list_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for d in doc.script.problems.iter().take(MAX_LISTED) {
                    problem(ui, look, &script::line_of(d));
                }
                if let Some(e) = &doc.script.error {
                    problem(ui, look, &script::build_message(e));
                }
                let more = doc.script.problems.len().saturating_sub(MAX_LISTED);
                if more > 0 {
                    ui.label(
                        RichText::new(trf("and {} more", &[&more.to_string()]))
                            .font(eng::caption())
                            .color(color(p.text_secondary)),
                    );
                }
            });
    }
}

fn problem(ui: &mut Ui, look: &Look, text: &str) {
    ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
        icon(ui, "error", 14.0, color(look.palette.status_danger_fg));
        ui.add_space(2.0);
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .font(eng::caption())
                    .color(color(look.palette.status_danger_fg)),
            )
            .wrap(),
        );
    });
    ui.add_space(vec2(0.0, 2.0).y);
}

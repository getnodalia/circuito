// SPDX-License-Identifier: MIT OR Apache-2.0
//! The panels around the drawing: the navigator, the properties, the rules check, the simulation
//! panel, the netlist, the status bar and Home. They draw and collect what the person did; the
//! workspace carries it out.

use std::path::PathBuf;
use std::time::Duration;

use circuito_app::commands::Cmd;
use circuito_app::document::{Kind, Sample};
use circuito_app::findings;
use circuito_app::format::eng;
use circuito_app::geometry::{Selection, View};
use circuito_app::i18n::{tr, trf};
use circuito_app::plot;
use circuito_app::settings::Settings;
use circuito_app::workspace::{describe, Open, RunState, Workspace};
use circuito_erc::Severity;
use circuito_sim::Unit;
use egui::{vec2, Align2, Color32, Rect, RichText, Sense, Stroke, Ui};
use nodalia_theme::engineering::{self as eng_ui, Look, StartCard, TreeHit, TreeRow};
use nodalia_theme::for_egui::color;

use crate::plotview;
use crate::widgets::icon_button;

/// The most parts the navigator lists; a bigger sheet says how many more there are.
const MAX_LISTED: usize = 500;

fn top_down() -> egui::Layout {
    egui::Layout::top_down(egui::Align::Min)
}

fn caption(look: &Look, text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .font(eng_ui::caption())
        .color(color(look.palette.text_secondary))
}

fn body(look: &Look, text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .font(eng_ui::body())
        .color(color(look.palette.text_primary))
}

/// A read-only value in the monospaced face, cut at the panel's edge (the whole text is its
/// tooltip) instead of running out of the window.
fn value(ui: &mut Ui, look: &Look, text: &str) -> egui::Response {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .font(eng_ui::value_font())
                .color(color(look.palette.text_primary)),
        )
        .truncate(),
    )
    .on_hover_text(text)
}

fn centred_message(ui: &mut Ui, look: &Look, text: &str) {
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        eng_ui::body(),
        color(look.palette.text_secondary),
    );
}

// --- Navigator ----------------------------------------------------------------------------------

pub fn navigator(ui: &mut Ui, look: &Look, ws: &mut Workspace) {
    let Some(doc) = ws.active() else {
        eng_ui::panel_header(ui, look, tr("Navigator"), |_| {});
        centred_message(ui, look, tr("No circuit is open."));
        return;
    };
    let mut pick_sheet: Option<usize> = None;
    let mut pick_part: Option<usize> = None;
    let mut pick_analysis: Option<usize> = None;
    let bounds = ui.available_rect_before_wrap();
    ui.set_clip_rect(ui.clip_rect().intersect(bounds));
    egui::ScrollArea::vertical()
        .id_salt("navigator-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(schematic) = &doc.circuit.schematic {
                eng_ui::panel_header(ui, look, tr("Sheets"), |ui| {
                    ui.label(caption(look, schematic.sheets.len().to_string()));
                });
                for (i, sheet) in schematic.sheets.iter().enumerate() {
                    let (_, hit) = eng_ui::tree_row(
                        ui,
                        look,
                        &TreeRow {
                            depth: 0,
                            expanded: Some(i == doc.sheet),
                            icon: "layers",
                            label: &sheet.name,
                            selected: i == doc.sheet,
                            muted: false,
                        },
                    );
                    if hit != TreeHit::None {
                        pick_sheet = Some(i);
                    }
                    if i == doc.sheet {
                        for (k, part) in sheet.parts.iter().take(MAX_LISTED).enumerate() {
                            let label = format!("{}  {}", part.refdes, part.value);
                            let selected =
                                doc.selection == Some(Selection::Part { sheet: i, part: k });
                            let (_, hit) = eng_ui::tree_row(
                                ui,
                                look,
                                &TreeRow {
                                    depth: 1,
                                    expanded: None,
                                    icon: "symbol",
                                    label: &label,
                                    selected,
                                    muted: false,
                                },
                            );
                            if hit != TreeHit::None {
                                pick_part = Some(k);
                            }
                        }
                        if sheet.parts.len() > MAX_LISTED {
                            let more = sheet.parts.len() - MAX_LISTED;
                            ui.label(caption(
                                look,
                                trf("and {} more parts", &[&more.to_string()]),
                            ));
                        }
                    }
                }
                ui.add_space(8.0);
            }
            eng_ui::panel_header(ui, look, tr("Analyses"), |ui| {
                ui.label(caption(look, doc.runs.len().to_string()));
            });
            if doc.runs.is_empty() {
                ui.label(caption(look, tr("The circuit asks for no analysis.")));
            }
            for (i, run) in doc.runs.iter().enumerate() {
                let label = doc
                    .circuit
                    .analyses()
                    .get(i)
                    .map_or_else(String::new, describe);
                let icon = match run {
                    RunState::Idle => "play",
                    RunState::Queued(_) | RunState::Running(_) => "refresh",
                    RunState::Done { .. } => "check",
                    RunState::Failed(_) => "error",
                    RunState::Cancelled => "stop",
                };
                let (_, hit) = eng_ui::tree_row(
                    ui,
                    look,
                    &TreeRow {
                        depth: 0,
                        expanded: None,
                        icon,
                        label: &label,
                        selected: i == doc.analysis,
                        muted: matches!(run, RunState::Idle),
                    },
                );
                if hit != TreeHit::None {
                    pick_analysis = Some(i);
                }
            }
            let notices: Vec<&String> = doc.circuit.warnings.iter().collect();
            if !notices.is_empty() || doc.circuit.blocked.is_some() {
                ui.add_space(8.0);
                eng_ui::panel_header(ui, look, tr("Notices"), |_| {});
                if let Some(reason) = &doc.circuit.blocked {
                    ui.add(
                        egui::Label::new(
                            RichText::new(reason)
                                .font(eng_ui::caption())
                                .color(color(look.palette.status_danger_fg)),
                        )
                        .wrap(),
                    );
                }
                for w in notices.iter().take(20) {
                    ui.add(egui::Label::new(caption(look, w.as_str())).wrap());
                }
            }
        });
    if let Some(i) = pick_sheet {
        if let Some(doc) = ws.active_mut() {
            if doc.sheet != i {
                doc.sheet = i;
                doc.selection = None;
                doc.fit_pending = true;
            }
        }
    }
    if let Some(part) = pick_part {
        let sheet = ws.active().map_or(0, |d| d.sheet);
        ws.select(Some(Selection::Part { sheet, part }));
    }
    if let Some(i) = pick_analysis {
        ws.choose_analysis(i);
    }
}

// --- Properties ---------------------------------------------------------------------------------

pub fn properties(ui: &mut Ui, look: &Look, doc: Option<&Open>) {
    let Some(doc) = doc else {
        eng_ui::panel_header(ui, look, tr("Properties"), |_| {});
        centred_message(ui, look, tr("No circuit is open."));
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt("properties-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(info) = doc.part_info() {
                eng_ui::panel_header(ui, look, &info.refdes, |ui| {
                    ui.label(caption(look, info.name.clone()));
                });
                eng_ui::properties(ui, "part-props", |ui| {
                    eng_ui::property(ui, look, tr("Value"), |ui| value(ui, look, &info.value));
                    eng_ui::property(ui, look, tr("Position"), |ui| {
                        value(
                            ui,
                            look,
                            &format!("{}, {}", info.position.0, info.position.1),
                        )
                    });
                    eng_ui::property(ui, look, tr("Rotation"), |ui| {
                        value(ui, look, &format!("{}°", info.rotation))
                    });
                    if info.mirror {
                        eng_ui::property(ui, look, tr("Mirrored"), |ui| value(ui, look, tr("Yes")));
                    }
                    if let Some(model) = &info.model {
                        eng_ui::property(ui, look, tr("Model"), |ui| value(ui, look, model));
                    }
                    for (k, v) in &info.fields {
                        eng_ui::property(ui, look, k, |ui| value(ui, look, v));
                    }
                });
                ui.add_space(10.0);
                eng_ui::panel_header(ui, look, tr("Pins"), |ui| {
                    ui.label(caption(look, info.pins.len().to_string()));
                });
                eng_ui::properties(ui, "pin-props", |ui| {
                    for pin in &info.pins {
                        let name = if pin.name.is_empty() || pin.name == pin.number {
                            pin.number.clone()
                        } else {
                            format!("{} · {}", pin.number, pin.name)
                        };
                        eng_ui::property(ui, look, &name, |ui| {
                            value(ui, look, pin.net.as_deref().unwrap_or("—"))
                        });
                    }
                });
                return;
            }
            eng_ui::panel_header(ui, look, tr("Circuit"), |_| {});
            let c = &doc.circuit;
            eng_ui::properties(ui, "circuit-props", |ui| {
                eng_ui::property(ui, look, tr("Title"), |ui| value(ui, look, &c.title));
                eng_ui::property(ui, look, tr("Kind"), |ui| {
                    value(
                        ui,
                        look,
                        match c.kind {
                            Kind::Schematic => tr("Schematic"),
                            Kind::Spice => tr("SPICE deck"),
                        },
                    )
                });
                if let Some(s) = &c.schematic {
                    let parts: usize = s.sheets.iter().map(|sh| sh.parts.len()).sum();
                    let wires: usize = s.sheets.iter().map(|sh| sh.wires.len()).sum();
                    eng_ui::property(ui, look, tr("Sheets"), |ui| {
                        value(ui, look, &s.sheets.len().to_string())
                    });
                    eng_ui::property(ui, look, tr("Parts"), |ui| {
                        value(ui, look, &parts.to_string())
                    });
                    eng_ui::property(ui, look, tr("Wires"), |ui| {
                        value(ui, look, &wires.to_string())
                    });
                } else {
                    eng_ui::property(ui, look, tr("Lines"), |ui| {
                        value(ui, look, &c.netlist_lines.len().to_string())
                    });
                }
                eng_ui::property(ui, look, tr("Analyses"), |ui| {
                    value(ui, look, &doc.runs.len().to_string())
                });
                if let Some(path) = &c.path {
                    eng_ui::property(ui, look, tr("File"), |ui| {
                        ui.add(egui::Label::new(body(look, path.display().to_string())).wrap());
                    });
                }
            });
            if doc.circuit.schematic.is_some() {
                ui.add_space(10.0);
                ui.label(caption(
                    look,
                    tr("Click a part on the drawing to see its pins and the nets they are on."),
                ));
            }
        });
}

// --- Rules check --------------------------------------------------------------------------------

fn severity_style(look: &Look, s: Severity) -> (&'static str, Color32) {
    let p = &look.palette;
    match s {
        Severity::Error => ("error", color(p.status_danger_fg)),
        Severity::Warning => ("warning", color(p.status_warning_fg)),
        Severity::Info => ("info", color(p.status_info_fg)),
    }
}

pub fn rules(ui: &mut Ui, look: &Look, ws: &mut Workspace, cmds: &mut Vec<Cmd>) {
    let Some(doc) = ws.active() else {
        centred_message(ui, look, tr("No circuit is open."));
        return;
    };
    let Some(report) = &doc.circuit.report else {
        centred_message(
            ui,
            look,
            tr("The rules check reads schematics. A SPICE deck has no drawing to check."),
        );
        return;
    };
    let (errors, warnings, notes) = doc.counts();
    let p = &look.palette;
    let mut picked: Option<usize> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        for (n, icon, text, s) in [
            (errors, "error", tr("errors"), Severity::Error),
            (warnings, "warning", tr("warnings"), Severity::Warning),
            (notes, "info", tr("notes"), Severity::Info),
        ] {
            let (_, ink) = severity_style(look, s);
            crate::widgets::icon(
                ui,
                icon,
                16.0,
                if n > 0 { ink } else { color(p.text_tertiary) },
            );
            ui.label(
                RichText::new(format!("{n} {text}"))
                    .font(eng_ui::body())
                    .color(if n > 0 {
                        color(p.text_primary)
                    } else {
                        color(p.text_tertiary)
                    }),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(!report.violations.is_empty(), |ui| {
                if icon_button(ui, look, "arrow-down", tr("Next finding"), 28.0, false).clicked() {
                    cmds.push(Cmd::NextProblem);
                }
                if icon_button(ui, look, "arrow-up", tr("Previous finding"), 28.0, false).clicked()
                {
                    cmds.push(Cmd::PreviousProblem);
                }
            });
        });
    });
    ui.add_space(4.0);
    if report.violations.is_empty() {
        centred_message(ui, look, tr("No findings. The rules check passed."));
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("rules-scroll")
        .auto_shrink([false, false])
        .show_rows(ui, 44.0, report.violations.len(), |ui, range| {
            for i in range {
                let Some(v) = report.violations.get(i) else {
                    continue;
                };
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
                let selected = doc.finding == Some(i);
                if selected {
                    ui.painter().rect_filled(rect, 7.0, color(p.accent_subtle));
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 7.0, color(p.control_bg_hover));
                }
                let (icon, ink) = severity_style(look, v.severity);
                let icon_rect =
                    Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(18.0, 18.0));
                if let Some(glyph) = nodalia_theme::icons::get(icon) {
                    nodalia_theme::icons::paint(
                        ui.painter(),
                        icon_rect,
                        glyph,
                        nodalia_theme::icons::Style::bold(ink),
                    );
                }
                let clip = ui.painter().with_clip_rect(rect.shrink2(vec2(0.0, 0.0)));
                clip.text(
                    rect.left_top() + vec2(36.0, 7.0),
                    Align2::LEFT_TOP,
                    format!(
                        "{}  ·  {}",
                        findings::title(v.rule),
                        findings::severity_word(v.severity)
                    ),
                    eng_ui::body(),
                    if selected {
                        color(p.accent_text)
                    } else {
                        color(p.text_primary)
                    },
                );
                let detail = if v.message.is_empty() {
                    findings::items(v)
                } else {
                    v.message.clone()
                };
                let mut job = egui::text::LayoutJob::simple_singleline(
                    detail,
                    eng_ui::caption(),
                    color(p.text_secondary),
                );
                job.wrap.max_width = (rect.width() - 56.0).max(40.0);
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                clip.galley(
                    rect.left_top() + vec2(36.0, 25.0),
                    galley,
                    color(p.text_secondary),
                );
                clip.text(
                    rect.right_top() + vec2(-10.0, 7.0),
                    Align2::RIGHT_TOP,
                    v.code,
                    eng_ui::value_font(),
                    color(p.text_tertiary),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, true, findings::title(v.rule))
                });
                if response.clicked() {
                    picked = Some(i);
                }
            }
        });
    if let Some(i) = picked {
        ws.select_finding(i);
    }
}

// --- Simulation ---------------------------------------------------------------------------------

pub fn simulation(ui: &mut Ui, look: &Look, ws: &mut Workspace, cmds: &mut Vec<Cmd>) {
    let p = &look.palette;
    let Some(i) = ws.active_index() else {
        centred_message(ui, look, tr("No circuit is open."));
        return;
    };
    let busy = ws.busy();
    let progress = ws.progress(ws.active().map_or(0, |d| d.analysis));
    let (blocked, count, chosen) = match ws.active() {
        Some(d) => (d.circuit.blocked.clone(), d.runs.len(), d.analysis),
        None => return,
    };
    let _ = i;
    let mut pick: Option<usize> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let can_run = blocked.is_none() && count > 0;
        ui.add_enabled_ui(can_run, |ui| {
            if icon_button(ui, look, "run", tr("Run the analysis"), 28.0, false).clicked() {
                cmds.push(Cmd::Run);
            }
            if icon_button(ui, look, "play", tr("Run every analysis"), 28.0, false).clicked() {
                cmds.push(Cmd::RunAll);
            }
        });
        ui.add_enabled_ui(busy, |ui| {
            if icon_button(ui, look, "stop", tr("Stop the simulation"), 28.0, false).clicked() {
                cmds.push(Cmd::Stop);
            }
        });
        ui.add_space(8.0);
        if let Some(doc) = ws.active() {
            for (k, a) in doc.circuit.analyses().iter().enumerate() {
                if eng_ui::ribbon_tab(ui, look, &describe(a), k == chosen).clicked() {
                    pick = Some(k);
                }
            }
        }
    });
    if let Some(k) = pick {
        ws.choose_analysis(k);
    }
    ui.add_space(4.0);

    if let Some(reason) = &blocked {
        ui.add(
            egui::Label::new(
                RichText::new(trf("This circuit cannot be simulated: {}", &[reason]))
                    .font(eng_ui::body())
                    .color(color(p.status_danger_fg)),
            )
            .wrap(),
        );
        return;
    }
    if count == 0 {
        centred_message(
            ui,
            look,
            tr("This circuit asks for no analysis. Add a .op, .dc, .ac or .tran line."),
        );
        return;
    }

    let Some(doc) = ws.active_mut() else {
        return;
    };
    let state = doc
        .runs
        .get(doc.analysis)
        .cloned()
        .unwrap_or(RunState::Idle);
    match state {
        RunState::Idle => centred_message(ui, look, tr("Not run yet. Press Run (F5).")),
        RunState::Queued(_) | RunState::Running(_) => {
            let rect = ui.available_rect_before_wrap();
            ui.allocate_rect(rect, Sense::hover());
            let w = 280.0_f32.min(rect.width() - 40.0).max(80.0);
            let bar = Rect::from_center_size(rect.center(), vec2(w, 6.0));
            ui.painter().rect_filled(bar, 3.0, color(p.control_track));
            let f = progress.unwrap_or(0.0).clamp(0.0, 1.0);
            let fill = Rect::from_min_size(bar.min, vec2(bar.width() * f.max(0.02), bar.height()));
            ui.painter().rect_filled(fill, 3.0, color(p.accent_default));
            ui.painter().text(
                bar.center_top() - vec2(0.0, 12.0),
                Align2::CENTER_BOTTOM,
                format!("{}  {:.0}%", tr("Simulating…"), f * 100.0),
                eng_ui::body(),
                color(p.text_primary),
            );
        }
        RunState::Failed(m) => {
            ui.add(
                egui::Label::new(
                    RichText::new(trf("The simulation failed: {}", &[&m]))
                        .font(eng_ui::body())
                        .color(color(p.status_danger_fg)),
                )
                .wrap(),
            );
        }
        RunState::Cancelled => centred_message(ui, look, tr("Stopped before it finished.")),
        RunState::Done { result, elapsed } => {
            let footer = footer_text(&result, elapsed);
            if result.kind == circuito_sim::Kind::Op {
                operating_point(ui, look, &result);
            } else {
                let signals = plot::signals(&result);
                let hidden = doc.hidden.clone();
                let plot = doc.plot();
                let mut toggled: Option<String> = None;
                ui.horizontal_top(|ui| {
                    let side = 196.0;
                    let plot_w = (ui.available_width() - side - 8.0).max(120.0);
                    let h = (ui.available_height() - 22.0).max(60.0);
                    ui.allocate_ui_with_layout(vec2(plot_w, h), top_down(), |ui| {
                        if let Some(plot) = &plot {
                            plotview::show(ui, look, plot);
                        }
                    });
                    ui.allocate_ui_with_layout(vec2(side, h), top_down(), |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("signals-scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(caption(look, tr("Signals")));
                                for s in &signals {
                                    let shown = !hidden.contains(&s.name);
                                    if signal_row(ui, look, &s.name, s.slot, shown).clicked() {
                                        toggled = Some(s.name.clone());
                                    }
                                }
                            });
                    });
                });
                if let Some(name) = toggled {
                    ws.toggle_signal(&name);
                }
            }
            ui.label(caption(look, footer));
        }
    }
}

fn footer_text(result: &circuito_sim::AnalysisResult, elapsed: Duration) -> String {
    let info = &result.info;
    let mut text = trf(
        "Solved in {} · {} · Newton iterations: {}",
        &[
            &duration_text(elapsed),
            &info.engine,
            &info.newton_iterations.to_string(),
        ],
    );
    if info.accepted_steps > 0 {
        text.push_str(&format!(
            " · {}",
            trf("{} steps", &[&info.accepted_steps.to_string()])
        ));
    }
    text.push_str(&format!(" · reltol {:.0e}", info.reltol));
    if let Some(w) = info.warnings.first() {
        text.push_str(&format!(" · {w}"));
    }
    text
}

fn signal_row(ui: &mut Ui, look: &Look, name: &str, slot: usize, shown: bool) -> egui::Response {
    let p = &look.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, color(p.control_bg_hover));
    }
    let swatch = Rect::from_center_size(rect.left_center() + vec2(12.0, 0.0), vec2(12.0, 12.0));
    let c = plotview::series_color(look, slot);
    if shown {
        ui.painter().rect_filled(swatch, 3.0, c);
    } else {
        ui.painter()
            .rect_stroke(swatch, 3.0, Stroke::new(1.5, c), egui::StrokeKind::Inside);
    }
    ui.painter().text(
        rect.left_center() + vec2(28.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        eng_ui::value_font(),
        if shown {
            color(p.text_primary)
        } else {
            color(p.text_tertiary)
        },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, shown, name));
    response
}

fn operating_point(ui: &mut Ui, look: &Look, result: &circuito_sim::AnalysisResult) {
    let rows = plot::operating_point(result);
    let h = (ui.available_height() - 22.0).max(40.0);
    egui::ScrollArea::vertical()
        .id_salt("op-scroll")
        .max_height(h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let unit = |u: Unit| match u {
                Unit::Volt => "V",
                Unit::Ampere => "A",
            };
            let half = rows.len().div_ceil(2).max(4);
            ui.columns(if rows.len() > 6 { 2 } else { 1 }, |cols| {
                for (c, chunk) in cols.iter_mut().zip(rows.chunks(half)) {
                    eng_ui::properties(c, "op-table", |ui| {
                        for (name, v, u) in chunk {
                            eng_ui::property(ui, look, name, |ui| {
                                value(ui, look, &eng(*v, unit(*u)))
                            });
                        }
                    });
                }
            });
        });
}

pub fn duration_text(d: Duration) -> String {
    let micros = d.as_micros();
    if micros < 1000 {
        format!("{micros} µs")
    } else if micros < 1_000_000 {
        format!("{:.1} ms", micros as f64 / 1000.0)
    } else {
        format!("{:.2} s", d.as_secs_f64())
    }
}

// --- Netlist ------------------------------------------------------------------------------------

pub fn netlist(ui: &mut Ui, look: &Look, doc: &Open) {
    let p = &look.palette;
    let lines = &doc.circuit.netlist_lines;
    if lines.is_empty() {
        centred_message(ui, look, tr("There is no netlist for this circuit."));
        return;
    }
    let row = 18.0;
    let font = eng_ui::value_font();
    let digits = lines.len().to_string().len();
    egui::ScrollArea::both()
        .id_salt("netlist-scroll")
        .auto_shrink([false, false])
        .show_rows(ui, row, lines.len(), |ui, range| {
            for i in range {
                let Some(line) = lines.get(i) else {
                    continue;
                };
                let ink = if line.starts_with('*') {
                    color(p.text_tertiary)
                } else if line.starts_with('.') {
                    color(p.accent_text)
                } else {
                    color(p.text_primary)
                };
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    &format!("{:>digits$}  ", i + 1),
                    0.0,
                    egui::TextFormat::simple(font.clone(), color(p.text_tertiary)),
                );
                job.append(line, 0.0, egui::TextFormat::simple(font.clone(), ink));
                ui.add(egui::Label::new(job).extend());
            }
            if doc.circuit.netlist_cut {
                ui.label(caption(
                    look,
                    tr("The netlist is longer; only the start is shown."),
                ));
            }
        });
}

// --- Status bar ---------------------------------------------------------------------------------

/// What the status bar reports besides the open document.
#[derive(Clone, Copy)]
pub struct StatusInfo {
    pub busy: bool,
    pub lost: bool,
    pub hover: Option<(f32, f32)>,
}

pub fn status(
    ui: &mut Ui,
    look: &Look,
    doc: Option<&Open>,
    settings: &Settings,
    info: StatusInfo,
    cmds: &mut Vec<Cmd>,
) {
    let StatusInfo { busy, lost, hover } = info;
    let p = &look.palette;
    eng_ui::status_bar(
        ui,
        |ui| {
            let Some(doc) = doc else {
                ui.label(RichText::new(tr("No circuit")).color(color(p.text_secondary)));
                return;
            };
            if let Some(s) = &doc.circuit.schematic {
                let parts: usize = s.sheets.iter().map(|sh| sh.parts.len()).sum();
                let text = if parts == 1 {
                    trf("{} part", &["1"])
                } else {
                    trf("{} parts", &[&parts.to_string()])
                };
                eng_ui::status_value(ui, look, &text);
            } else {
                eng_ui::status_value(
                    ui,
                    look,
                    &trf("{} lines", &[&doc.circuit.netlist_lines.len().to_string()]),
                );
            }
            ui.label(RichText::new("·").color(color(p.text_tertiary)));
            let (e, w, _) = doc.counts();
            if doc.circuit.report.is_none() {
                ui.label(RichText::new(tr("Not a schematic")).color(color(p.text_secondary)));
            } else if e > 0 {
                ui.label(
                    RichText::new(trf("{} errors", &[&e.to_string()]))
                        .color(color(p.status_danger_fg)),
                );
            } else if w > 0 {
                ui.label(
                    RichText::new(trf("{} warnings", &[&w.to_string()]))
                        .color(color(p.status_warning_fg)),
                );
            } else {
                ui.label(RichText::new(tr("Rules check passed")).color(color(p.status_success_fg)));
            }
            ui.label(RichText::new("·").color(color(p.text_tertiary)));
            if lost {
                ui.label(
                    RichText::new(tr("The simulation engine stopped"))
                        .color(color(p.status_danger_fg)),
                );
            } else if busy {
                ui.label(RichText::new(tr("Simulating…")).color(color(p.accent_text)));
            } else if doc.result().is_some() {
                ui.label(RichText::new(tr("Result ready")).color(color(p.status_success_fg)));
            } else {
                ui.label(RichText::new(tr("Ready")).color(color(p.text_secondary)));
            }
        },
        |ui| {
            if eng_ui::status_toggle(ui, look, "grid", tr("Show the grid"), settings.show_grid)
                .clicked()
            {
                cmds.push(Cmd::ToggleGrid);
            }
            if let Some(doc) = doc {
                if doc.circuit.schematic.is_some() {
                    eng_ui::status_value(
                        ui,
                        look,
                        &format!("{:.0}%", doc.view.zoom / View::default().zoom * 100.0),
                    );
                }
            }
            if let Some((x, y)) = hover {
                // A grid unit is 1.27 mm (0.05 inch).
                eng_ui::status_value(ui, look, &format!("{:.2}, {:.2} mm", x * 1.27, y * 1.27));
            }
        },
    );
}

// --- Home ---------------------------------------------------------------------------------------

/// What Home asked for.
pub enum HomeAction {
    Sample(Sample),
    Open,
    Recent(PathBuf),
}

pub fn home(
    ui: &mut Ui,
    look: &Look,
    icon: egui::TextureId,
    settings: &Settings,
) -> Option<HomeAction> {
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("home-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(12.0);
            eng_ui::home_header(
                ui,
                look,
                Some(icon),
                circuito_app::NAME,
                tr("Schematics and SPICE decks to open, check and simulate."),
            );
            ui.label(
                RichText::new(tr("Start"))
                    .font(eng_ui::panel_title())
                    .color(color(look.palette.text_primary)),
            );
            ui.add_space(8.0);
            let mut cards: Vec<StartCard> = Sample::ALL
                .iter()
                .map(|s| StartCard {
                    icon: "sketch",
                    name: tr(s.label()),
                    detail: tr(s.detail()),
                })
                .collect();
            cards.push(StartCard {
                icon: "open",
                name: tr("Open…"),
                detail: tr("A circuit or SPICE deck from your computer"),
            });
            match eng_ui::start_cards(ui, look, &cards) {
                Some(i) if i < Sample::ALL.len() => {
                    if let Some(s) = Sample::ALL.get(i) {
                        action = Some(HomeAction::Sample(*s));
                    }
                }
                Some(_) => action = Some(HomeAction::Open),
                None => {}
            }
            ui.add_space(22.0);
            ui.label(
                RichText::new(tr("Recent circuits"))
                    .font(eng_ui::panel_title())
                    .color(color(look.palette.text_primary)),
            );
            ui.add_space(8.0);
            let names: Vec<(String, String)> = settings
                .recent
                .iter()
                .map(|p| {
                    (
                        circuito_app::document::file_name(p),
                        crate::app::folder_of(p),
                    )
                })
                .collect();
            let list: Vec<eng_ui::RecentDocument> = names
                .iter()
                .map(|(name, folder)| eng_ui::RecentDocument {
                    name,
                    folder,
                    when: "",
                    icon: "sketch",
                    thumbnail: None,
                })
                .collect();
            if let Some(i) =
                eng_ui::recent_documents(ui, look, &list, tr("Circuits you open appear here."))
            {
                if let Some(path) = settings.recent.get(i) {
                    action = Some(HomeAction::Recent(path.clone()));
                }
            }
        });
    action
}

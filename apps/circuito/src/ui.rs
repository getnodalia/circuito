// SPDX-License-Identifier: MIT OR Apache-2.0
//! The window: title bar, ribbon, navigator, drawing, properties, results panel and status bar.
//! Everything here only draws and collects what the person did; the app carries it out after the
//! frame, so no widget holds a borrow of the state it changes.

use circuito_app::commands::Cmd;
use circuito_app::editing::Tool;
use circuito_app::i18n::{tr, trf};
use circuito_app::workspace::{Bottom, Side, Tab};
use egui::{pos2, vec2, Id, Rect, Sense, Ui, UiBuilder};
use nodalia_theme::engineering::{
    self as eng, layout, menu, Document, Look, TitleAction, TitleBar, TitleLabels,
};
use nodalia_theme::for_egui::color;
use nodalia_theme::Mode;

use crate::app::{App, Overlay};
use crate::panels::HomeAction;
use crate::widgets::icon_button;
use crate::{canvas, chrome, codepane, inspector, overlays, palette, panels};

const RIBBON_H: f32 = 84.0;
const MIN_CENTER: f32 = 440.0;
const RESULTS_H: f32 = 330.0;

fn file_menu_id() -> Id {
    Id::unique("circuito-file-menu")
}

fn app_menu_id() -> Id {
    Id::unique("circuito-app-menu")
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let (look, mode) = app.look(&ctx);
    let ndl = app.ndl_controls();
    let full = ui.max_rect();
    ui.painter()
        .rect_filled(full, 0.0, color(look.palette.surface_window));

    let mut cmds: Vec<Cmd> = Vec::new();
    let gap = layout::GAP;
    app.hover = None;

    let title = Rect::from_min_size(full.min, vec2(full.width(), layout::TITLE_BAR));
    title_bar(ui, title, app, &look, mode, ndl, &mut cmds);

    let ribbon = Rect::from_min_max(
        pos2(full.left() + gap, title.bottom()),
        pos2(full.right() - gap, title.bottom() + RIBBON_H),
    );
    island(ui, &look, ribbon, "ribbon", |ui| {
        ribbon_tools(ui, &look, app, &mut cmds)
    });

    let status = Rect::from_min_max(
        pos2(full.left() + gap, full.bottom() - layout::STATUS_BAR),
        pos2(full.right() - gap, full.bottom()),
    );
    let body = Rect::from_min_max(
        pos2(full.left() + gap, ribbon.bottom() + gap),
        pos2(full.right() - gap, status.top()),
    );

    let has_doc = !app.home && app.ws.active().is_some();
    // Side panels give way before the drawing does.
    let mut left = app.settings.show_navigator && has_doc;
    let mut right = app.settings.show_properties && has_doc;
    let room = |l: bool, r: bool| {
        body.width()
            - if l { layout::BROWSER_WIDTH + gap } else { 0.0 }
            - if r {
                layout::PROPERTIES_WIDTH + gap
            } else {
                0.0
            }
    };
    if room(left, right) < MIN_CENTER {
        right = false;
    }
    if room(left, right) < MIN_CENTER {
        left = false;
    }
    let mut x0 = body.left();
    let mut x1 = body.right();
    if left {
        let r = Rect::from_min_max(
            body.min,
            pos2(body.left() + layout::BROWSER_WIDTH, body.bottom()),
        );
        island(ui, &look, r, "navigator", |ui| {
            left_panel(ui, &look, app, &mut cmds)
        });
        x0 = r.right() + gap;
    }
    if right {
        let r = Rect::from_min_max(
            pos2(body.right() - layout::PROPERTIES_WIDTH, body.top()),
            body.max,
        );
        let mut asked = inspector::Asked::default();
        island(ui, &look, r, "properties", |ui| {
            asked = inspector::show(ui, &look, app.ws.active());
        });
        cmds.extend(asked.commands);
        for action in asked.actions {
            app.apply(action);
        }
        x1 = r.left() - gap;
    }
    let column = Rect::from_min_max(pos2(x0, body.top()), pos2(x1, body.bottom()));
    let show_results = app.settings.show_results && has_doc;
    let results_h = if show_results {
        RESULTS_H.min(column.height() * 0.46).max(160.0)
    } else {
        0.0
    };
    let centre = Rect::from_min_max(
        column.min,
        pos2(
            column.right(),
            column.bottom() - if show_results { results_h + gap } else { 0.0 },
        ),
    );
    island(ui, &look, centre, "document", |ui| {
        centre_content(ui, &look, app, &ctx, &mut cmds);
    });
    if show_results {
        let r = Rect::from_min_max(pos2(column.left(), centre.bottom() + gap), column.max);
        island(ui, &look, r, "results", |ui| {
            results(ui, &look, app, &mut cmds)
        });
    }

    ui.scope_builder(UiBuilder::new().max_rect(status), |ui| {
        let busy = app.ws.busy();
        let lost = app.ws.runner_lost();
        panels::status(
            ui,
            &look,
            app.ws.active(),
            &app.settings,
            panels::StatusInfo {
                busy,
                lost,
                hover: app.hover,
            },
            &mut cmds,
        );
    });

    overlays::show(&ctx, app, &look);
    toasts(ui, app, &look);
    if ndl {
        chrome::resize_edges(ui, full);
    }
    for cmd in cmds {
        app.run(cmd);
    }
}

/// A rounded panel surface filling `rect`.
fn island(ui: &mut Ui, look: &Look, rect: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
        eng::island(look).show(ui, |ui| {
            let inner = vec2(rect.width() - 16.0, rect.height() - 12.0);
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            add(ui);
        });
    });
}

/// The left panel: the parts to draw with, or the sheets and analyses of the circuit.
fn left_panel(ui: &mut Ui, look: &Look, app: &mut App, cmds: &mut Vec<Cmd>) {
    let drawable = app.ws.active().is_some_and(|d| d.editable());
    let side = if drawable {
        app.ws.side
    } else {
        Side::Navigator
    };
    if drawable {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if eng::ribbon_tab(ui, look, tr("Parts"), side == Side::Parts).clicked() {
                app.ws.side = Side::Parts;
            }
            if eng::ribbon_tab(ui, look, tr("Navigator"), side == Side::Navigator).clicked() {
                app.ws.side = Side::Navigator;
            }
        });
        ui.add_space(4.0);
    }
    match side {
        Side::Parts => palette::show(ui, look, &mut app.ws, &mut app.part_filter, cmds),
        Side::Navigator => panels::navigator(ui, look, &mut app.ws),
    }
}

fn centre_content(
    ui: &mut Ui,
    look: &Look,
    app: &mut App,
    ctx: &egui::Context,
    cmds: &mut Vec<Cmd>,
) {
    if app.home || app.ws.active().is_none() {
        let icon = app.icon(ctx);
        match panels::home(ui, look, icon, &app.settings) {
            Some(HomeAction::New) => cmds.push(Cmd::New),
            Some(HomeAction::Sample(s)) => app.open_sample(s),
            Some(HomeAction::Open) => cmds.push(Cmd::Open),
            Some(HomeAction::Recent(path)) => app.open_path(path),
            None => {}
        }
        return;
    }
    let has_drawing = app
        .ws
        .active()
        .is_some_and(|d| d.circuit.schematic.is_some());
    let tab = if has_drawing {
        app.ws.tab
    } else {
        Tab::Netlist
    };
    if has_drawing {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if eng::ribbon_tab(ui, look, tr("Schematic"), tab == Tab::Schematic).clicked() {
                cmds.push(Cmd::ShowSchematic);
            }
            if eng::ribbon_tab(ui, look, tr("Netlist"), tab == Tab::Netlist).clicked() {
                cmds.push(Cmd::ShowNetlist);
            }
            if let Some(doc) = app.ws.active() {
                if let Some(s) = &doc.circuit.schematic {
                    if let Some(sheet) = s.sheets.get(doc.sheet) {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(&sheet.name)
                                    .font(eng::caption())
                                    .color(color(look.palette.text_secondary)),
                            );
                        });
                    }
                }
            }
        });
        ui.add_space(4.0);
    }
    let grid = app.settings.show_grid;
    match tab {
        Tab::Schematic => {
            let keys = app.overlay == Overlay::None && !ctx.egui_wants_keyboard_input();
            let tool = app.ws.tool.clone();
            let ghost = app.ws.ghost;
            let show_code = app.settings.show_script
                && app.ws.active().is_some_and(|d| d.script_available())
                && ui.available_width() > 640.0;
            let now = ctx.input(|i| i.time);
            let mut drew = None;
            let pen = canvas::Pen {
                tool: &tool,
                ghost,
                keys,
            };
            if show_code {
                let width = (ui.available_width() * 0.36).clamp(280.0, 460.0);
                let full = ui.available_rect_before_wrap();
                let (left, right) = (
                    Rect::from_min_max(full.min, pos2(full.right() - width - 8.0, full.bottom())),
                    Rect::from_min_max(pos2(full.right() - width, full.top()), full.max),
                );
                if let Some(doc) = app.ws.active_mut() {
                    ui.scope_builder(UiBuilder::new().max_rect(left), |ui| {
                        drew = Some(canvas::show(ui, look, doc, grid, &pen));
                    });
                    ui.scope_builder(UiBuilder::new().max_rect(right).id_salt("code"), |ui| {
                        codepane::show(ui, look, doc, now);
                    });
                }
            } else if let Some(doc) = app.ws.active_mut() {
                drew = Some(canvas::show(ui, look, doc, grid, &pen));
            }
            if let Some(out) = drew {
                app.hover = out.hover;
                cmds.extend(out.commands);
                if let Some(tool) = out.tool {
                    app.ws.tool = tool;
                }
            }
        }
        Tab::Netlist => {
            if let Some(doc) = app.ws.active() {
                panels::netlist(ui, look, doc);
            }
        }
    }
}

fn results(ui: &mut Ui, look: &Look, app: &mut App, cmds: &mut Vec<Cmd>) {
    let (errors, warnings, _) = app.ws.active().map_or((0, 0, 0), |d| d.counts());
    let findings = errors + warnings;
    let bottom = app.ws.bottom;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let rules = if findings > 0 {
            trf("Rules check ({})", &[&findings.to_string()])
        } else {
            tr("Rules check").to_owned()
        };
        if eng::ribbon_tab(ui, look, &rules, bottom == Bottom::Rules).clicked() {
            cmds.push(Cmd::ShowProblems);
        }
        if eng::ribbon_tab(ui, look, tr("Simulation"), bottom == Bottom::Simulation).clicked() {
            cmds.push(Cmd::ShowSimulation);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, look, "close", tr("Hide the results panel"), 26.0, false).clicked() {
                cmds.push(Cmd::ToggleBottomPanel);
            }
        });
    });
    ui.add_space(4.0);
    match bottom {
        Bottom::Rules => panels::rules(ui, look, &mut app.ws, cmds),
        Bottom::Simulation => panels::simulation(ui, look, &mut app.ws, cmds),
    }
}

fn ribbon_tools(ui: &mut Ui, look: &Look, app: &App, cmds: &mut Vec<Cmd>) {
    let doc = app.ws.active();
    let has = doc.is_some() && !app.home;
    let drawing = doc.is_some_and(|d| d.circuit.schematic.is_some());
    let editing = has && doc.is_some_and(|d| d.editable()) && app.ws.tab() == Tab::Schematic;
    let can_run = doc.is_some_and(|d| d.circuit.blocked.is_none() && !d.runs.is_empty());
    let busy = app.ws.busy();
    let s = &app.settings;
    let tool = |ui: &mut Ui, cmd: Cmd, enabled: bool, active: bool, cmds: &mut Vec<Cmd>| {
        let name = cmd.icon().unwrap_or("more-horizontal");
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                eng::ribbon_tool(ui, look, name, short_label(cmd), active)
            })
            .inner;
        let hint = match cmd.shortcut_label(cfg!(target_os = "macos")) {
            Some(k) => format!("{}  {k}", cmd.translated()),
            None => cmd.translated().to_owned(),
        };
        if response.on_hover_text(hint).clicked() && enabled {
            cmds.push(cmd);
        }
    };
    egui::ScrollArea::horizontal()
        .id_salt("ribbon-scroll")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                eng::ribbon_group(ui, look, tr("Circuit"), |ui| {
                    tool(ui, Cmd::New, true, false, cmds);
                    tool(ui, Cmd::Open, true, false, cmds);
                    tool(ui, Cmd::Save, editing, false, cmds);
                });
                eng::ribbon_group(ui, look, tr("Edit"), |ui| {
                    tool(ui, Cmd::Undo, editing, false, cmds);
                    tool(ui, Cmd::Redo, editing, false, cmds);
                    tool(ui, Cmd::Delete, editing, false, cmds);
                });
                eng::ribbon_group(ui, look, tr("Draw"), |ui| {
                    let now = &app.ws.tool;
                    tool(ui, Cmd::ToolSelect, editing, *now == Tool::Select, cmds);
                    tool(ui, Cmd::ToolWire, editing, *now == Tool::Wire, cmds);
                    tool(ui, Cmd::ToolLabel, editing, *now == Tool::Label, cmds);
                    tool(
                        ui,
                        Cmd::ToolPower,
                        editing,
                        matches!(now, Tool::Power(_)),
                        cmds,
                    );
                    tool(ui, Cmd::RotateRight, editing, false, cmds);
                    tool(ui, Cmd::MirrorHorizontal, editing, false, cmds);
                });
                eng::ribbon_group(ui, look, tr("View"), |ui| {
                    tool(ui, Cmd::FitView, drawing, false, cmds);
                    tool(ui, Cmd::ToggleGrid, drawing, s.show_grid, cmds);
                    tool(ui, Cmd::ToggleScript, editing, s.show_script, cmds);
                });
                eng::ribbon_group(ui, look, tr("Panels"), |ui| {
                    tool(ui, Cmd::ToggleNavigator, true, s.show_navigator, cmds);
                    tool(ui, Cmd::ToggleProperties, true, s.show_properties, cmds);
                    tool(ui, Cmd::ToggleBottomPanel, true, s.show_results, cmds);
                });
                eng::ribbon_group(ui, look, tr("Simulation"), |ui| {
                    tool(ui, Cmd::Run, can_run, false, cmds);
                    tool(ui, Cmd::RunAll, can_run, false, cmds);
                    tool(ui, Cmd::Stop, busy, false, cmds);
                });
                eng::ribbon_group(ui, look, tr("Rules"), |ui| {
                    tool(ui, Cmd::PreviousProblem, drawing, false, cmds);
                    tool(ui, Cmd::NextProblem, drawing, false, cmds);
                });
            });
        });
}

/// A command's label as it fits under a ribbon icon.
fn short_label(cmd: Cmd) -> &'static str {
    match cmd {
        Cmd::New => tr("New"),
        Cmd::Save => tr("Save"),
        Cmd::Undo => tr("Undo"),
        Cmd::Redo => tr("Redo"),
        Cmd::Duplicate => tr("Duplicate"),
        Cmd::Delete => tr("Delete"),
        Cmd::ToolSelect => tr("Select"),
        Cmd::ToolWire => tr("Wire"),
        Cmd::ToolLabel => tr("Label"),
        Cmd::ToolPower => tr("Ground"),
        Cmd::ToolNoConnect => tr("No-connect"),
        Cmd::RotateRight => tr("Rotate"),
        Cmd::MirrorHorizontal => tr("Mirror"),
        Cmd::ToggleScript => tr("Code"),
        Cmd::Open => tr("Open"),
        Cmd::Reload => tr("Reload"),
        Cmd::FitView => tr("Fit"),
        Cmd::ZoomIn => tr("Zoom in"),
        Cmd::ZoomOut => tr("Zoom out"),
        Cmd::ToggleGrid => tr("Grid"),
        Cmd::ShowSchematic => tr("Schematic"),
        Cmd::ShowNetlist => tr("Netlist"),
        Cmd::ToggleNavigator => tr("Navigator"),
        Cmd::ToggleProperties => tr("Properties"),
        Cmd::ToggleBottomPanel => tr("Results"),
        Cmd::Run => tr("Run"),
        Cmd::RunAll => tr("Run all"),
        Cmd::Stop => tr("Stop"),
        Cmd::PreviousProblem => tr("Previous"),
        Cmd::NextProblem => tr("Next"),
        other => other.translated(),
    }
}

fn title_bar(
    ui: &mut Ui,
    bar: Rect,
    app: &mut App,
    look: &Look,
    mode: Mode,
    ndl: bool,
    cmds: &mut Vec<Cmd>,
) {
    let ctx = ui.ctx().clone();
    let controls_left = ndl && chrome::side() == nodalia_theme::titlebar::Side::Left;
    let controls_width = if ndl && !controls_left {
        chrome::CONTROLS_WIDTH
    } else {
        0.0
    };
    let leading_width = if controls_left {
        chrome::CONTROLS_WIDTH
    } else {
        chrome::leading_inset()
    };
    let names: Vec<String> = app.ws.docs().iter().map(|d| d.circuit.name()).collect();
    let documents: Vec<Document> = names
        .iter()
        .map(|name| Document {
            name,
            modified: false,
        })
        .collect();
    let dirty: Vec<bool> = app.ws.docs().iter().map(|d| d.dirty()).collect();
    let documents: Vec<Document> = documents
        .into_iter()
        .zip(dirty)
        .map(|(d, modified)| Document { modified, ..d })
        .collect();
    let icon = app.icon(&ctx);
    let title = TitleBar {
        app: circuito_app::NAME,
        icon: Some(icon),
        documents: &documents,
        active: if app.home {
            None
        } else {
            app.ws.active_index()
        },
        labels: TitleLabels {
            home: tr("Home"),
            file: tr("File"),
            save: tr("Save"),
            undo: tr("Undo"),
            redo: tr("Redo"),
            new_document: tr("New circuit"),
            close_document: tr("Close"),
            search: tr("Search commands"),
            menu: tr("Main menu"),
        },
        quick_access: &[],
        controls_width,
        leading_width,
    };
    let out = ui
        .scope_builder(UiBuilder::new().max_rect(bar), |ui| {
            eng::title_bar_with(ui, look, &title, |ui, rect| {
                if chrome::drags_window(ndl) {
                    chrome::drag_area(ui, rect.with_max_x(rect.right() - controls_width))
                } else {
                    ui.interact(rect, ui.scope_id().with("circuito_title"), Sense::hover())
                }
            })
        })
        .inner;
    if ndl {
        chrome::controls(
            ui,
            if controls_left { bar } else { out.controls },
            &look.palette,
            mode,
        );
    }
    let anchor = |x: f32| pos2(x, bar.bottom() - 4.0);
    let at = |ui: &Ui, fallback: f32| ui.ctx().pointer_interact_pos().map_or(fallback, |p| p.x);
    match out.action {
        Some(TitleAction::Home) => app.home = true,
        Some(TitleAction::File) => {
            let x = at(ui, bar.left()) - 20.0;
            ui.data_mut(|d| d.insert_temp(file_menu_id().with("at"), anchor(x)));
            egui::Popup::toggle_id(&ctx, file_menu_id());
        }
        Some(TitleAction::Menu) => {
            let x = at(ui, bar.right());
            ui.data_mut(|d| d.insert_temp(app_menu_id().with("at"), anchor(x)));
            egui::Popup::toggle_id(&ctx, app_menu_id());
        }
        Some(TitleAction::NewDocument) => cmds.push(Cmd::New),
        Some(TitleAction::CloseDocument(i)) => {
            if let Some(id) = app.ws.docs().get(i).map(|d| d.id) {
                app.request_close(id);
            }
        }
        Some(TitleAction::Document(i)) => {
            app.ws.activate(i);
            app.home = false;
        }
        Some(TitleAction::Search) => cmds.push(Cmd::CommandSearch),
        Some(TitleAction::Save) => cmds.push(Cmd::Save),
        Some(TitleAction::Undo) => cmds.push(Cmd::Undo),
        Some(TitleAction::Redo) => cmds.push(Cmd::Redo),
        None => {}
    }
    title_menus(ui, look, cmds);
}

/// The File menu and the hamburger menu, opened from the title bar.
fn title_menus(ui: &mut Ui, look: &Look, cmds: &mut Vec<Cmd>) {
    let ctx = ui.ctx().clone();
    let at = |id: Id| {
        ctx.data(|d| d.get_temp::<egui::Pos2>(id.with("at")))
            .unwrap_or_default()
    };
    let popup = |id: Id| {
        menu::popup(
            egui::Popup::new(id, ctx.clone(), at(id), ui.layer_id())
                .kind(egui::PopupKind::Menu)
                .open_memory(None),
            look,
        )
    };
    let mac = cfg!(target_os = "macos");
    let item = |ui: &mut Ui, cmd: Cmd, cmds: &mut Vec<Cmd>| {
        let shortcut = cmd.shortcut_label(mac);
        let mut row = menu::Item::new(cmd.translated()).gutter();
        if let Some(icon) = cmd.icon() {
            row = row.icon(icon);
        }
        if let Some(s) = &shortcut {
            row = row.shortcut(s);
        }
        if menu::item(ui, look, &row).clicked() {
            cmds.push(cmd);
            ui.close();
        }
    };
    popup(file_menu_id()).show(|ui| {
        ui.set_min_width(menu::metric::MIN_WIDTH + 40.0);
        for cmd in [Cmd::New, Cmd::Open] {
            item(ui, cmd, cmds);
        }
        menu::separator(ui, look);
        for cmd in [Cmd::Save, Cmd::SaveAs, Cmd::Reload, Cmd::CloseDocument] {
            item(ui, cmd, cmds);
        }
        menu::separator(ui, look);
        item(ui, Cmd::Quit, cmds);
    });
    popup(app_menu_id()).show(|ui| {
        ui.set_min_width(menu::metric::MIN_WIDTH);
        for cmd in [Cmd::Preferences, Cmd::KeyboardShortcuts, Cmd::About] {
            item(ui, cmd, cmds);
        }
    });
}

fn toasts(ui: &mut Ui, app: &App, look: &Look) {
    let p = &look.palette;
    let full = ui.max_rect();
    let mut y = full.bottom() - layout::STATUS_BAR - 12.0;
    for toast in app.toasts.iter().rev() {
        let galley = ui.painter().layout(
            toast.text.clone(),
            eng::body(),
            color(p.text_primary),
            380.0,
        );
        let size = galley.size() + vec2(28.0, 18.0);
        let rect = Rect::from_min_size(pos2(full.right() - size.x - 16.0, y - size.y), size);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            Id::unique(("circuito-toast", toast.text.len())),
        ));
        painter.rect_filled(rect, 12.0, color(p.surface_overlay));
        painter.rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(1.0, color(p.border_default)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + vec2(14.0, 9.0), galley, color(p.text_primary));
        y = rect.top() - 8.0;
    }
}

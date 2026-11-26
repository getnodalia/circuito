// SPDX-License-Identifier: MIT OR Apache-2.0
//! The properties panel: what the selection is and the fields that change it. It collects
//! [`Action`]s and commands; the app carries them out after the frame.

use circuito_app::commands::Cmd;
use circuito_app::document::Kind;
use circuito_app::editing::Action;
use circuito_app::i18n::{tr, trf};
use circuito_app::workspace::Open;
use circuito_core::edit::Item;
use circuito_core::library::Library;
use circuito_core::schematic::LabelScope;
use egui::{Id, Key, RichText, Ui};
use nodalia_theme::engineering::{self as eng_ui, Look};
use nodalia_theme::for_egui::color;

use crate::widgets::{field, icon_button};

/// What the panel asks for.
#[derive(Default)]
pub struct Asked {
    pub actions: Vec<Action>,
    pub commands: Vec<Cmd>,
}

fn caption(look: &Look, text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .font(eng_ui::caption())
        .color(color(look.palette.text_secondary))
}

fn read_only(ui: &mut Ui, look: &Look, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .font(eng_ui::value_font())
                .color(color(look.palette.text_primary)),
        )
        .truncate(),
    )
    .on_hover_text(text);
}

/// A text field that shows `current` until it is clicked into, and hands back what was typed
/// when it is left (Enter or a click elsewhere). Escape gives the edit up.
fn text(ui: &mut Ui, look: &Look, id: Id, current: &str, hint: &str) -> Option<String> {
    let mut buffer = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| current.to_owned());
    if !ui.memory(|m| m.has_focus(id)) {
        buffer = current.to_owned();
    }
    let response = field(look)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .font(eng_ui::value_font())
                    .hint_text(hint)
                    .align(egui::Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY),
            )
        })
        .inner;
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(id, buffer.clone()));
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        let gave_up = ui.input(|i| i.key_pressed(Key::Escape));
        if !gave_up && buffer != current {
            return Some(buffer);
        }
    }
    None
}

pub fn show(ui: &mut Ui, look: &Look, doc: Option<&Open>) -> Asked {
    let mut asked = Asked::default();
    let Some(doc) = doc else {
        eng_ui::panel_header(ui, look, tr("Properties"), |_| {});
        return asked;
    };
    egui::ScrollArea::vertical()
        .id_salt("properties-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let sheet = doc.current_sheet();
            let one = match doc.items.iter().collect::<Vec<_>>().as_slice() {
                [only] => Some(**only),
                _ => None,
            };
            let key = |name: &str| Id::NULL.with(("circuito-prop", doc.id, name));
            match (one, sheet) {
                (Some(Item::Part(k)), Some(sheet)) if sheet.parts.get(k).is_some() => {
                    part(ui, look, doc, k, &key, &mut asked);
                }
                (Some(Item::Label(k)), Some(sheet)) => {
                    if let Some(label) = sheet.labels.get(k) {
                        eng_ui::panel_header(ui, look, tr("Net label"), |_| {});
                        eng_ui::properties(ui, "label-props", |ui| {
                            eng_ui::property(ui, look, tr("Name"), |ui| {
                                if let Some(v) =
                                    text(ui, look, key(&format!("label{k}")), &label.name, "")
                                {
                                    asked.actions.push(Action::LabelName(v));
                                }
                            });
                            eng_ui::property(ui, look, tr("Reaches"), |ui| {
                                ui.horizontal(|ui| {
                                    for (scope, name) in [
                                        (LabelScope::Local, tr("This sheet")),
                                        (LabelScope::Global, tr("Every sheet")),
                                    ] {
                                        if ui.selectable_label(label.scope == scope, name).clicked()
                                            && label.scope != scope
                                        {
                                            asked.actions.push(Action::LabelScope(scope));
                                        }
                                    }
                                });
                            });
                        });
                        ui.add_space(6.0);
                        ui.add(
                            egui::Label::new(caption(
                                look,
                                tr("Labels with the same name on a sheet are one net."),
                            ))
                            .wrap(),
                        );
                        actions(ui, look, &mut asked, false);
                    }
                }
                (Some(Item::Port(k)), Some(sheet)) => {
                    if let Some(port) = sheet.power_ports.get(k) {
                        eng_ui::panel_header(ui, look, tr("Power port"), |_| {});
                        eng_ui::properties(ui, "port-props", |ui| {
                            eng_ui::property(ui, look, tr("Net"), |ui| {
                                if let Some(v) =
                                    text(ui, look, key(&format!("port{k}")), &port.name, "")
                                {
                                    asked.actions.push(Action::PortName(v));
                                }
                            });
                        });
                        actions(ui, look, &mut asked, false);
                    }
                }
                (Some(Item::Wire(_)), _) => {
                    eng_ui::panel_header(ui, look, tr("Wire"), |_| {});
                    ui.add(
                        egui::Label::new(caption(
                            look,
                            tr("A wire is one conductor. Its ends join what they touch."),
                        ))
                        .wrap(),
                    );
                    actions(ui, look, &mut asked, false);
                }
                (Some(_), _) => {
                    actions(ui, look, &mut asked, false);
                }
                (None, _) if doc.items.len() > 1 => {
                    eng_ui::panel_header(ui, look, tr("Selection"), |ui| {
                        ui.label(caption(look, doc.items.len().to_string()));
                    });
                    ui.add(
                        egui::Label::new(caption(
                            look,
                            trf("{} things are selected.", &[&doc.items.len().to_string()]),
                        ))
                        .wrap(),
                    );
                    actions(ui, look, &mut asked, true);
                }
                _ => circuit(ui, look, doc, &key, &mut asked),
            }
        });
    asked
}

/// Rotate, mirror, duplicate and delete buttons for the selection.
fn actions(ui: &mut Ui, look: &Look, asked: &mut Asked, turnable: bool) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut button = |ui: &mut Ui, cmd: Cmd| {
            if icon_button(
                ui,
                look,
                cmd.icon().unwrap_or("more-horizontal"),
                cmd.translated(),
                30.0,
                false,
            )
            .clicked()
            {
                asked.commands.push(cmd);
            }
        };
        if turnable {
            button(ui, Cmd::RotateRight);
            button(ui, Cmd::MirrorHorizontal);
        }
        button(ui, Cmd::Duplicate);
        button(ui, Cmd::Delete);
    });
}

fn part(
    ui: &mut Ui,
    look: &Look,
    doc: &Open,
    k: usize,
    key: &dyn Fn(&str) -> Id,
    asked: &mut Asked,
) {
    let Some(p) = doc.current_sheet().and_then(|s| s.parts.get(k)) else {
        return;
    };
    let def = Library::builtin().get(&p.lib_id);
    let name = def.map_or_else(|| p.lib_id.clone(), |d| d.name.to_owned());
    eng_ui::panel_header(ui, look, &p.refdes, |ui| {
        ui.label(caption(look, name));
    });
    eng_ui::properties(ui, "part-props", |ui| {
        eng_ui::property(ui, look, tr("Reference"), |ui| {
            if let Some(v) = text(ui, look, key(&format!("refdes{k}")), &p.refdes, "") {
                asked.actions.push(Action::Refdes(v));
            }
        });
        if def.is_some_and(circuito_script::takes_value) {
            eng_ui::property(ui, look, tr("Value"), |ui| {
                if let Some(v) = text(ui, look, key(&format!("value{k}")), &p.value, "") {
                    asked.actions.push(Action::Value(v));
                }
            });
        }
        if def.is_some_and(|d| d.default_model.is_some()) {
            eng_ui::property(ui, look, tr("Model"), |ui| {
                let current = p.model.clone().unwrap_or_default();
                if let Some(v) = text(ui, look, key(&format!("model{k}")), &current, "") {
                    let v = v.trim().to_owned();
                    asked
                        .actions
                        .push(Action::Model((!v.is_empty()).then_some(v)));
                }
            });
        }
        for (field_name, current) in &p.fields {
            eng_ui::property(ui, look, field_name, |ui| {
                ui.horizontal(|ui| {
                    if let Some(v) =
                        text(ui, look, key(&format!("field{k}{field_name}")), current, "")
                    {
                        asked.actions.push(Action::Field(field_name.clone(), v));
                    }
                });
            });
        }
        eng_ui::property(ui, look, tr("Position"), |ui| {
            read_only(ui, look, &format!("{}, {}", p.position.x, p.position.y));
        });
    });
    if let Some(info) = doc.part_info() {
        ui.add_space(4.0);
        actions(ui, look, asked, true);
        ui.add_space(10.0);
        eng_ui::panel_header(ui, look, tr("Pins"), |ui| {
            ui.label(caption(look, info.pins.len().to_string()));
        });
        eng_ui::properties(ui, "pin-props", |ui| {
            for pin in &info.pins {
                let name = if pin.name.is_empty() || pin.name == pin.number || pin.name == "~" {
                    pin.number.clone()
                } else {
                    format!("{} · {}", pin.number, pin.name)
                };
                eng_ui::property(ui, look, &name, |ui| {
                    read_only(ui, look, pin.net.as_deref().unwrap_or("—"));
                });
            }
        });
    }
}

fn circuit(ui: &mut Ui, look: &Look, doc: &Open, key: &dyn Fn(&str) -> Id, asked: &mut Asked) {
    let c = &doc.circuit;
    eng_ui::panel_header(ui, look, tr("Circuit"), |_| {});
    let editable = c.kind == Kind::Schematic && doc.editable();
    eng_ui::properties(ui, "circuit-props", |ui| {
        eng_ui::property(ui, look, tr("Title"), |ui| {
            if editable {
                if let Some(v) = text(ui, look, key("title"), &c.title, "") {
                    asked.actions.push(Action::Title(v));
                }
            } else {
                read_only(ui, look, &c.title);
            }
        });
        eng_ui::property(ui, look, tr("Kind"), |ui| {
            read_only(
                ui,
                look,
                match c.kind {
                    Kind::Schematic => tr("Schematic"),
                    Kind::Spice => tr("SPICE deck"),
                },
            );
        });
        if let Some(s) = &c.schematic {
            let parts: usize = s.sheets.iter().map(|sh| sh.parts.len()).sum();
            let wires: usize = s.sheets.iter().map(|sh| sh.wires.len()).sum();
            eng_ui::property(ui, look, tr("Sheets"), |ui| {
                read_only(ui, look, &s.sheets.len().to_string());
            });
            eng_ui::property(ui, look, tr("Parts"), |ui| {
                read_only(ui, look, &parts.to_string());
            });
            eng_ui::property(ui, look, tr("Wires"), |ui| {
                read_only(ui, look, &wires.to_string());
            });
        } else {
            eng_ui::property(ui, look, tr("Lines"), |ui| {
                read_only(ui, look, &c.netlist_lines.len().to_string());
            });
        }
        if let Some(path) = &c.path {
            eng_ui::property(ui, look, tr("File"), |ui| {
                ui.add(egui::Label::new(caption(look, path.display().to_string())).truncate())
                    .on_hover_text(path.display().to_string());
            });
        }
    });
    if !editable {
        return;
    }
    ui.add_space(10.0);
    let lines = doc.analysis_lines();
    eng_ui::panel_header(ui, look, tr("Analyses"), |ui| {
        ui.label(caption(look, lines.len().to_string()));
    });
    for (i, line) in lines.iter().enumerate() {
        ui.horizontal(|ui| {
            let width = ui.available_width() - 34.0;
            ui.allocate_ui(egui::vec2(width.max(80.0), 28.0), |ui| {
                if let Some(v) = text(ui, look, key(&format!("analysis{i}")), line, "") {
                    asked.actions.push(Action::Analysis(Some(i), v));
                }
            });
            if icon_button(ui, look, "close", tr("Remove"), 26.0, false).clicked() {
                asked.actions.push(Action::RemoveAnalysis(i));
            }
        });
    }
    // A new line: type it and press Enter.
    let id = key(&format!("new-analysis{}", lines.len()));
    if let Some(v) = text(ui, look, id, "", ".ac dec 20 10 100k") {
        if !v.trim().is_empty() {
            asked.actions.push(Action::Analysis(None, v));
        }
    }
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(caption(
            look,
            tr("Click a part on the drawing to edit it, or pick one from the list on the left and click to place it."),
        ))
        .wrap(),
    );
}

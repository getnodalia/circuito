// SPDX-License-Identifier: MIT OR Apache-2.0
//! The parts palette: the tools that draw (select, wire, label, ground, no-connect) and the
//! library's parts, grouped, searchable and draggable onto the drawing.

use circuito_app::commands::Cmd;
use circuito_app::editing::Tool;
use circuito_app::i18n::tr;
use circuito_app::parts::{self, Payload};
use circuito_app::workspace::Workspace;
use egui::{vec2, Rect, RichText, Sense, Ui};
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::color;

use crate::canvas;
use crate::widgets::{field, icon_button};

const ROW_H: f32 = 40.0;

/// Draw the palette. `filter` is what is typed in its search field.
pub fn show(
    ui: &mut Ui,
    look: &Look,
    ws: &mut Workspace,
    filter: &mut String,
    cmds: &mut Vec<Cmd>,
) {
    let p = look.palette;
    let editable = ws.active().is_some_and(|d| d.editable());
    if !editable {
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tr("A SPICE deck has no drawing to edit."),
            eng::body(),
            color(p.text_secondary),
        );
        return;
    }
    // The drawing tools.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (cmd, tool) in [
            (Cmd::ToolSelect, Tool::Select),
            (Cmd::ToolWire, Tool::Wire),
            (Cmd::ToolLabel, Tool::Label),
            (Cmd::ToolPower, Tool::Power("GND".to_owned())),
            (Cmd::ToolNoConnect, Tool::NoConnect),
        ] {
            let active =
                ws.tool == tool || matches!((&ws.tool, &tool), (Tool::Power(_), Tool::Power(_)));
            let name = cmd.icon().unwrap_or("more-horizontal");
            if icon_button(ui, look, name, cmd.translated(), 30.0, active).clicked() {
                cmds.push(cmd);
            }
        }
    });
    ui.add_space(6.0);
    field(look).show(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(filter)
                .frame(egui::Frame::NONE)
                .font(eng::body())
                .hint_text(tr("Search parts"))
                .align(egui::Align2::LEFT_CENTER)
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(6.0);
    let needle = filter.trim().to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("palette-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut any = false;
            for group in parts::GROUPS {
                let entries: Vec<_> = group
                    .entries
                    .iter()
                    .filter(|e| e.matches(&needle))
                    .collect();
                if entries.is_empty() {
                    continue;
                }
                any = true;
                eng::panel_header(ui, look, group.label(), |_| {});
                for entry in entries {
                    let active = ws.tool == Tool::Part(entry.id.to_owned());
                    let r = row(
                        ui,
                        look,
                        entry.label(),
                        entry.id,
                        active,
                        |painter, rect| {
                            canvas::thumbnail(painter, rect, entry.id, color(p.text_primary));
                        },
                    );
                    if r.clicked() {
                        ws.tool = Tool::Part(entry.id.to_owned());
                    }
                    r.dnd_set_drag_payload(Payload::Part(entry.id));
                }
                ui.add_space(6.0);
            }
            let ports: Vec<&str> = parts::POWER_PORTS
                .iter()
                .copied()
                .filter(|n| {
                    needle.is_empty()
                        || n.to_lowercase().contains(&needle)
                        || parts::power_label(n).to_lowercase().contains(&needle)
                })
                .collect();
            if !ports.is_empty() {
                any = true;
                eng::panel_header(ui, look, tr("Power and ground"), |_| {});
                for name in ports {
                    let active = ws.tool == Tool::Power(name.to_owned());
                    let label = parts::power_label(name);
                    let r = row(ui, look, &label, name, active, |painter, rect| {
                        canvas::thumbnail_power(painter, rect, name, color(p.status_warning_fg));
                    });
                    if r.clicked() {
                        ws.tool = Tool::Power(name.to_owned());
                    }
                    r.dnd_set_drag_payload(Payload::Power(name.to_owned()));
                }
            }
            if !any {
                ui.label(
                    RichText::new(tr("No part matches."))
                        .font(eng::body())
                        .color(color(p.text_secondary)),
                );
            }
        });
}

/// One entry: a thumbnail, the name and the id, highlighted when it is the tool in hand.
fn row(
    ui: &mut Ui,
    look: &Look,
    label: &str,
    id: &str,
    active: bool,
    thumb: impl FnOnce(&egui::Painter, Rect),
) -> egui::Response {
    let p = look.palette;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    let fill = if active {
        Some(color(p.accent_subtle))
    } else if response.hovered() {
        Some(color(p.control_bg_hover))
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 8.0, fill);
    }
    let thumb_rect = Rect::from_min_size(rect.min + vec2(6.0, 4.0), vec2(44.0, ROW_H - 8.0));
    thumb(&ui.painter_at(thumb_rect), thumb_rect);
    let text_x = rect.left() + 60.0;
    ui.painter().text(
        egui::pos2(text_x, rect.center().y - 7.0),
        egui::Align2::LEFT_CENTER,
        label,
        eng::body(),
        color(if active {
            p.accent_text
        } else {
            p.text_primary
        }),
    );
    ui.painter().text(
        egui::pos2(text_x, rect.center().y + 9.0),
        egui::Align2::LEFT_CENTER,
        id,
        eng::caption(),
        color(p.text_secondary),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_cursor(egui::CursorIcon::Grab)
}

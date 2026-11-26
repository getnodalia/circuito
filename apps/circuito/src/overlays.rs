// SPDX-License-Identifier: MIT OR Apache-2.0
//! Everything that opens over the document: command search, Preferences, Keyboard shortcuts,
//! About and the question about unsaved changes. Each is a modal, closed with Escape, its close
//! button or a click outside it.

use circuito_app::commands::{Category, Cmd};
use circuito_app::i18n::{self, tr, trf, Lang};
use circuito_app::settings::Appearance;
use egui::{vec2, Align, Align2, Id, Key, Layout, RichText, Sense, Ui};
use nodalia_theme::about::{self, Info, Link};
use nodalia_theme::engineering::{self as eng, Look};
use nodalia_theme::for_egui::color;

use crate::app::{App, Overlay, Pending};
use crate::widgets::creator;

type Shown = (Overlay, bool);

fn modal(
    ctx: &egui::Context,
    look: &Look,
    id: &'static str,
    width: f32,
    top: bool,
    add: impl FnOnce(&mut Ui),
) -> bool {
    let p = &look.palette;
    let frame = eng::island(look)
        .inner_margin(16)
        .stroke(egui::Stroke::new(1.0, color(p.border_subtle)));
    let mut modal = egui::Modal::new(Id::unique(id)).frame(frame);
    if top {
        modal = modal.area(
            egui::Area::new(Id::unique(id))
                .kind(egui::UiKind::Modal)
                .anchor(Align2::CENTER_TOP, vec2(0.0, 96.0))
                .order(egui::Order::Foreground),
        );
    }
    let response = modal.show(ctx, |ui| {
        ui.set_width(width.min(ctx.content_rect().width() - 40.0));
        add(ui);
    });
    response.should_close()
}

pub fn show(ctx: &egui::Context, app: &mut App, look: &Look) {
    let overlay = std::mem::replace(&mut app.overlay, Overlay::None);
    let (next, close) = match overlay {
        Overlay::None => return,
        Overlay::Search(query) => search(ctx, app, look, query),
        Overlay::Preferences => preferences(ctx, app, look),
        Overlay::Shortcuts(filter) => shortcuts(ctx, look, filter),
        Overlay::About => about_window(ctx, app, look),
        Overlay::Unsaved(pending) => unsaved(ctx, app, look, pending),
    };
    // A command run from inside an overlay may have opened another one: keep that.
    if matches!(app.overlay, Overlay::None) && !close {
        app.overlay = next;
    }
}

fn heading(ui: &mut Ui, look: &Look, title: &str, id: &'static str) -> bool {
    about::heading(ui, &creator(look), title, Id::unique((id, "close"))).closed
}

fn section(ui: &mut Ui, look: &Look, title: &str) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(title)
            .font(eng::panel_title())
            .color(color(look.palette.text_secondary)),
    );
    ui.add_space(4.0);
}

// --- Command search -----------------------------------------------------------------------------

fn search(ctx: &egui::Context, app: &mut App, look: &Look, mut query: String) -> Shown {
    let mac = cfg!(target_os = "macos");
    let p = look.palette;
    let sel_id = Id::unique("circuito-search-selection");
    let mut sel: usize = ctx.data(|d| d.get_temp(sel_id)).unwrap_or(0);
    let needle = query.to_lowercase();
    let matches: Vec<Cmd> = Cmd::ALL
        .iter()
        .copied()
        .filter(|c| needle.is_empty() || c.translated().to_lowercase().contains(&needle))
        .collect();
    sel = sel.min(matches.len().saturating_sub(1));
    let (down, up, enter) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::Enter),
        )
    });
    if down && sel + 1 < matches.len() {
        sel += 1;
    }
    if up {
        sel = sel.saturating_sub(1);
    }
    let mut chosen = if enter {
        matches.get(sel).copied()
    } else {
        None
    };
    let dismissed = modal(ctx, look, "circuito-search", 560.0, true, |ui| {
        crate::widgets::field(look).show(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .frame(egui::Frame::NONE)
                    .font(eng::body())
                    .hint_text(tr("Type a command"))
                    .align(Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY),
            );
            if !response.has_focus() && !response.lost_focus() {
                response.request_focus();
            }
        });
        ui.add_space(8.0);
        if matches.is_empty() {
            ui.label(
                RichText::new(tr("No command matches."))
                    .font(eng::body())
                    .color(color(p.text_secondary)),
            );
        }
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .id_salt("search-scroll")
            .show(ui, |ui| {
                for (i, cmd) in matches.iter().enumerate() {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    if i == sel || response.hovered() {
                        ui.painter().rect_filled(rect, 7.0, color(p.accent_subtle));
                    }
                    let ink = if i == sel {
                        color(p.accent_text)
                    } else {
                        color(p.text_primary)
                    };
                    ui.painter().text(
                        rect.left_center() + vec2(10.0, 0.0),
                        Align2::LEFT_CENTER,
                        cmd.translated(),
                        eng::body(),
                        ink,
                    );
                    if let Some(s) = cmd.shortcut_label(mac) {
                        ui.painter().text(
                            rect.right_center() - vec2(10.0, 0.0),
                            Align2::RIGHT_CENTER,
                            s,
                            eng::value_font(),
                            color(p.text_secondary),
                        );
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, cmd.translated())
                    });
                    if response.clicked() {
                        chosen = Some(*cmd);
                    }
                }
            });
    });
    ctx.data_mut(|d| d.insert_temp(sel_id, sel));
    if let Some(cmd) = chosen {
        app.run(cmd);
        return (Overlay::None, true);
    }
    if dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::Search(query), false)
    }
}

// --- Preferences --------------------------------------------------------------------------------

fn preferences(ctx: &egui::Context, app: &mut App, look: &Look) -> Shown {
    let mut changed = false;
    let mut closed = false;
    let dismissed = modal(ctx, look, "circuito-preferences", 520.0, false, |ui| {
        closed = heading(ui, look, tr("Preferences"), "prefs");
        egui::ScrollArea::vertical()
            .id_salt("prefs-scroll")
            .auto_shrink([false, false])
            .min_scrolled_height((ui.ctx().content_rect().height() - 160.0).clamp(200.0, 470.0))
            .max_height((ui.ctx().content_rect().height() - 160.0).clamp(200.0, 470.0))
            .show(ui, |ui| {
                let s = &mut app.settings;
                section(ui, look, tr("Appearance"));
                ui.horizontal(|ui| {
                    for (a, label) in [
                        (Appearance::System, tr("System")),
                        (Appearance::Light, tr("Light")),
                        (Appearance::Dark, tr("Dark")),
                    ] {
                        if ui.selectable_label(s.appearance == a, label).clicked() {
                            s.appearance = a;
                            changed = true;
                        }
                    }
                });
                changed |= ui
                    .checkbox(&mut s.high_contrast, tr("High contrast"))
                    .changed();
                changed |= ui
                    .checkbox(&mut s.reduced_motion, tr("Reduce motion"))
                    .changed();
                if crate::chrome::NDL_CONTROLS_POSSIBLE {
                    changed |= ui
                        .checkbox(
                            &mut s.native_window_controls,
                            tr("Use the desktop's own window buttons"),
                        )
                        .changed();
                }

                section(ui, look, tr("Language"));
                ui.horizontal(|ui| {
                    for (code, label) in
                        [("auto", tr("System")), ("es", "Español"), ("en", "English")]
                    {
                        if ui.selectable_label(s.language == code, label).clicked() {
                            s.language = code.to_owned();
                            i18n::set(Lang::from_setting(code));
                            changed = true;
                        }
                    }
                });

                section(ui, look, tr("Simulation"));
                changed |= ui
                    .checkbox(
                        &mut s.auto_run,
                        tr("Run the analyses when a circuit opens or changes"),
                    )
                    .changed();
                ui.label(
                    RichText::new(tr(
                        "Analyses run on a worker thread, so the window stays responsive.",
                    ))
                    .font(eng::caption())
                    .color(color(look.palette.text_tertiary)),
                );
                section(ui, look, tr("Drawing"));
                changed |= ui.checkbox(&mut s.show_grid, tr("Show the grid")).changed();
            });
    });
    if changed {
        app.save_settings();
    }
    if closed || dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::Preferences, false)
    }
}

// --- Keyboard shortcuts -------------------------------------------------------------------------

fn shortcuts(ctx: &egui::Context, look: &Look, mut filter: String) -> Shown {
    let mac = cfg!(target_os = "macos");
    let p = look.palette;
    let mut closed = false;
    let dismissed = modal(ctx, look, "circuito-shortcuts", 560.0, false, |ui| {
        closed = heading(ui, look, tr("Keyboard shortcuts"), "keys");
        ui.add_space(8.0);
        crate::widgets::field(look).show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .frame(egui::Frame::NONE)
                    .font(eng::body())
                    .hint_text(tr("Filter commands…"))
                    .align(Align2::LEFT_CENTER)
                    .desired_width(f32::INFINITY),
            );
        });
        ui.add_space(8.0);
        let needle = filter.to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("keys-scroll")
            .auto_shrink([false, false])
            .min_scrolled_height((ui.ctx().content_rect().height() - 240.0).clamp(200.0, 520.0))
            .max_height((ui.ctx().content_rect().height() - 240.0).clamp(200.0, 520.0))
            .show(ui, |ui| {
                for category in Category::ALL {
                    let rows: Vec<Cmd> = Cmd::ALL
                        .iter()
                        .copied()
                        .filter(|c| c.category() == category)
                        .filter(|c| {
                            needle.is_empty() || c.translated().to_lowercase().contains(&needle)
                        })
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    section(ui, look, tr(category.label()));
                    for cmd in rows {
                        ui.horizontal(|ui| {
                            // Leave room for the floating scroll bar on the right.
                            ui.set_max_width(ui.available_width() - 14.0);
                            ui.label(
                                RichText::new(cmd.translated())
                                    .font(eng::body())
                                    .color(color(p.text_primary)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if let Some(s) = cmd.shortcut_label(mac) {
                                    ui.label(
                                        RichText::new(s)
                                            .font(eng::value_font())
                                            .color(color(p.text_secondary)),
                                    );
                                }
                            });
                        });
                    }
                }
            });
    });
    if closed || dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::Shortcuts(filter), false)
    }
}

// --- About --------------------------------------------------------------------------------------

fn about_window(ctx: &egui::Context, app: &mut App, look: &Look) -> Shown {
    let version = trf("Version {}", &[env!("CARGO_PKG_VERSION")]);
    let links = [
        Link {
            label: tr("Website"),
            url: "https://getnodalia.com/circuito",
        },
        Link {
            label: tr("Source code"),
            url: "https://github.com/getnodalia/circuito",
        },
        Link {
            label: tr("Report a problem"),
            url: "https://github.com/getnodalia/circuito/issues",
        },
    ];
    let footer = format!(
        "MIT OR Apache-2.0 · © Nodalia · {}",
        tr("Icon: original work.")
    );
    let info = Info {
        name: circuito_app::NAME,
        version: &version,
        tagline: tr("Open and check schematics, then simulate them: operating point, AC, transient and DC sweeps."),
        suite: tr("Part of Nodalia · Engineering"),
        links: &links,
        footer: &footer,
    };
    let texture = app.icon(ctx);
    let mut closed = false;
    let dismissed = modal(
        ctx,
        look,
        "circuito-about",
        about::measures::WIDTH,
        false,
        |ui| {
            closed = heading(ui, look, tr("About Circuito"), "about");
            let page = about::page(ui, &creator(look), &info, |ui, rect| {
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            });
            if let Some(link) = page.picked.and_then(|i| links.get(i)) {
                ui.ctx().open_url(egui::OpenUrl::new_tab(link.url));
            }
        },
    );
    if closed || dismissed {
        (Overlay::None, true)
    } else {
        (Overlay::About, false)
    }
}

// --- Unsaved changes ----------------------------------------------------------------------------

fn unsaved(ctx: &egui::Context, app: &mut App, look: &Look, pending: Pending) -> Shown {
    let p = look.palette;
    let names: Vec<(u64, String)> = app
        .ws
        .docs()
        .iter()
        .filter(|d| d.dirty())
        .map(|d| (d.id, d.circuit.name()))
        .collect();
    let target = match pending {
        Pending::Close(id) => names.iter().find(|(i, _)| *i == id).cloned(),
        Pending::Quit => None,
    };
    // Nothing is left to ask about.
    if names.is_empty() || (matches!(pending, Pending::Close(_)) && target.is_none()) {
        if let Pending::Close(id) = pending {
            app.request_close(id);
        }
        return (Overlay::None, true);
    }
    enum Answer {
        Cancel,
        Discard,
        Save,
    }
    let mut answer: Option<Answer> = None;
    let dismissed = modal(ctx, look, "circuito-unsaved", 440.0, false, |ui| {
        let title = match &target {
            Some((_, name)) => trf("Save the changes to {}?", &[name]),
            None => tr("Quit with unsaved changes?").to_owned(),
        };
        ui.label(
            RichText::new(title)
                .font(eng::panel_title())
                .color(color(p.text_primary)),
        );
        ui.add_space(8.0);
        let body = match &target {
            Some(_) => tr("Your changes are lost if you do not save them.").to_owned(),
            None => trf(
                "These circuits have changes that are not saved: {}",
                &[&names
                    .iter()
                    .map(|(_, n)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")],
            ),
        };
        ui.label(
            RichText::new(body)
                .font(eng::body())
                .color(color(p.text_secondary)),
        );
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if target.is_some() && ui.button(tr("Save")).clicked() {
                answer = Some(Answer::Save);
            }
            if ui
                .button(if target.is_some() {
                    tr("Discard")
                } else {
                    tr("Quit without saving")
                })
                .clicked()
            {
                answer = Some(Answer::Discard);
            }
            if ui.button(tr("Cancel")).clicked() {
                answer = Some(Answer::Cancel);
            }
        });
    });
    match answer {
        Some(Answer::Save) => {
            if let Pending::Close(id) = pending {
                app.save_then_close(id);
            }
            (Overlay::None, true)
        }
        Some(Answer::Discard) => {
            app.discard(pending);
            (Overlay::None, true)
        }
        Some(Answer::Cancel) => (Overlay::None, true),
        None if dismissed => (Overlay::None, true),
        None => (Overlay::Unsaved(pending), false),
    }
}

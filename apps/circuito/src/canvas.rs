// SPDX-License-Identifier: MIT OR Apache-2.0
//! The schematic canvas: the sheet drawn from the library's symbols, panned with a drag and
//! zoomed with the wheel or a pinch. With the select tool a press picks, moves or rubber-bands;
//! the other tools place parts, wires, labels and ports on the grid. What the rules check points
//! at is ringed in the danger colour. Every change goes through the open circuit's editing
//! methods, so it can be undone.

use circuito_app::commands::Cmd;
use circuito_app::editing::{Gesture, Ghost, Tool};
use circuito_app::geometry::{self, Selection, View};
use circuito_app::hit;
use circuito_app::i18n::tr;
use circuito_app::parts::Payload;
use circuito_app::workspace::Open;
use circuito_core::edit::{self, Item, Items};
use circuito_core::library::{Library, PartDef, Shape};
use circuito_core::schematic::{LabelScope, Part, Point, PowerStyle, Sheet};
use egui::{
    pos2, vec2, Align2, Color32, FontFamily, FontId, Key, PointerButton, Pos2, Rect, Sense, Stroke,
    Ui, UiBuilder,
};
use nodalia_theme::engineering::{self as eng, menu, Look, ViewItem};
use nodalia_theme::for_egui::color;

/// What the person did on the canvas this frame.
#[derive(Default)]
pub struct Out {
    /// Where the pointer is, in grid units.
    pub hover: Option<(f32, f32)>,
    pub commands: Vec<Cmd>,
    /// The tool to go back to (Escape or a right click ends the current one).
    pub tool: Option<Tool>,
}

/// What the drawing needs to know about the pen: the tool, how a part would be turned and whether
/// keys belong to the drawing right now (no dialog or text field has them).
pub struct Pen<'a> {
    pub tool: &'a Tool,
    pub ghost: Ghost,
    pub keys: bool,
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

    fn world(&self, at: Pos2) -> (f32, f32) {
        self.view
            .to_world((at.x - self.origin.x, at.y - self.origin.y), self.size)
    }
}

fn payload_tool(p: &Payload) -> Tool {
    match p {
        Payload::Part(id) => Tool::Part((*id).to_owned()),
        Payload::Power(name) => Tool::Power(name.clone()),
    }
}

/// Place what `tool` places at `at`. Returns whether the drawing changed.
fn place(doc: &mut Open, tool: &Tool, ghost: Ghost, at: Point) -> bool {
    match tool {
        Tool::Part(id) => doc.place_part(id, at, ghost.rotation(), ghost.mirror),
        Tool::Power(name) => doc.place_power(name, at),
        Tool::Label => doc.place_label(at),
        Tool::NoConnect => doc.place_no_connect(at),
        Tool::Select | Tool::Wire => false,
    }
}

/// Pick what is under a click: shift adds it to or takes it from the selection.
fn pick(doc: &mut Open, world: (f32, f32), shift: bool) {
    let hit = doc.current_sheet().and_then(|s| hit::item_at(s, world));
    let mut items = if shift {
        doc.items.clone()
    } else {
        Items::new()
    };
    if let Some(item) = hit {
        if !items.remove(&item) {
            items.insert(item);
        }
    }
    doc.set_items(items);
}

/// The pointer started dragging at `origin`: what that grabs.
fn begin_drag(doc: &mut Open, tool: &Tool, origin: (f32, f32), shift: bool) {
    doc.end_gesture();
    let pin = doc
        .current_sheet()
        .and_then(|s| hit::pin_at(s, origin, hit::PIN_SLACK));
    match tool {
        Tool::Wire => {
            doc.gesture = Gesture::Wire {
                from: pin.unwrap_or_else(|| hit::snap(origin)),
                flip: false,
                dragging: true,
            };
        }
        Tool::Select => {
            if let Some(from) = pin {
                doc.gesture = Gesture::Wire {
                    from,
                    flip: false,
                    dragging: true,
                };
                return;
            }
            let hit = doc.current_sheet().and_then(|s| hit::item_at(s, origin));
            match hit {
                Some(item) => {
                    if !doc.items.contains(&item) {
                        let mut items = if shift {
                            doc.items.clone()
                        } else {
                            Items::new()
                        };
                        items.insert(item);
                        doc.set_items(items);
                    }
                    doc.gesture = Gesture::Move {
                        last: hit::snap(origin),
                    };
                }
                None => {
                    if !shift {
                        doc.set_items(Items::new());
                    }
                    doc.gesture = Gesture::Band { from: origin };
                }
            }
        }
        _ => {}
    }
}

/// The pointer was released after a drag: finish what it was doing.
fn end_drag(doc: &mut Open, at: (f32, f32), shift: bool) {
    match std::mem::take(&mut doc.gesture) {
        Gesture::Move { .. } => doc.end_gesture(),
        Gesture::Band { from } => {
            let inside = doc
                .current_sheet()
                .map(|s| hit::items_in(s, from, at))
                .unwrap_or_default();
            let mut items = if shift {
                doc.items.clone()
            } else {
                Items::new()
            };
            items.extend(inside);
            doc.set_items(items);
        }
        Gesture::Wire {
            from,
            flip,
            dragging: true,
        } => {
            let to = hit::snap(at);
            doc.add_wire(&edit::route(from, to, !flip));
        }
        other => doc.gesture = other,
    }
}

/// A click while drawing wires: start a run, or add to it.
fn wire_click(doc: &mut Open, at: Point) {
    match doc.gesture.clone() {
        Gesture::Wire { from, flip, .. } => {
            if at == from {
                doc.gesture = Gesture::Idle;
                return;
            }
            // Ending on something that connects finishes the run; ending in the open goes on.
            let lands = doc
                .current_sheet()
                .is_some_and(|s| hit::is_connection(s, at));
            doc.add_wire(&edit::route(from, at, !flip));
            doc.gesture = if lands {
                Gesture::Idle
            } else {
                Gesture::Wire {
                    from: at,
                    flip: false,
                    dragging: false,
                }
            };
        }
        _ => {
            doc.gesture = Gesture::Wire {
                from: at,
                flip: false,
                dragging: false,
            };
        }
    }
}

pub fn show(ui: &mut Ui, look: &Look, doc: &mut Open, grid: bool, pen: &Pen) -> Out {
    let p = &look.palette;
    let mut out = Out::default();
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, color(p.surface_sunken));
    let size = (rect.width().max(1.0), rect.height().max(1.0));
    if doc.current_sheet().is_none() {
        return out;
    }

    if doc.fit_pending {
        if let Some(bounds) = doc.current_sheet().and_then(geometry::sheet_bounds) {
            doc.view = View::fit(bounds, size, 56.0);
        }
        doc.fit_pending = false;
    }

    let (shift, space) = ui.input(|i| (i.modifiers.shift, i.key_down(Key::Space)));
    // Pan and zoom first, so what is drawn is where the pointer now is.
    let panning = response.dragged_by(PointerButton::Middle)
        || response.dragged_by(PointerButton::Secondary)
        || (space && response.dragged_by(PointerButton::Primary));
    if panning {
        let d = response.drag_delta();
        doc.view = doc.view.panned((d.x, d.y));
    }
    if response.hovered() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = pinch * (scroll * 0.0015).exp();
        if (factor - 1.0).abs() > 1e-4 {
            if let Some(at) = response.hover_pos() {
                doc.view =
                    doc.view
                        .zoomed_at(factor, (at.x - rect.left(), at.y - rect.top()), size);
            }
        }
    }
    let frame = Frame {
        origin: rect.min,
        size,
        view: doc.view,
    };
    let dropped = response.dnd_release_payload::<Payload>();
    let dragged_in = response.dnd_hover_payload::<Payload>();
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let hover_world = response
        .hover_pos()
        .or_else(|| dragged_in.as_ref().and(pointer))
        .map(|at| frame.world(at));
    out.hover = hover_world;

    // What the person did this frame.
    let primary = PointerButton::Primary;
    if !panning {
        if response.drag_started_by(primary) {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                begin_drag(doc, pen.tool, frame.world(origin), shift);
            }
        }
        if response.dragged_by(primary) {
            if let (Gesture::Move { last }, Some(at)) = (doc.gesture.clone(), pointer) {
                let now = hit::snap(frame.world(at));
                doc.move_selection(now.x - last.x, now.y - last.y);
                doc.gesture = Gesture::Move { last: now };
            }
        }
        if response.drag_stopped_by(primary) {
            if let Some(at) = pointer {
                end_drag(doc, frame.world(at), shift);
            }
        }
        if response.clicked_by(primary) {
            if let Some(at) = response.interact_pointer_pos() {
                let world = frame.world(at);
                let snapped = hit::snap(world);
                match pen.tool {
                    Tool::Select => pick(doc, world, shift),
                    Tool::Wire => wire_click(doc, snapped),
                    other => {
                        place(doc, other, pen.ghost, snapped);
                    }
                }
            }
        }
    }
    if let (Some(payload), Some(at)) = (dropped, pointer) {
        let snapped = hit::snap(frame.world(at));
        place(doc, &payload_tool(&payload), pen.ghost, snapped);
    }
    if response.secondary_clicked() && doc.gesture != Gesture::Idle {
        doc.gesture = Gesture::Idle;
    } else if response.secondary_clicked() && *pen.tool != Tool::Select {
        out.tool = Some(Tool::Select);
    }
    if pen.keys {
        keys(ui, doc, pen, &mut out);
    }
    if *pen.tool == Tool::Select && doc.gesture == Gesture::Idle {
        // A right click acts on what is under it, as in every editor: pick it first unless it is
        // already part of the selection, so the menu's Cut, Rotate and Delete mean that thing.
        if response.secondary_clicked() && !panning {
            if let Some(at) = pointer {
                let under = doc
                    .current_sheet()
                    .and_then(|sh| hit::item_at(sh, frame.world(at)));
                match under {
                    Some(item) if !doc.items.contains(&item) => {
                        doc.set_items(std::iter::once(item).collect());
                    }
                    None => doc.set_items(Items::new()),
                    Some(_) => {}
                }
            }
        }
        let mut commands = Vec::new();
        response.context_menu(|ui| context_menu(ui, look, doc, &mut commands));
        out.commands.extend(commands);
    }

    // The cursor says what a press would do here.
    let pin_here = hover_world.and_then(|w| {
        doc.current_sheet()
            .and_then(|s| hit::pin_at(s, w, hit::PIN_SLACK))
    });
    if hover_world.is_some() && !dragged_in.is_some() {
        let icon = if panning {
            egui::CursorIcon::Grabbing
        } else if !matches!(doc.gesture, Gesture::Idle) {
            match doc.gesture {
                Gesture::Move { .. } => egui::CursorIcon::Move,
                _ => egui::CursorIcon::Crosshair,
            }
        } else if pin_here.is_some() || *pen.tool != Tool::Select {
            egui::CursorIcon::Crosshair
        } else if hover_world
            .and_then(|w| doc.current_sheet().and_then(|s| hit::item_at(s, w)))
            .is_some()
        {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        };
        ui.ctx().set_cursor_icon(icon);
    }

    // Drawing, with the drawing as it now is.
    let Some(sheet_data) = doc.current_sheet() else {
        return out;
    };
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
        sheet_data,
        &doc.items,
        doc.selection,
        doc.finding.is_some(),
        look,
    );
    draw_sheet(&painter, &frame, sheet_data, &ink);

    if geometry::sheet_bounds(sheet_data).is_none() && doc.gesture == Gesture::Idle {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            if *pen.tool == Tool::Select {
                tr("Drag a part from the list on the left, or write the circuit in the code pane.")
            } else {
                tr("Click to place.")
            },
            eng::body(),
            color(p.text_secondary),
        );
    }

    let accent = color(p.accent_default);
    let snapped = hover_world.map(hit::snap);
    // A pin or wire end the pointer is about to use.
    if let Some(pin) = pin_here.filter(|_| matches!(pen.tool, Tool::Select | Tool::Wire)) {
        let at = frame.at((pin.x as f32, pin.y as f32));
        painter.circle_stroke(
            at,
            (frame.view.zoom * 0.6).clamp(5.0, 12.0),
            Stroke::new(2.0, accent),
        );
    }
    // What would be placed, drawn faintly where it would land.
    let placing = match (&dragged_in, pen.tool) {
        (Some(payload), _) => Some(payload_tool(payload)),
        (None, tool @ (Tool::Part(_) | Tool::Power(_) | Tool::Label | Tool::NoConnect)) => {
            Some(tool.clone())
        }
        _ => None,
    };
    if let (Some(tool), Some(at), true) = (placing, snapped, hover_world.is_some()) {
        draw_ghost(&painter, &frame, &tool, pen.ghost, at, &ink, look);
    }
    // The wire being drawn.
    if let Gesture::Wire { from, flip, .. } = &doc.gesture {
        let to = snapped.unwrap_or(*from);
        let points: Vec<Pos2> = edit::route(*from, to, !*flip)
            .iter()
            .map(|q| frame.at((q.x as f32, q.y as f32)))
            .collect();
        if points.len() >= 2 {
            painter.add(egui::Shape::line(points, Stroke::new(2.0, accent)));
        }
        let start = frame.at((from.x as f32, from.y as f32));
        painter.circle_filled(start, 4.0, accent);
        let end = frame.at((to.x as f32, to.y as f32));
        painter.circle_stroke(end, 5.0, Stroke::new(1.5, accent));
    }
    if let (Gesture::Band { from }, Some(at)) = (&doc.gesture, pointer) {
        let band = Rect::from_two_pos(frame.at(*from), at);
        painter.rect_filled(band, 2.0, color(p.selection_bg));
        painter.rect_stroke(
            band,
            2.0,
            Stroke::new(1.0, accent),
            egui::StrokeKind::Inside,
        );
    }

    view_bar(ui, look, rect, grid, &mut out);
    out
}

/// Keys that belong to the drawing itself.
fn keys(ui: &Ui, doc: &mut Open, pen: &Pen, out: &mut Out) {
    let (escape, space, arrows, released) = ui.input(|i| {
        let step = |k: Key| i.key_pressed(k);
        (
            i.key_pressed(Key::Escape),
            step(Key::Space),
            (
                i32::from(step(Key::ArrowRight)) - i32::from(step(Key::ArrowLeft)),
                i32::from(step(Key::ArrowDown)) - i32::from(step(Key::ArrowUp)),
            ),
            [
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::ArrowUp,
                Key::ArrowDown,
            ]
            .iter()
            .any(|k| i.key_released(*k)),
        )
    });
    if escape {
        if doc.gesture != Gesture::Idle {
            doc.gesture = Gesture::Idle;
        } else if *pen.tool != Tool::Select {
            out.tool = Some(Tool::Select);
        } else if !doc.items.is_empty() {
            doc.set_items(Items::new());
        }
    }
    if space {
        if let Gesture::Wire { flip, .. } = &mut doc.gesture {
            *flip = !*flip;
        }
    }
    if arrows != (0, 0) {
        if doc.items.is_empty() {
            // Nothing selected: the arrows move the view instead.
            doc.view = doc
                .view
                .panned((-arrows.0 as f32 * 40.0, -arrows.1 as f32 * 40.0));
        } else if matches!(doc.gesture, Gesture::Idle) {
            doc.move_selection(arrows.0, arrows.1);
        }
    }
    if released {
        doc.end_gesture();
    }
}

fn context_menu(ui: &mut Ui, look: &Look, doc: &Open, commands: &mut Vec<Cmd>) {
    let mac = cfg!(target_os = "macos");
    ui.set_min_width(menu::metric::MIN_WIDTH);
    let has = !doc.items.is_empty();
    let rows: [(Cmd, bool); 9] = [
        (Cmd::Cut, has),
        (Cmd::Copy, has),
        (Cmd::Paste, true),
        (Cmd::Duplicate, has),
        (Cmd::Delete, has),
        (Cmd::RotateRight, has),
        (Cmd::MirrorHorizontal, has),
        (Cmd::SelectAll, true),
        (Cmd::FitView, true),
    ];
    for (i, (cmd, enabled)) in rows.into_iter().enumerate() {
        if i == 5 || i == 7 {
            menu::separator(ui, look);
        }
        let mut row = menu::Item::new(cmd.translated()).gutter();
        if let Some(icon) = cmd.icon() {
            row = row.icon(icon);
        }
        let shortcut = cmd.shortcut_label(mac);
        if let Some(s) = &shortcut {
            row = row.shortcut(s);
        }
        if !enabled {
            row = row.enabled(false);
        }
        if menu::item(ui, look, &row).clicked() && enabled {
            commands.push(cmd);
            ui.close();
        }
    }
}

/// The view tools float over the drawing.
fn view_bar(ui: &mut Ui, look: &Look, rect: Rect, grid: bool, out: &mut Out) {
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
}

/// What a tool would place, drawn faintly at `at`.
fn draw_ghost(
    painter: &egui::Painter,
    frame: &Frame,
    tool: &Tool,
    ghost: Ghost,
    at: Point,
    ink: &Ink,
    look: &Look,
) {
    let faint = |c: Color32| c.gamma_multiply(0.55);
    let ink = Ink {
        part: faint(ink.part),
        wire: faint(ink.wire),
        label: faint(ink.label),
        power: faint(ink.power),
        refdes: faint(ink.refdes),
        value: faint(ink.value),
        mark: faint(ink.mark),
    };
    let zoom = frame.view.zoom;
    let line = (zoom * 0.13).clamp(1.1, 2.4);
    let font = FontId::new((zoom * 0.8).clamp(8.0, 15.0), FontFamily::Monospace);
    match tool {
        Tool::Part(id) => {
            let Some(def) = Library::builtin().get(id) else {
                return;
            };
            let mut part = edit::new_part(def, def.prefix, at);
            part.rotation = ghost.rotation();
            part.mirror = ghost.mirror;
            draw_part(painter, frame, &part, def, &ink, line, zoom >= 4.0, &font);
            for pin in edit::pin_points(&part) {
                painter.circle_filled(
                    frame.at((pin.x as f32, pin.y as f32)),
                    (zoom * 0.2).clamp(2.0, 4.0),
                    color(look.palette.accent_default),
                );
            }
        }
        Tool::Power(name) => {
            let port = edit::new_power_port(name, at);
            draw_power(
                painter,
                frame,
                at.x,
                at.y,
                port.style,
                name,
                &ink,
                line,
                zoom >= 4.0,
                &font,
            );
        }
        Tool::Label => {
            let a = frame.at((at.x as f32, at.y as f32));
            painter.line_segment([a, a + vec2(zoom * 3.0, 0.0)], Stroke::new(line, ink.label));
        }
        Tool::NoConnect => {
            let a = frame.at((at.x as f32, at.y as f32));
            let d = (zoom * 0.45).clamp(3.0, 9.0);
            let stroke = Stroke::new(line, ink.mark);
            painter.line_segment([a + vec2(-d, -d), a + vec2(d, d)], stroke);
            painter.line_segment([a + vec2(-d, d), a + vec2(d, -d)], stroke);
        }
        Tool::Select | Tool::Wire => {}
    }
}

/// A part's symbol drawn to fill `rect`, for the palette.
pub fn thumbnail(painter: &egui::Painter, rect: Rect, lib_id: &str, ink: Color32) {
    let Some(def) = Library::builtin().get(lib_id) else {
        return;
    };
    let part = edit::new_part(def, def.prefix, Point::new(0, 0));
    let Some(bounds) = geometry::part_bounds(&part) else {
        return;
    };
    let size = (rect.width().max(1.0), rect.height().max(1.0));
    let frame = Frame {
        origin: rect.min,
        size,
        view: View::fit(bounds, size, 3.0),
    };
    let ink = Ink {
        part: ink,
        wire: ink,
        label: ink,
        power: ink,
        refdes: ink,
        value: ink,
        mark: ink,
    };
    let font = FontId::new(8.0, FontFamily::Monospace);
    draw_part(painter, &frame, &part, def, &ink, 1.4, false, &font);
}

/// A power port's symbol drawn to fill `rect`, for the palette.
pub fn thumbnail_power(painter: &egui::Painter, rect: Rect, name: &str, ink: Color32) {
    let port = edit::new_power_port(name, Point::new(0, 0));
    let size = (rect.width().max(1.0), rect.height().max(1.0));
    let zoom = (size.1 / 5.0).min(size.0 / 3.0).max(1.0);
    let centre = match port.style {
        PowerStyle::Ground => (0.0, 1.8),
        PowerStyle::Supply => (0.0, -0.8),
    };
    let frame = Frame {
        origin: rect.min,
        size,
        view: View { centre, zoom },
    };
    let ink = Ink {
        part: ink,
        wire: ink,
        label: ink,
        power: ink,
        refdes: ink,
        value: ink,
        mark: ink,
    };
    let font = FontId::new(8.0, FontFamily::Monospace);
    draw_power(
        painter, &frame, 0, 0, port.style, name, &ink, 1.4, false, &font,
    );
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
    sheet: &Sheet,
    items: &Items,
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
    let boxed = |b: geometry::Bounds| {
        let r = Rect::from_two_pos(frame.at(b.expanded(0.8).min), frame.at(b.expanded(0.8).max));
        painter.rect_filled(r, 6.0, fill);
        painter.rect_stroke(r, 6.0, Stroke::new(1.5, stroke), egui::StrokeKind::Inside);
    };
    for item in items {
        if let Item::Wire(i) = item {
            let Some(w) = sheet.wires.get(*i) else {
                continue;
            };
            let pts: Vec<Pos2> = w
                .points
                .iter()
                .map(|q| frame.at((q.x as f32, q.y as f32)))
                .collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(
                    pts,
                    Stroke::new(5.0, stroke.gamma_multiply(0.5)),
                ));
            }
        } else if let Some(b) = hit::item_bounds(sheet, *item) {
            boxed(b);
        }
    }
    // A finding points at a spot as well as at a part.
    if let Some(Selection::Spot { x, y, .. }) = selection {
        let at = frame.at((x as f32, y as f32));
        let r = (1.6 * frame.view.zoom).max(8.0);
        painter.circle_filled(at, r, fill);
        painter.circle_stroke(at, r, Stroke::new(1.5, stroke));
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

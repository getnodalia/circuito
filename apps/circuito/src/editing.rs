// SPDX-License-Identifier: MIT OR Apache-2.0
//! Changing the drawing: what each tool and each field of the properties panel does to the open
//! circuit, with undo and redo, the selection, and the text form kept in step. No window is
//! needed; the canvas and the panels only call these.

use circuito_core::edit::{self, Clip, Item, Items, RenameError};
use circuito_core::library::Library;
use circuito_core::netlist::Analysis;
use circuito_core::schematic::{Label, LabelScope, NoConnect, Point, Rotation, Schematic, Sheet};
use circuito_script::{apply, design_of, parse, print, PrintOptions};

use crate::geometry::Selection;
use crate::i18n::{tr, trf};
use crate::workspace::{Open, RunState};

/// What a click on the drawing does.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Tool {
    /// Pick, move and draw wires from pins.
    #[default]
    Select,
    /// Draw wires.
    Wire,
    /// Place the library part with this id.
    Part(String),
    /// Place a power port of this name.
    Power(String),
    /// Place a net label.
    Label,
    /// Place a no-connect flag.
    NoConnect,
}

/// What the pointer is in the middle of on the drawing.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Gesture {
    #[default]
    Idle,
    /// Dragging the selection; `last` is the grid point it was moved to.
    Move { last: Point },
    /// A rubber band from this point (grid units) to the pointer.
    Band { from: (f32, f32) },
    /// A wire from `from` towards the pointer. `dragging` is a press and drag; otherwise the next
    /// click ends the run.
    Wire {
        from: Point,
        flip: bool,
        dragging: bool,
    },
}

/// The turns and mirroring a part about to be placed has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ghost {
    /// Quarter turns clockwise, 0 to 3.
    pub turns: u8,
    pub mirror: bool,
}

impl Ghost {
    /// The turn another quarter round (the other way with `counter_clockwise`).
    #[must_use]
    pub fn turned(self, counter_clockwise: bool) -> Ghost {
        let turns = if counter_clockwise {
            (self.turns + 3) % 4
        } else {
            (self.turns + 1) % 4
        };
        Ghost { turns, ..self }
    }

    pub fn rotation(self) -> Rotation {
        match self.turns % 4 {
            1 => Rotation::R90,
            2 => Rotation::R180,
            3 => Rotation::R270,
            _ => Rotation::R0,
        }
    }
}

/// A change made from the properties panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The selected part's reference designator.
    Refdes(String),
    /// The selected part's value.
    Value(String),
    /// The selected part's model (`None` removes it).
    Model(Option<String>),
    /// A field of the selected part.
    Field(String, String),
    /// Remove a field of the selected part.
    RemoveField(String),
    /// The selected label's name.
    LabelName(String),
    /// Whether the selected label is local or global.
    LabelScope(LabelScope),
    /// The selected power port's net.
    PortName(String),
    /// The circuit's title.
    Title(String),
    /// Change analysis `n`, or add one (`None`), from its text (`.ac dec 20 10 100k`).
    Analysis(Option<usize>, String),
    /// Remove analysis `n`.
    RemoveAnalysis(usize),
}

/// How far a pasted copy lands from the original, in grid units.
const PASTE_OFFSET: i32 = 4;

impl Open {
    /// Whether this circuit can be drawn on.
    pub fn editable(&self) -> bool {
        self.circuit.editable()
    }

    /// Whether the circuit has changes the file does not have.
    pub fn dirty(&self) -> bool {
        self.rev != self.saved_rev
    }

    /// Whether the text form is shown for this circuit (one sheet only).
    pub fn script_available(&self) -> bool {
        self.circuit
            .schematic
            .as_ref()
            .is_some_and(|s| s.sheets.len() == 1)
    }

    fn sheet_ref(&self) -> Option<&Sheet> {
        self.circuit.schematic.as_ref()?.sheets.get(self.sheet)
    }

    /// A copy of the selected sheet, for what needs to look without holding the circuit.
    pub fn current_sheet(&self) -> Option<&Sheet> {
        self.sheet_ref()
    }

    /// Select exactly these items.
    pub fn set_items(&mut self, items: Items) {
        self.selection = match items.iter().collect::<Vec<_>>().as_slice() {
            [Item::Part(part)] => Some(Selection::Part {
                sheet: self.sheet,
                part: *part,
            }),
            _ => None,
        };
        self.finding = None;
        self.items = items;
    }

    /// Select everything on the sheet.
    pub fn select_all(&mut self) {
        let mut items = Items::new();
        if let Some(s) = self.sheet_ref() {
            items.extend((0..s.parts.len()).map(Item::Part));
            items.extend((0..s.wires.len()).map(Item::Wire));
            items.extend((0..s.labels.len()).map(Item::Label));
            items.extend((0..s.power_ports.len()).map(Item::Port));
            items.extend((0..s.no_connects.len()).map(Item::NoConnect));
            items.extend((0..s.junctions.len()).map(Item::Junction));
        }
        self.set_items(items);
    }

    fn touched(&mut self) {
        self.rev += 1;
        self.script.regenerate = true;
    }

    /// Change the shown sheet with `f`, as one step that can be undone. A change that leaves
    /// the drawing as it was leaves no step. `merge` joins a run of the same kind of change
    /// (nudging, dragging) into one step.
    pub fn edit<R>(
        &mut self,
        label: &str,
        merge: Option<&str>,
        f: impl FnOnce(&mut Schematic, usize, &mut Items) -> R,
    ) -> Option<R> {
        let sheet = self.sheet;
        let s = self.circuit.schematic.as_mut()?;
        if sheet >= s.sheets.len() {
            return None;
        }
        let before = s.clone();
        let mut items = std::mem::take(&mut self.items);
        let out = f(s, sheet, &mut items);
        if *s == before {
            self.items = items;
            return Some(out);
        }
        match merge {
            Some(key) => self.history.record_merging(key, before, label),
            None => self.history.record(before, label),
        }
        self.touched();
        self.set_items(items);
        Some(out)
    }

    /// The end of a gesture: the next change starts a step of its own.
    pub fn end_gesture(&mut self) {
        self.history.close_run();
    }

    /// Go back one step. Returns what was undone.
    pub fn undo(&mut self) -> Option<String> {
        let current = self.circuit.schematic.clone()?;
        let (state, label) = self.history.undo(current)?;
        self.restore(state);
        Some(label)
    }

    /// Go forward one step. Returns what was redone.
    pub fn redo(&mut self) -> Option<String> {
        let current = self.circuit.schematic.clone()?;
        let (state, label) = self.history.redo(current)?;
        self.restore(state);
        Some(label)
    }

    fn restore(&mut self, state: Schematic) {
        let n = state.sheets.len();
        self.circuit.schematic = Some(state);
        self.sheet = self.sheet.min(n.saturating_sub(1));
        self.set_items(Items::new());
        self.touched();
    }

    // --- What the tools do --------------------------------------------------------------------

    /// Place a library part with its origin at `at`.
    pub fn place_part(
        &mut self,
        lib_id: &str,
        at: Point,
        rotation: Rotation,
        mirror: bool,
    ) -> bool {
        let Some(def) = Library::builtin().get(lib_id) else {
            return false;
        };
        self.edit("Place a part", None, |s, sheet, items| {
            if sheet >= s.sheets.len() {
                return false;
            }
            let refdes = edit::next_refdes(s, def.prefix);
            let mut part = edit::new_part(def, &refdes, at);
            part.rotation = rotation;
            part.mirror = mirror;
            let Some(sh) = s.sheets.get_mut(sheet) else {
                return false;
            };
            sh.parts.push(part);
            let index = sh.parts.len() - 1;
            edit::normalize(sh);
            *items = Items::from([Item::Part(index)]);
            true
        })
        .unwrap_or(false)
    }

    /// Place a power port (ground for `GND`) at `at`.
    pub fn place_power(&mut self, name: &str, at: Point) -> bool {
        self.edit("Place a power port", None, |s, sheet, items| {
            let Some(sh) = s.sheets.get_mut(sheet) else {
                return false;
            };
            sh.power_ports.push(edit::new_power_port(name, at));
            let index = sh.power_ports.len() - 1;
            edit::normalize(sh);
            *items = Items::from([Item::Port(index)]);
            true
        })
        .unwrap_or(false)
    }

    /// Place a net label at `at`, named `n1`, `n2`… (the first not used) until it is renamed.
    pub fn place_label(&mut self, at: Point) -> bool {
        self.edit("Place a net label", None, |s, sheet, items| {
            let Some(sh) = s.sheets.get_mut(sheet) else {
                return false;
            };
            let name = (1..)
                .map(|k| format!("net{k}"))
                .find(|n| !sh.labels.iter().any(|l| l.name.eq_ignore_ascii_case(n)))
                .unwrap_or_default();
            sh.labels.push(Label {
                name,
                position: at,
                scope: LabelScope::Local,
            });
            *items = Items::from([Item::Label(sh.labels.len() - 1)]);
            true
        })
        .unwrap_or(false)
    }

    /// Place a no-connect flag at `at`.
    pub fn place_no_connect(&mut self, at: Point) -> bool {
        self.edit("Place a no-connect flag", None, |s, sheet, items| {
            let Some(sh) = s.sheets.get_mut(sheet) else {
                return false;
            };
            if sh.no_connects.iter().any(|n| n.position == at) {
                return false;
            }
            sh.no_connects.push(NoConnect { position: at });
            *items = Items::from([Item::NoConnect(sh.no_connects.len() - 1)]);
            true
        })
        .unwrap_or(false)
    }

    /// Draw a wire through `points`.
    pub fn add_wire(&mut self, points: &[Point]) -> bool {
        if points.len() < 2 {
            return false;
        }
        self.edit("Draw a wire", None, |s, sheet, items| {
            let Some(sh) = s.sheets.get_mut(sheet) else {
                return false;
            };
            let (index, _) = edit::add_wire(sh, points);
            *items = index.map(Item::Wire).into_iter().collect();
            index.is_some()
        })
        .unwrap_or(false)
    }

    /// Move the selection by `(dx, dy)` grid units. A run of moves is one step until
    /// [`Open::end_gesture`].
    pub fn move_selection(&mut self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 || self.items.is_empty() {
            return;
        }
        self.edit("Move", Some("move"), |s, sheet, items| {
            if let Some(sh) = s.sheets.get_mut(sheet) {
                edit::translate(sh, items, dx, dy);
            }
        });
    }

    /// Delete the selection.
    pub fn delete_selection(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.edit("Delete", None, |s, sheet, items| {
            if let Some(sh) = s.sheets.get_mut(sheet) {
                edit::delete(sh, items);
            }
            items.clear();
        });
    }

    /// Turn the selection a quarter turn.
    pub fn rotate_selection(&mut self, counter_clockwise: bool) {
        if self.items.is_empty() {
            return;
        }
        self.edit("Rotate", None, |s, sheet, items| {
            if let Some(sh) = s.sheets.get_mut(sheet) {
                edit::rotate(sh, items, counter_clockwise);
            }
        });
    }

    /// Mirror the selection left to right (or top to bottom).
    pub fn flip_selection(&mut self, horizontal: bool) {
        if self.items.is_empty() {
            return;
        }
        self.edit("Mirror", None, |s, sheet, items| {
            if let Some(sh) = s.sheets.get_mut(sheet) {
                edit::flip(sh, items, horizontal);
            }
        });
    }

    /// A copy of the selection.
    pub fn copy_selection(&self) -> Clip {
        match self.sheet_ref() {
            Some(sh) => edit::copy(sh, &self.items),
            None => Clip::default(),
        }
    }

    /// Paste a copy, `offset` grid units from where it was taken.
    pub fn paste(&mut self, clip: &Clip, offset: i32) -> bool {
        if clip.is_empty() {
            return false;
        }
        let pasted = self.edit("Paste", None, |s, sheet, items| {
            *items = edit::paste(s, sheet, clip, offset, offset);
        });
        pasted.is_some()
    }

    /// Paste a copy of the selection beside it.
    pub fn duplicate(&mut self) -> bool {
        let clip = self.copy_selection();
        self.paste(&clip, PASTE_OFFSET)
    }

    // --- The properties panel -----------------------------------------------------------------

    fn selected_part(&self) -> Option<usize> {
        match self.items.iter().collect::<Vec<_>>().as_slice() {
            [Item::Part(k)] => Some(*k),
            _ => None,
        }
    }

    /// Carry out a change from the properties panel.
    ///
    /// # Errors
    /// What is wrong with it, in the interface's language, when it cannot be done.
    pub fn apply(&mut self, action: Action) -> Result<(), String> {
        let one = self.items.iter().next().copied();
        let part = self.selected_part();
        match action {
            Action::Refdes(name) => {
                let k = part.ok_or_else(nothing)?;
                let sheet = self.sheet;
                let outcome = self
                    .edit("Rename", None, |s, _, _| {
                        edit::rename_part(s, sheet, k, &name)
                    })
                    .ok_or_else(nothing)?;
                outcome.map_err(|e| match e {
                    RenameError::Invalid => tr("That is not a valid name").to_owned(),
                    RenameError::Taken => trf("Another part is already called {}", &[name.trim()]),
                })
            }
            Action::Value(v) => self.change_part(part, "Change the value", move |p| p.value = v),
            Action::Model(m) => self.change_part(part, "Change the model", move |p| {
                p.model = m.filter(|m| !m.trim().is_empty());
            }),
            Action::Field(k, v) => {
                if k.trim().is_empty() {
                    return Err(tr("A name cannot be empty").to_owned());
                }
                self.change_part(part, "Change a field", move |p| {
                    p.fields.insert(k.trim().to_owned(), v);
                })
            }
            Action::RemoveField(k) => self.change_part(part, "Remove a field", move |p| {
                p.fields.remove(&k);
            }),
            Action::LabelName(name) => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return Err(tr("A name cannot be empty").to_owned());
                }
                let Some(Item::Label(k)) = one else {
                    return Err(nothing());
                };
                self.edit("Rename the net", None, |s, sheet, _| {
                    if let Some(l) = s.sheets.get_mut(sheet).and_then(|sh| sh.labels.get_mut(k)) {
                        l.name = name;
                    }
                })
                .map(|_| ())
                .ok_or_else(nothing)
            }
            Action::LabelScope(scope) => {
                let Some(Item::Label(k)) = one else {
                    return Err(nothing());
                };
                self.edit("Change the label", None, |s, sheet, _| {
                    if let Some(l) = s.sheets.get_mut(sheet).and_then(|sh| sh.labels.get_mut(k)) {
                        l.scope = scope;
                    }
                })
                .map(|_| ())
                .ok_or_else(nothing)
            }
            Action::PortName(name) => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return Err(tr("A name cannot be empty").to_owned());
                }
                let Some(Item::Port(k)) = one else {
                    return Err(nothing());
                };
                self.edit("Rename the net", None, |s, sheet, _| {
                    if let Some(p) = s
                        .sheets
                        .get_mut(sheet)
                        .and_then(|sh| sh.power_ports.get_mut(k))
                    {
                        *p = edit::new_power_port(&name, p.position);
                    }
                })
                .map(|_| ())
                .ok_or_else(nothing)
            }
            Action::Title(t) => self
                .edit("Change the title", None, |s, _, _| {
                    s.title = t.trim().to_owned()
                })
                .map(|_| ())
                .ok_or_else(nothing),
            Action::Analysis(at, text) => {
                let parsed = parse(&format!("{}\n", text.trim()));
                let analysis = match (parsed.problems.first(), parsed.design.analyses.as_slice()) {
                    (Some(d), _) => return Err(crate::script::message(&d.problem)),
                    (None, [a]) => a.clone(),
                    (None, _) => {
                        return Err(trf("{} is not an analysis", &[text.trim()]));
                    }
                };
                self.edit("Change the analyses", None, |s, _, _| match at {
                    Some(i) => {
                        if let Some(slot) = s.analyses.get_mut(i) {
                            *slot = analysis;
                        }
                    }
                    None => s.analyses.push(analysis),
                })
                .map(|_| ())
                .ok_or_else(nothing)
            }
            Action::RemoveAnalysis(i) => self
                .edit("Change the analyses", None, |s, _, _| {
                    if i < s.analyses.len() {
                        s.analyses.remove(i);
                    }
                })
                .map(|_| ())
                .ok_or_else(nothing),
        }
    }

    fn change_part(
        &mut self,
        part: Option<usize>,
        label: &str,
        f: impl FnOnce(&mut circuito_core::schematic::Part),
    ) -> Result<(), String> {
        let k = part.ok_or_else(nothing)?;
        self.edit(label, None, |s, sheet, _| {
            if let Some(p) = s.sheets.get_mut(sheet).and_then(|sh| sh.parts.get_mut(k)) {
                f(p);
            }
        })
        .map(|_| ())
        .ok_or_else(nothing)
    }

    // --- Keeping the rest in step -------------------------------------------------------------

    /// Work out again what the drawing says, if it changed, and write the text form again if the
    /// person is not in the middle of typing it. Returns whether results are now out of date.
    pub fn refresh(&mut self) -> bool {
        let mut changed = false;
        if self.derived_rev != self.rev {
            self.circuit.rederive();
            self.derived_rev = self.rev;
            let n = self.circuit.analyses().len();
            self.runs = vec![RunState::Idle; n];
            self.analysis = self.analysis.min(n.saturating_sub(1));
            self.invalidate_plot();
            // An empty sheet has nothing to solve; running it would only report an error.
            let drawn = self
                .circuit
                .schematic
                .as_ref()
                .is_none_or(|s| s.sheets.iter().any(|sh| !sh.parts.is_empty()));
            self.run_pending = drawn;
            changed = true;
        }
        if self.script.regenerate && self.script.typed_at.is_none() {
            self.script.regenerate = false;
            if let Some(s) = self
                .circuit
                .schematic
                .as_ref()
                .filter(|s| s.sheets.len() == 1)
            {
                self.script.text = print(&design_of(s), PrintOptions::default());
                self.script.problems.clear();
                self.script.error = None;
            }
        }
        changed
    }

    /// The person typed in the code pane.
    pub fn script_typed(&mut self, now: f64) {
        self.script.typed_at = Some(now);
    }

    /// Read the code pane's text once the person has paused (or at once with `force`) and change
    /// the drawing to match. A text with problems leaves the drawing as it was. Returns whether
    /// the drawing changed.
    pub fn script_settle(&mut self, now: f64, force: bool) -> bool {
        let Some(typed) = self.script.typed_at else {
            return false;
        };
        if !force && now - typed < 0.35 {
            return false;
        }
        self.script.typed_at = None;
        let parsed = parse(&self.script.text);
        self.script.problems = parsed.problems;
        self.script.error = None;
        if !self.script.problems.is_empty() {
            return false;
        }
        let Some(current) = self.circuit.schematic.as_ref() else {
            return false;
        };
        match apply(current, &parsed.design) {
            Ok(next) if next != *current => {
                // Writing code adds and removes parts the person has not placed anywhere, so the
                // view follows them: the whole circuit stays in sight as it grows.
                let count = |s: &Schematic| s.sheets.iter().map(|sh| sh.parts.len()).sum::<usize>();
                if count(&next) != count(current) {
                    self.fit_pending = true;
                }
                self.history
                    .record_merging("code", current.clone(), "Edit the code");
                self.circuit.schematic = Some(next);
                self.set_items(Items::new());
                self.rev += 1;
                self.script.regenerate = false;
                true
            }
            Ok(_) => false,
            Err(e) => {
                self.script.error = Some(e);
                false
            }
        }
    }

    /// The analysis lines as text, for the analyses editor.
    pub fn analysis_lines(&self) -> Vec<String> {
        self.circuit.schematic.as_ref().map_or_else(Vec::new, |s| {
            s.analyses.iter().map(Analysis::to_string).collect()
        })
    }
}

fn nothing() -> String {
    tr("Select one thing on the drawing first").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Circuit, Sample};
    use crate::workspace::Workspace;

    fn open() -> Open {
        let mut ws = Workspace::new(None);
        ws.open(Circuit::blank());
        let _ = ws.active_mut();
        ws.take_active_for_test()
    }

    fn at(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    fn parts(o: &Open) -> usize {
        o.circuit
            .schematic
            .as_ref()
            .map_or(0, |s| s.sheets[0].parts.len())
    }

    #[test]
    fn the_ghost_turns_round_both_ways() {
        let g = Ghost::default();
        assert_eq!(g.turned(false).rotation(), Rotation::R90);
        assert_eq!(g.turned(true).rotation(), Rotation::R270);
        assert_eq!(g.turned(false).turned(false).turned(false).turned(false), g);
    }

    #[test]
    fn placing_a_part_names_it_and_selects_it() {
        let mut o = open();
        assert!(o.place_part("R", at(0, 0), Rotation::R0, false));
        assert!(o.place_part("R", at(20, 0), Rotation::R90, true));
        let s = o.circuit.schematic.as_ref().unwrap();
        assert_eq!(s.sheets[0].parts[0].refdes, "R1");
        assert_eq!(s.sheets[0].parts[1].refdes, "R2");
        assert_eq!(s.sheets[0].parts[1].rotation, Rotation::R90);
        assert!(s.sheets[0].parts[1].mirror);
        assert_eq!(o.items, Items::from([Item::Part(1)]));
        assert_eq!(o.selection, Some(Selection::Part { sheet: 0, part: 1 }));
        assert!(!o.place_part("NOPE", at(0, 0), Rotation::R0, false));
        assert!(o.dirty());
    }

    #[test]
    fn every_edit_can_be_undone_and_redone() {
        let mut o = open();
        o.place_part("R", at(0, 0), Rotation::R0, false);
        o.place_part("C", at(20, 0), Rotation::R0, false);
        assert_eq!(parts(&o), 2);
        assert_eq!(o.undo().as_deref(), Some("Place a part"));
        assert_eq!(parts(&o), 1);
        assert!(o.items.is_empty());
        assert!(o.redo().is_some());
        assert_eq!(parts(&o), 2);
        o.undo();
        o.undo();
        assert_eq!(parts(&o), 0);
        assert!(o.undo().is_none());
    }

    #[test]
    fn a_drag_is_one_step_and_a_no_op_leaves_none() {
        let mut o = open();
        o.place_part("R", at(0, 0), Rotation::R0, false);
        o.end_gesture();
        for _ in 0..5 {
            o.move_selection(1, 0);
        }
        o.end_gesture();
        let moved = o.circuit.schematic.as_ref().unwrap().sheets[0].parts[0].position;
        assert_eq!(moved, at(5, 0));
        o.undo();
        assert_eq!(
            o.circuit.schematic.as_ref().unwrap().sheets[0].parts[0].position,
            at(0, 0)
        );
        let steps = o.history.can_undo();
        o.move_selection(0, 0);
        o.rotate_selection(false);
        o.rotate_selection(true);
        assert!(steps && o.history.can_undo());
    }

    #[test]
    fn wires_follow_moved_parts_and_deleting_removes_them() {
        let mut o = open();
        o.place_part("R", at(0, 0), Rotation::R0, false);
        o.place_part("R", at(20, 0), Rotation::R0, false);
        assert!(
            o.add_wire(&[at(0, 4), at(20, -4)])
                || o.add_wire(&[at(0, 4), at(0, 8), at(20, 8), at(20, -4)])
        );
        o.select_all();
        o.delete_selection();
        let s = o.circuit.schematic.as_ref().unwrap();
        assert!(s.sheets[0].parts.is_empty() && s.sheets[0].wires.is_empty());
    }

    #[test]
    fn copy_and_paste_make_new_names() {
        let mut o = open();
        o.place_part("R", at(0, 0), Rotation::R0, false);
        assert!(o.duplicate());
        let names: Vec<String> = o.circuit.schematic.as_ref().unwrap().sheets[0]
            .parts
            .iter()
            .map(|p| p.refdes.clone())
            .collect();
        assert_eq!(names, vec!["R1".to_string(), "R2".to_string()]);
        let clip = Clip::default();
        assert!(!o.paste(&clip, 4));
    }

    #[test]
    fn the_panel_changes_values_names_and_fields() {
        let mut o = open();
        o.place_part("R", at(0, 0), Rotation::R0, false);
        o.place_part("R", at(20, 0), Rotation::R0, false);
        o.apply(Action::Value("4.7k".into())).unwrap();
        o.apply(Action::Refdes("Rload".into())).unwrap();
        assert!(o.apply(Action::Refdes("R1".into())).is_err());
        assert!(o.apply(Action::Refdes("a b".into())).is_err());
        o.apply(Action::Field("tol".into(), "1%".into())).unwrap();
        let p = &o.circuit.schematic.as_ref().unwrap().sheets[0].parts[1];
        assert_eq!((p.refdes.as_str(), p.value.as_str()), ("Rload", "4.7k"));
        assert_eq!(p.fields.get("tol").map(String::as_str), Some("1%"));
        o.apply(Action::RemoveField("tol".into())).unwrap();
        o.apply(Action::Model(Some("  ".into()))).unwrap();
        o.set_items(Items::new());
        assert!(o.apply(Action::Value("1".into())).is_err());
    }

    #[test]
    fn labels_and_power_ports_are_renamed() {
        let mut o = open();
        o.place_label(at(0, 0));
        o.apply(Action::LabelName("out".into())).unwrap();
        assert!(o.apply(Action::LabelName("  ".into())).is_err());
        o.apply(Action::LabelScope(LabelScope::Global)).unwrap();
        o.place_power("VCC", at(4, 4));
        o.apply(Action::PortName("gnd".into())).unwrap();
        let sh = &o.circuit.schematic.as_ref().unwrap().sheets[0];
        assert_eq!(
            (sh.labels[0].name.as_str(), sh.labels[0].scope),
            ("out", LabelScope::Global)
        );
        assert_eq!(
            sh.power_ports[0].style,
            circuito_core::schematic::PowerStyle::Ground
        );
    }

    #[test]
    fn analyses_are_edited_as_text() {
        let mut o = open();
        assert_eq!(o.analysis_lines(), vec![".op".to_string()]);
        o.apply(Action::Analysis(None, ".ac dec 20 10 100k".into()))
            .unwrap();
        o.apply(Action::Analysis(Some(0), ".tran 10u 1m".into()))
            .unwrap();
        assert_eq!(o.analysis_lines().len(), 2);
        assert!(o.apply(Action::Analysis(None, ".ac".into())).is_err());
        assert!(o.apply(Action::Analysis(None, "R1 a b".into())).is_err());
        o.apply(Action::RemoveAnalysis(1)).unwrap();
        o.refresh();
        assert_eq!(o.runs.len(), 1);
    }

    #[test]
    fn refreshing_rederives_and_resets_results() {
        let mut o = open();
        o.place_part("V", at(0, 0), Rotation::R0, false);
        let pins = o
            .circuit
            .schematic
            .as_ref()
            .and_then(|s| s.sheets[0].parts.first().map(edit::pin_points))
            .unwrap();
        o.place_power("GND", pins[1]);
        o.place_label(pins[0]);
        assert!(o.refresh());
        assert!(!o.refresh());
        assert_eq!(o.circuit.analyses().len(), 1);
        assert!(o.circuit.netlist.is_some());
    }

    #[test]
    fn the_text_follows_the_drawing_and_the_drawing_follows_the_text() {
        let mut ws = Workspace::new(None);
        ws.open(Circuit::sample(Sample::Divider));
        let mut o = ws.take_active_for_test();
        o.refresh();
        assert!(o.script.text.contains("R1"), "{}", o.script.text);
        assert!(o.script.is_clean());
        // Editing the text changes the drawing, and is one step that can be undone.
        let edited = o.script.text.replace("2.2k", "3.3k");
        o.script.text = edited;
        o.script_typed(0.0);
        assert!(!o.script_settle(0.1, false), "waits for a pause");
        assert!(o.script_settle(1.0, false));
        let r2 = o.circuit.schematic.as_ref().unwrap().sheets[0]
            .parts
            .iter()
            .find(|p| p.refdes == "R2")
            .map(|p| p.value.clone());
        assert_eq!(r2.as_deref(), Some("3.3k"));
        assert!(o.script.text.contains("3.3k"));
        o.refresh();
        assert!(o.history.can_undo());
        o.undo();
        o.refresh();
        assert!(o.script.text.contains("2.2k"), "{}", o.script.text);
    }

    #[test]
    fn an_empty_sheet_is_not_run_and_code_that_adds_parts_refits_the_view() {
        let mut ws = Workspace::new(None);
        ws.open(Circuit::blank());
        let mut o = ws.take_active_for_test();
        o.refresh();
        assert!(!o.run_pending, "nothing to solve on an empty sheet");
        o.fit_pending = false;
        o.script.text = "V1 5 in gnd\nR1 1k in out\nR2 1k out gnd\nop\n".into();
        o.script_typed(0.0);
        assert!(o.script_settle(1.0, false));
        assert!(o.fit_pending, "new parts are brought into view");
        o.refresh();
        assert!(o.run_pending, "a drawn circuit is solved");
        // Changing only a value moves nothing, so the view stays where the person left it.
        o.fit_pending = false;
        o.script.text = o.script.text.replace("1k out gnd", "2k out gnd");
        o.script_typed(2.0);
        assert!(o.script_settle(3.0, false));
        assert!(!o.fit_pending);
    }

    #[test]
    fn a_text_with_a_problem_leaves_the_drawing_alone() {
        let mut ws = Workspace::new(None);
        ws.open(Circuit::sample(Sample::Divider));
        let mut o = ws.take_active_for_test();
        o.refresh();
        let before = o.circuit.schematic.clone();
        o.script.text.push_str("R9 1k onlyone\nZ?? what\n");
        o.script_typed(0.0);
        assert!(!o.script_settle(0.0, true));
        assert!(!o.script.problems.is_empty());
        assert_eq!(o.circuit.schematic, before);
        // The text is kept so the person can fix it; a drawing edit would replace it.
        assert!(o.script.text.contains("onlyone"));
    }

    #[test]
    fn a_new_connection_in_the_text_is_drawn() {
        let mut o = open();
        o.refresh();
        o.script.text = "V1 5 in gnd\nR1 1k in out\nR2 1k out gnd\n.op\n".into();
        o.script_typed(0.0);
        assert!(o.script_settle(0.0, true));
        o.refresh();
        assert_eq!(parts(&o), 3);
        assert!(o.circuit.blocked.is_none(), "{:?}", o.circuit.blocked);
        assert_eq!(o.circuit.analyses().len(), 1);
    }
}

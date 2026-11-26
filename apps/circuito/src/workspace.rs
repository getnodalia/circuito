// SPDX-License-Identifier: MIT OR Apache-2.0
//! The open circuits and what the person did with them: which is shown, what is selected, which
//! finding is looked at, which analysis is chosen and how each one is getting on. The window
//! reads this and calls its methods; nothing here draws.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use circuito_core::edit::{History, Items};
use circuito_core::library::Library;
use circuito_core::netlist::Analysis;
use circuito_core::schematic::{Rotation, Schematic};
use circuito_erc::Severity;
use circuito_sim::AnalysisResult;

use crate::document::{Circuit, Kind};
use crate::editing::{Gesture, Ghost, Tool};
use crate::geometry::{self, Selection, View};
use crate::plot::{self, Plot};
use crate::script::Script;
use crate::sim::{Outcome, Reply, Runner};

/// The left panel's pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The parts to draw with.
    Parts,
    /// The sheets, parts and analyses of the open circuit.
    Navigator,
}

/// What the centre of the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Schematic,
    Netlist,
}

/// What the bottom panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bottom {
    Rules,
    Simulation,
}

/// How one analysis is getting on.
#[derive(Clone, Debug, PartialEq)]
pub enum RunState {
    Idle,
    Queued(u64),
    Running(u64),
    Done {
        result: Arc<AnalysisResult>,
        elapsed: Duration,
    },
    /// The engine's message.
    Failed(String),
    Cancelled,
}

impl RunState {
    pub fn is_active(&self) -> bool {
        matches!(self, RunState::Queued(_) | RunState::Running(_))
    }
}

/// Which analyses to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    /// The chosen one.
    Chosen,
    All,
}

/// Why nothing ran.
#[derive(Clone, Debug, PartialEq)]
pub enum RunError {
    NoCircuit,
    NoAnalyses,
    /// The circuit cannot be simulated; the engine's reason.
    Blocked(String),
    WorkerLost,
}

/// One open circuit.
pub struct Open {
    pub id: u64,
    pub circuit: Circuit,
    pub sheet: usize,
    pub view: View,
    /// Fit the drawing to the canvas the next time its size is known.
    pub fit_pending: bool,
    pub selection: Option<Selection>,
    /// The finding the person is looking at (an index into the report).
    pub finding: Option<usize>,
    pub runs: Vec<RunState>,
    /// The analysis shown in the simulation panel.
    pub analysis: usize,
    pub hidden: HashSet<String>,
    plot: Option<Arc<Plot>>,
    /// What the person has selected on the shown sheet, for editing.
    pub items: Items,
    /// The drawing as it was before each edit, to go back to.
    pub history: History<Schematic>,
    /// Counts the edits (and undos) made.
    pub rev: u64,
    /// The value of `rev` the file on disk has.
    pub saved_rev: u64,
    /// The value of `rev` the rules check, netlist and results were made from.
    pub derived_rev: u64,
    /// The analyses should run again once the pointer is up.
    pub run_pending: bool,
    /// The text form of the drawing.
    pub script: Script,
    /// What the pointer is in the middle of on the drawing.
    pub gesture: Gesture,
}

/// A part, as the properties panel lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct PartInfo {
    pub refdes: String,
    pub name: String,
    pub value: String,
    pub position: (i32, i32),
    pub rotation: u16,
    pub mirror: bool,
    pub model: Option<String>,
    pub fields: Vec<(String, String)>,
    pub pins: Vec<PinInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PinInfo {
    pub number: String,
    pub name: String,
    pub net: Option<String>,
}

impl Open {
    fn new(id: u64, circuit: Circuit) -> Open {
        let runs = vec![RunState::Idle; circuit.analyses().len()];
        Open {
            id,
            circuit,
            sheet: 0,
            view: View::default(),
            fit_pending: true,
            selection: None,
            finding: None,
            runs,
            analysis: 0,
            hidden: HashSet::new(),
            plot: None,
            items: Items::new(),
            history: History::default(),
            rev: 0,
            saved_rev: 0,
            derived_rev: 0,
            run_pending: false,
            script: Script::new(),
            gesture: Gesture::Idle,
        }
    }

    /// Errors, warnings and notes the rules check found.
    pub fn counts(&self) -> (usize, usize, usize) {
        match &self.circuit.report {
            Some(r) => (
                r.count(Severity::Error),
                r.count(Severity::Warning),
                r.count(Severity::Info),
            ),
            None => (0, 0, 0),
        }
    }

    /// The result of the chosen analysis, when it has one.
    pub fn result(&self) -> Option<&Arc<AnalysisResult>> {
        match self.runs.get(self.analysis)? {
            RunState::Done { result, .. } => Some(result),
            _ => None,
        }
    }

    /// The plot of the chosen analysis, built when first asked for.
    pub fn plot(&mut self) -> Option<Arc<Plot>> {
        if self.plot.is_none() {
            let result = self.result()?.clone();
            self.plot = Some(Arc::new(plot::build(&result, &self.hidden)));
        }
        self.plot.clone()
    }

    pub(crate) fn invalidate_plot(&mut self) {
        self.plot = None;
    }

    /// The selected part and its pins with the nets they are on.
    pub fn part_info(&self) -> Option<PartInfo> {
        let Selection::Part { sheet, part } = self.selection? else {
            return None;
        };
        let p = self
            .circuit
            .schematic
            .as_ref()?
            .sheets
            .get(sheet)?
            .parts
            .get(part)?;
        let def = Library::builtin().get(&p.lib_id);
        let pins = def
            .map(|d| {
                d.pins
                    .iter()
                    .enumerate()
                    .map(|(i, pin)| PinInfo {
                        number: pin.number.to_owned(),
                        name: pin.name.to_owned(),
                        net: self.circuit.net_of_pin(sheet, part, i).map(str::to_owned),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(PartInfo {
            refdes: p.refdes.clone(),
            name: def.map_or_else(|| p.lib_id.clone(), |d| d.name.to_owned()),
            value: p.value.clone(),
            position: (p.position.x, p.position.y),
            rotation: match p.rotation {
                Rotation::R0 => 0,
                Rotation::R90 => 90,
                Rotation::R180 => 180,
                Rotation::R270 => 270,
            },
            mirror: p.mirror,
            model: p.model.clone(),
            fields: p
                .fields
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            pins,
        })
    }
}

/// The open circuits.
pub struct Workspace {
    docs: Vec<Open>,
    active: usize,
    next_id: u64,
    pub tab: Tab,
    pub bottom: Bottom,
    /// What a click on the drawing does.
    pub tool: Tool,
    /// The turn and mirroring of what the tool places.
    pub ghost: Ghost,
    /// Which of the left panel's pages shows.
    pub side: Side,
    runner: Option<Runner>,
}

impl Workspace {
    pub fn new(runner: Option<Runner>) -> Workspace {
        Workspace {
            docs: Vec::new(),
            active: 0,
            next_id: 1,
            tab: Tab::Schematic,
            bottom: Bottom::Simulation,
            tool: Tool::default(),
            ghost: Ghost::default(),
            side: Side::Parts,
            runner,
        }
    }

    /// Take the shown circuit out of the workspace (tests of what happens to one circuit).
    #[cfg(test)]
    pub(crate) fn take_active_for_test(&mut self) -> Open {
        let i = self.active_index().unwrap_or(0);
        self.docs.remove(i)
    }

    pub fn docs(&self) -> &[Open] {
        &self.docs
    }

    pub fn docs_mut(&mut self) -> &mut [Open] {
        &mut self.docs
    }

    pub fn active_index(&self) -> Option<usize> {
        (!self.docs.is_empty()).then(|| self.active.min(self.docs.len() - 1))
    }

    pub fn active(&self) -> Option<&Open> {
        self.docs.get(self.active_index()?)
    }

    pub fn active_mut(&mut self) -> Option<&mut Open> {
        let i = self.active_index()?;
        self.docs.get_mut(i)
    }

    /// The tab that applies: a SPICE deck has only its netlist.
    pub fn tab(&self) -> Tab {
        match self.active() {
            Some(d) if d.circuit.kind == Kind::Spice => Tab::Netlist,
            _ => self.tab,
        }
    }

    pub fn runner_lost(&self) -> bool {
        self.runner.as_ref().is_none_or(Runner::is_lost)
    }

    pub fn busy(&self) -> bool {
        self.runner.as_ref().is_some_and(Runner::busy)
    }

    /// Show `circuit`. One already open from the same file (or the same sample) is shown instead
    /// of opening it twice. Returns its index.
    pub fn open(&mut self, circuit: Circuit) -> usize {
        let same = self.docs.iter().position(|d| {
            (circuit.path.is_some() && d.circuit.path == circuit.path)
                || (circuit.path.is_none()
                    && circuit.sample.is_some()
                    && d.circuit.sample == circuit.sample
                    && d.circuit.path.is_none())
        });
        if let Some(i) = same {
            self.active = i;
            return i;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.docs.push(Open::new(id, circuit));
        self.active = self.docs.len() - 1;
        self.docs.len() - 1
    }

    /// Replace circuit `i` with a fresh read of the same file, keeping the view.
    pub fn replace(&mut self, i: usize, circuit: Circuit) {
        let Some(doc) = self.docs.get_mut(i) else {
            return;
        };
        let (view, sheet) = (doc.view, doc.sheet);
        let id = self.next_id;
        self.next_id += 1;
        *doc = Open::new(id, circuit);
        doc.view = view;
        doc.fit_pending = false;
        doc.sheet = sheet.min(
            doc.circuit
                .schematic
                .as_ref()
                .map_or(0, |s| s.sheets.len().saturating_sub(1)),
        );
    }

    pub fn close(&mut self, i: usize) {
        if i >= self.docs.len() {
            return;
        }
        self.docs.remove(i);
        if self.active > i || self.active >= self.docs.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    pub fn activate(&mut self, i: usize) {
        if i < self.docs.len() {
            self.active = i;
        }
    }

    /// The next (or previous) circuit, wrapping round.
    pub fn step(&mut self, forward: bool) {
        let n = self.docs.len();
        if n < 2 {
            return;
        }
        self.active = if forward {
            (self.active + 1) % n
        } else {
            (self.active + n - 1) % n
        };
    }

    // --- Selection and findings ---------------------------------------------------------------

    pub fn select(&mut self, selection: Option<Selection>) {
        if let Some(doc) = self.active_mut() {
            doc.items = match selection {
                Some(Selection::Part { part, .. }) => {
                    circuito_core::edit::Items::from([circuito_core::edit::Item::Part(part)])
                }
                _ => circuito_core::edit::Items::new(),
            };
            doc.selection = selection;
            doc.finding = None;
            if let Some(s) = selection {
                doc.sheet = s.sheet();
            }
        }
    }

    /// Look at finding `index` of the rules check: select what it points at and bring it to the
    /// middle of the drawing.
    pub fn select_finding(&mut self, index: usize) {
        let Some(doc) = self.active_mut() else {
            return;
        };
        let Some(schematic) = doc.circuit.schematic.as_ref() else {
            return;
        };
        let Some(v) = doc
            .circuit
            .report
            .as_ref()
            .and_then(|r| r.violations.get(index))
        else {
            return;
        };
        let target = geometry::target_of(schematic, v);
        let centre = target.and_then(|t| geometry::centre_of(schematic, t));
        doc.finding = Some(index);
        doc.selection = target;
        doc.items = match target {
            Some(Selection::Part { part, .. }) => {
                circuito_core::edit::Items::from([circuito_core::edit::Item::Part(part)])
            }
            _ => circuito_core::edit::Items::new(),
        };
        if let Some(t) = target {
            doc.sheet = t.sheet();
        }
        if let Some(c) = centre {
            doc.view = doc.view.centred_on(c);
        }
        self.tab = Tab::Schematic;
    }

    /// The next (or previous) finding, wrapping round.
    pub fn step_finding(&mut self, forward: bool) {
        let Some(doc) = self.active() else {
            return;
        };
        let n = doc
            .circuit
            .report
            .as_ref()
            .map_or(0, |r| r.violations.len());
        if n == 0 {
            return;
        }
        let next = match (doc.finding, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        self.select_finding(next);
    }

    // --- Analyses -----------------------------------------------------------------------------

    pub fn choose_analysis(&mut self, index: usize) {
        if let Some(doc) = self.active_mut() {
            if index < doc.runs.len() && doc.analysis != index {
                doc.analysis = index;
                doc.invalidate_plot();
            }
        }
    }

    /// Show or hide a signal of the chosen result.
    pub fn toggle_signal(&mut self, name: &str) {
        if let Some(doc) = self.active_mut() {
            if !doc.hidden.remove(name) {
                doc.hidden.insert(name.to_owned());
            }
            doc.invalidate_plot();
        }
    }

    /// Queue analyses of the shown circuit. Returns how many were queued.
    ///
    /// # Errors
    /// There is no circuit, it asks for no analysis, it cannot be simulated, or the worker is gone.
    pub fn run(&mut self, which: Which) -> Result<usize, RunError> {
        let i = self.active_index().ok_or(RunError::NoCircuit)?;
        let Some(runner) = self.runner.as_mut().filter(|r| !r.is_lost()) else {
            return Err(RunError::WorkerLost);
        };
        let doc = self.docs.get_mut(i).ok_or(RunError::NoCircuit)?;
        if let Some(reason) = &doc.circuit.blocked {
            return Err(RunError::Blocked(reason.clone()));
        }
        let netlist = doc.circuit.netlist.clone().ok_or(RunError::NoAnalyses)?;
        let indexes: Vec<usize> = match which {
            Which::All => (0..netlist.analyses.len()).collect(),
            Which::Chosen => vec![doc.analysis],
        };
        let mut queued = 0;
        for index in indexes {
            let Some(analysis) = netlist.analyses.get(index) else {
                continue;
            };
            if doc.runs.get(index).is_some_and(RunState::is_active) {
                continue;
            }
            let run = runner
                .submit(doc.id, index, netlist.clone(), analysis.clone())
                .map_err(|_| RunError::WorkerLost)?;
            if let Some(slot) = doc.runs.get_mut(index) {
                *slot = RunState::Queued(run);
            }
            queued += 1;
        }
        if queued == 0 && netlist.analyses.is_empty() {
            return Err(RunError::NoAnalyses);
        }
        Ok(queued)
    }

    /// Stop everything queued or running.
    pub fn stop(&mut self) {
        if let Some(runner) = &self.runner {
            runner.stop();
        }
    }

    /// The fraction done of analysis `index` of the shown circuit.
    pub fn progress(&self, index: usize) -> Option<f32> {
        let run = match self.active()?.runs.get(index)? {
            RunState::Queued(r) | RunState::Running(r) => *r,
            _ => return None,
        };
        self.runner.as_ref()?.progress(run)
    }

    /// Take in what the worker has said. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let Some(runner) = self.runner.as_mut() else {
            return false;
        };
        let replies = runner.poll();
        let changed = !replies.is_empty();
        for reply in replies {
            let (circuit, index) = match &reply {
                Reply::Started { circuit, index, .. } | Reply::Done { circuit, index, .. } => {
                    (*circuit, *index)
                }
            };
            // A circuit closed in the meantime has nothing to show the answer in.
            let Some(doc) = self.docs.iter_mut().find(|d| d.id == circuit) else {
                continue;
            };
            let Some(current) = doc.runs.get(index).cloned() else {
                continue;
            };
            match reply {
                Reply::Started { run, .. } => {
                    if current == RunState::Queued(run) {
                        if let Some(slot) = doc.runs.get_mut(index) {
                            *slot = RunState::Running(run);
                        }
                    }
                }
                Reply::Done {
                    run,
                    outcome,
                    elapsed,
                    ..
                } => {
                    // A newer run of the same analysis owns the slot.
                    if !matches!(current, RunState::Queued(r) | RunState::Running(r) if r == run) {
                        continue;
                    }
                    let next = match outcome {
                        Outcome::Finished(result) => {
                            if doc.analysis == index {
                                // The default view of a first result: voltages only.
                                doc.hidden = plot::default_hidden(&result);
                                doc.invalidate_plot();
                            }
                            RunState::Done {
                                result: Arc::new(*result),
                                elapsed,
                            }
                        }
                        Outcome::Failed(m) => RunState::Failed(m),
                        Outcome::Cancelled => RunState::Cancelled,
                    };
                    if let Some(slot) = doc.runs.get_mut(index) {
                        *slot = next;
                    }
                }
            }
        }
        changed
    }
}

/// A short description of an analysis, as the list shows it (`.ac dec 20 10 100k`).
pub fn describe(a: &Analysis) -> String {
    a.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Sample;
    use circuito_core::schematic::builder::Builder;
    use std::time::Instant;

    fn workspace() -> Workspace {
        Workspace::new(Some(Runner::spawn(|| {}).unwrap()))
    }

    fn settle(ws: &mut Workspace) {
        let end = Instant::now() + Duration::from_secs(30);
        while ws.busy() && Instant::now() < end {
            ws.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        ws.poll();
        assert!(!ws.busy(), "the worker did not finish");
    }

    /// A resistor with one pin left open: the rules check has something to say.
    fn unfinished() -> Circuit {
        let mut b = Builder::new("unfinished");
        let r1 = b.place("R", "R1", "1k", 0, 0);
        b.label_pin(r1, "1", "a");
        Circuit::from_schematic(b.build())
    }

    #[test]
    fn opening_the_same_sample_or_file_shows_the_one_already_open() {
        let mut ws = workspace();
        let a = ws.open(Circuit::sample(Sample::RcLowPass));
        let b = ws.open(Circuit::sample(Sample::Divider));
        assert_eq!(
            (a, b, ws.docs().len(), ws.active_index()),
            (0, 1, 2, Some(1))
        );
        assert_eq!(ws.open(Circuit::sample(Sample::RcLowPass)), 0);
        assert_eq!((ws.docs().len(), ws.active_index()), (2, Some(0)));
        let mut file = Circuit::sample(Sample::Divider);
        file.sample = None;
        file.path = Some("/tmp/d.circuito".into());
        let c = ws.open(file.clone());
        assert_eq!(ws.open(file), c);
        assert_eq!(ws.docs().len(), 3);
    }

    #[test]
    fn closing_and_stepping_keep_the_active_circuit_valid() {
        let mut ws = workspace();
        assert_eq!(ws.active_index(), None);
        ws.step(true);
        ws.close(3);
        for s in Sample::ALL {
            ws.open(Circuit::sample(s));
        }
        assert_eq!(ws.active_index(), Some(2));
        ws.step(true);
        assert_eq!(ws.active_index(), Some(0));
        ws.step(false);
        assert_eq!(ws.active_index(), Some(2));
        ws.close(2);
        assert_eq!(ws.active_index(), Some(1));
        ws.close(0);
        assert_eq!(ws.active_index(), Some(0));
        ws.close(0);
        assert_eq!(ws.active_index(), None);
        ws.activate(7);
        assert!(ws.active().is_none());
    }

    #[test]
    fn a_finding_selects_its_part_and_brings_it_to_the_middle() {
        let mut ws = workspace();
        ws.open(unfinished());
        let doc = ws.active().unwrap();
        assert!(doc.counts().0 + doc.counts().1 + doc.counts().2 > 0);
        ws.tab = Tab::Netlist;
        let at = ws
            .active()
            .unwrap()
            .circuit
            .report
            .as_ref()
            .unwrap()
            .violations
            .iter()
            .position(|v| v.items.iter().any(|i| i.starts_with("R1")))
            .expect("a finding names R1");
        ws.select_finding(at);
        let doc = ws.active().unwrap();
        assert_eq!(doc.finding, Some(at));
        assert_eq!(doc.selection, Some(Selection::Part { sheet: 0, part: 0 }));
        let info = doc.part_info().unwrap();
        assert_eq!(info.refdes, "R1");
        assert_eq!(info.pins.len(), 2);
        assert_eq!(info.pins[0].net.as_deref(), Some("a"));
        assert!((doc.view.centre.0).abs() < 3.0);
        assert_eq!(ws.tab, Tab::Schematic, "the finding is on the drawing");
        // Past the end does nothing; stepping wraps.
        ws.select_finding(999);
        assert_eq!(ws.active().unwrap().finding, Some(at));
        let n = ws
            .active()
            .unwrap()
            .circuit
            .report
            .as_ref()
            .unwrap()
            .violations
            .len();
        for _ in 0..n {
            ws.step_finding(true);
        }
        assert_eq!(
            ws.active().unwrap().finding,
            Some(at),
            "a full lap comes back"
        );
        ws.select_finding(0);
        ws.step_finding(false);
        assert_eq!(ws.active().unwrap().finding, Some(n - 1));
        // Choosing a part by hand drops the finding.
        ws.select(Some(Selection::Part { sheet: 0, part: 0 }));
        assert_eq!(ws.active().unwrap().finding, None);
    }

    #[test]
    fn running_every_analysis_fills_each_slot() {
        let mut ws = workspace();
        ws.open(Circuit::sample(Sample::RcLowPass));
        assert_eq!(ws.run(Which::All), Ok(2));
        assert!(ws.busy());
        assert!(ws.active().unwrap().runs.iter().all(RunState::is_active));
        // Asking again while they run queues nothing more.
        assert_eq!(ws.run(Which::All), Ok(0));
        settle(&mut ws);
        let doc = ws.active().unwrap();
        assert!(
            doc.runs.iter().all(|r| matches!(r, RunState::Done { .. })),
            "{:?}",
            doc.runs
        );
        assert!(doc.result().is_some());
        // The first result starts with the currents hidden.
        let hidden = doc.hidden.clone();
        assert!(hidden
            .iter()
            .all(|n| n.to_ascii_uppercase().starts_with("I(")));
    }

    #[test]
    fn the_plot_follows_the_chosen_analysis_and_the_hidden_signals() {
        let mut ws = workspace();
        ws.open(Circuit::sample(Sample::RcLowPass));
        ws.run(Which::All).unwrap();
        settle(&mut ws);
        let ac_panels = ws.active_mut().unwrap().plot().unwrap().panels.len();
        assert_eq!(ac_panels, 2);
        ws.choose_analysis(1);
        assert_eq!(ws.active_mut().unwrap().plot().unwrap().panels.len(), 1);
        ws.choose_analysis(0);
        let name = ws.active_mut().unwrap().plot().unwrap().panels[0].series[0]
            .name
            .clone();
        ws.toggle_signal(&name);
        let doc = ws.active_mut().unwrap();
        assert!(doc.plot().unwrap().panels[0]
            .series
            .iter()
            .all(|s| s.name != name));
        ws.toggle_signal(&name);
        assert!(ws.active_mut().unwrap().plot().unwrap().panels[0]
            .series
            .iter()
            .any(|s| s.name == name));
        // An analysis that does not exist is not chosen.
        ws.choose_analysis(9);
        assert_eq!(ws.active().unwrap().analysis, 0);
    }

    #[test]
    fn stopping_ends_the_queue_and_a_closed_circuit_ignores_late_answers() {
        let mut ws = workspace();
        ws.open(Circuit::sample(Sample::SeriesRlc));
        ws.run(Which::All).unwrap();
        ws.stop();
        ws.close(0);
        settle(&mut ws);
        assert!(ws.active().is_none());
        // The same circuit again starts clean.
        ws.open(Circuit::sample(Sample::SeriesRlc));
        assert!(ws
            .active()
            .unwrap()
            .runs
            .iter()
            .all(|r| *r == RunState::Idle));
    }

    #[test]
    fn what_cannot_run_says_why() {
        let mut ws = workspace();
        assert_eq!(ws.run(Which::All), Err(RunError::NoCircuit));
        let mut b = Builder::new("broken");
        let r1 = b.place("R", "R1", "", 0, 0);
        b.power_pin(r1, "2", "GND");
        let mut circuit = Circuit::from_schematic(b.build());
        circuit.blocked = Some("R1 has no value".into());
        ws.open(circuit);
        assert_eq!(
            ws.run(Which::All),
            Err(RunError::Blocked("R1 has no value".into()))
        );
        let mut gone = Workspace::new(None);
        gone.open(Circuit::sample(Sample::Divider));
        assert!(gone.runner_lost());
        assert_eq!(gone.run(Which::All), Err(RunError::WorkerLost));
        let mut none = workspace();
        none.open(
            Circuit::from_text(Some("x.cir".as_ref()), "t\nV1 a 0 1\nR1 a 0 1k\n.end\n").unwrap(),
        );
        assert_eq!(none.run(Which::All), Err(RunError::NoAnalyses));
    }

    #[test]
    fn a_spice_deck_always_shows_its_netlist() {
        let mut ws = workspace();
        ws.open(
            Circuit::from_text(
                Some("x.cir".as_ref()),
                "t\nV1 a 0 1\nR1 a 0 1k\n.op\n.end\n",
            )
            .unwrap(),
        );
        ws.tab = Tab::Schematic;
        assert_eq!(ws.tab(), Tab::Netlist);
        ws.open(Circuit::sample(Sample::Divider));
        assert_eq!(ws.tab(), Tab::Schematic);
    }

    #[test]
    fn replacing_a_circuit_keeps_the_view_and_clears_the_results() {
        let mut ws = workspace();
        ws.open(Circuit::sample(Sample::Divider));
        ws.run(Which::All).unwrap();
        settle(&mut ws);
        ws.active_mut().unwrap().view = View {
            centre: (5.0, 6.0),
            zoom: 33.0,
        };
        ws.replace(0, Circuit::sample(Sample::Divider));
        let doc = ws.active().unwrap();
        assert_eq!(doc.view.zoom, 33.0);
        assert!(!doc.fit_pending);
        assert!(doc.runs.iter().all(|r| *r == RunState::Idle));
        ws.replace(9, Circuit::sample(Sample::Divider));
    }
}

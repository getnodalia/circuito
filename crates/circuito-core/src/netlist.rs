// SPDX-License-Identifier: MIT OR Apache-2.0
//! The netlist: nodes, elements, model cards, subcircuits and analysis requests.
//!
//! Names are case-insensitive as in SPICE. Element, model and subcircuit names are stored in
//! upper case; node names keep the case they were first written with but compare without regard to
//! case. Node `0` (or `gnd`, in any case) is ground and is always [`NodeId::GROUND`].
//!
//! `Display` on [`Netlist`] writes canonical SPICE text that the reader in `circuito-spice` parses
//! back to an equal netlist.

use crate::source::Source;
use crate::value::format_value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use thiserror::Error;

/// Most elements a netlist (including after subcircuit expansion) may hold.
pub const MAX_ELEMENTS: usize = 500_000;
/// Deepest subcircuit nesting that is expanded.
pub const MAX_SUBCKT_DEPTH: usize = 16;
const MAX_NAME_LEN: usize = 128;

/// Everything that can be wrong with a netlist, each naming the thing involved.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum NetlistError {
    /// A node name is empty, too long or contains forbidden characters.
    #[error("invalid node name `{0}` (names must be 1-128 characters without spaces or any of ( ) , = ; {{ }} \" ')")]
    InvalidNodeName(String),
    /// An element/model/subcircuit name is malformed.
    #[error("invalid name `{0}` (names must be 1-128 characters without spaces or any of ( ) , = ; {{ }} \" ')")]
    InvalidName(String),
    /// Two elements share a name.
    #[error("duplicate element name `{0}`")]
    DuplicateElement(String),
    /// A `.model` is defined twice.
    #[error("duplicate model name `{0}`")]
    DuplicateModel(String),
    /// A `.subckt` is defined twice.
    #[error("duplicate subcircuit name `{0}`")]
    DuplicateSubckt(String),
    /// An element refers to a model that is not defined.
    #[error("{element}: model `{model}` is not defined (add a .model card)")]
    UnknownModel {
        /// The element.
        element: String,
        /// The missing model.
        model: String,
    },
    /// An element uses a model of the wrong kind.
    #[error("{element}: model `{model}` is a {found} model but a {expected} model is needed")]
    ModelKind {
        /// The element.
        element: String,
        /// The model name.
        model: String,
        /// The kind found.
        found: String,
        /// The kind needed.
        expected: String,
    },
    /// A controlled source or coupling refers to an element that is not there.
    #[error("{element}: controlling element `{control}` does not exist")]
    UnknownControl {
        /// The element.
        element: String,
        /// The missing controlling element.
        control: String,
    },
    /// A controlling element has the wrong kind.
    #[error("{element}: `{control}` must be {expected}")]
    WrongControlKind {
        /// The element.
        element: String,
        /// The controlling element.
        control: String,
        /// What it must be.
        expected: &'static str,
    },
    /// An `X` instance refers to a subcircuit that is not defined.
    #[error("{element}: subcircuit `{subckt}` is not defined")]
    UnknownSubckt {
        /// The instance.
        element: String,
        /// The missing subcircuit.
        subckt: String,
    },
    /// An `X` instance has the wrong number of nodes.
    #[error("{element}: subcircuit `{subckt}` has {expected} ports but {got} nodes were given")]
    SubcktPorts {
        /// The instance.
        element: String,
        /// The subcircuit.
        subckt: String,
        /// Ports in the definition.
        expected: usize,
        /// Nodes on the instance.
        got: usize,
    },
    /// Subcircuits instantiate each other in a loop or nest too deeply.
    #[error("subcircuit nesting is deeper than {MAX_SUBCKT_DEPTH} levels or recursive (at `{0}`)")]
    SubcktDepth(String),
    /// A value is not usable (non-finite, zero where forbidden, out of range).
    #[error("{element}: {message}")]
    BadValue {
        /// The element or card.
        element: String,
        /// What is wrong, and what is allowed.
        message: String,
    },
    /// The netlist is bigger than the engine accepts.
    #[error("netlist has more than {MAX_ELEMENTS} elements")]
    TooLarge,
}

fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_NAME_LEN
        && s.chars().all(|c| {
            !c.is_whitespace()
                && !c.is_control()
                && !matches!(c, '(' | ')' | ',' | '=' | ';' | '{' | '}' | '"' | '\'')
        })
}

/// A node, by index into a [`NodeTable`]. Index 0 is ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The ground node.
    pub const GROUND: NodeId = NodeId(0);
    /// True for ground.
    #[must_use]
    pub fn is_ground(self) -> bool {
        self.0 == 0
    }
}

/// Interned node names. Lookup ignores case; ground is always id 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeTable {
    names: Vec<String>,
    index: BTreeMap<String, NodeId>,
}

impl Default for NodeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeTable {
    /// A table holding only ground.
    #[must_use]
    pub fn new() -> Self {
        let mut index = BTreeMap::new();
        index.insert("0".to_string(), NodeId::GROUND);
        index.insert("gnd".to_string(), NodeId::GROUND);
        NodeTable {
            names: vec!["0".to_string()],
            index,
        }
    }

    /// Find or create a node.
    ///
    /// # Errors
    /// Fails for an invalid name or when the table is absurdly large.
    pub fn intern(&mut self, name: &str) -> Result<NodeId, NetlistError> {
        if !name_ok(name) {
            return Err(NetlistError::InvalidNodeName(name.to_string()));
        }
        let key = name.to_lowercase();
        if let Some(id) = self.index.get(&key) {
            return Ok(*id);
        }
        if self.names.len() >= MAX_ELEMENTS * 4 {
            return Err(NetlistError::TooLarge);
        }
        let id = NodeId(self.names.len() as u32);
        self.names.push(name.to_string());
        self.index.insert(key, id);
        Ok(id)
    }

    /// Look a node up without creating it.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<NodeId> {
        self.index.get(&name.to_lowercase()).copied()
    }

    /// The name of a node, or `"?"` for an id that is not in this table.
    #[must_use]
    pub fn name(&self, id: NodeId) -> &str {
        self.names.get(id.0 as usize).map_or("?", String::as_str)
    }

    /// Number of nodes including ground.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Never true: ground always exists.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// All names in id order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }
}

/// What kind of device a `.model` card describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModelKind {
    /// Diode (`D`).
    D,
    /// NPN bipolar transistor.
    Npn,
    /// PNP bipolar transistor.
    Pnp,
    /// N-channel MOSFET (level 1).
    Nmos,
    /// P-channel MOSFET (level 1).
    Pmos,
    /// Voltage-controlled switch with hysteresis.
    Sw,
    /// Op-amp macro-model (a Circuito extension, element letter `U`).
    OpAmp,
}

impl ModelKind {
    /// The keyword used on a `.model` card.
    #[must_use]
    pub fn keyword(self) -> &'static str {
        match self {
            ModelKind::D => "D",
            ModelKind::Npn => "NPN",
            ModelKind::Pnp => "PNP",
            ModelKind::Nmos => "NMOS",
            ModelKind::Pmos => "PMOS",
            ModelKind::Sw => "SW",
            ModelKind::OpAmp => "OPAMP",
        }
    }

    /// Parse a `.model` type keyword, ignoring case.
    #[must_use]
    pub fn from_keyword(s: &str) -> Option<Self> {
        Some(match s.to_ascii_uppercase().as_str() {
            "D" => ModelKind::D,
            "NPN" => ModelKind::Npn,
            "PNP" => ModelKind::Pnp,
            "NMOS" => ModelKind::Nmos,
            "PMOS" => ModelKind::Pmos,
            "SW" => ModelKind::Sw,
            "OPAMP" => ModelKind::OpAmp,
            _ => return None,
        })
    }

    /// The parameter names the simulator understands for this kind.
    #[must_use]
    pub fn known_params(self) -> &'static [&'static str] {
        match self {
            ModelKind::D => &["IS", "RS", "N", "TT", "CJO", "VJ", "M", "FC", "BV", "IBV"],
            ModelKind::Npn | ModelKind::Pnp => &[
                "IS", "BF", "BR", "NF", "NR", "VAF", "VAR", "RB", "RC", "RE", "CJE", "VJE", "MJE",
                "CJC", "VJC", "MJC", "FC", "TF", "TR",
            ],
            ModelKind::Nmos | ModelKind::Pmos => &[
                "LEVEL", "VTO", "KP", "GAMMA", "PHI", "LAMBDA", "RD", "RS", "CGSO", "CGDO", "CGBO",
                "CBD", "CBS", "W", "L",
            ],
            ModelKind::Sw => &["VT", "VH", "RON", "ROFF"],
            ModelKind::OpAmp => &["GAIN", "VMAX", "VMIN", "ROUT", "IDEAL"],
        }
    }
}

impl fmt::Display for ModelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.keyword())
    }
}

/// A `.model` card: a name, a kind and numeric parameters (names in upper case).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// Model name, upper case.
    pub name: String,
    /// Device kind.
    pub kind: ModelKind,
    /// Parameters by upper-case name.
    pub params: BTreeMap<String, f64>,
}

impl Model {
    /// A model with no parameters (all defaults).
    #[must_use]
    pub fn new(name: &str, kind: ModelKind) -> Self {
        Model {
            name: name.to_uppercase(),
            kind,
            params: BTreeMap::new(),
        }
    }

    /// Add a parameter and return the model (builder style).
    #[must_use]
    pub fn with(mut self, key: &str, value: f64) -> Self {
        self.params.insert(key.to_uppercase(), value);
        self
    }

    /// A parameter value or a default.
    #[must_use]
    pub fn get(&self, key: &str, default: f64) -> f64 {
        self.params.get(key).copied().unwrap_or(default)
    }

    /// Parameter names that [`ModelKind::known_params`] does not list.
    #[must_use]
    pub fn unknown_params(&self) -> Vec<String> {
        let known = self.kind.known_params();
        self.params
            .keys()
            .filter(|k| !known.contains(&k.as_str()))
            .cloned()
            .collect()
    }
}

/// One circuit element.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    /// Resistor `R`.
    Resistor {
        /// Name.
        name: String,
        /// First terminal.
        a: NodeId,
        /// Second terminal.
        b: NodeId,
        /// Resistance in ohms (non-zero).
        value: f64,
    },
    /// Capacitor `C`.
    Capacitor {
        /// Name.
        name: String,
        /// First terminal.
        a: NodeId,
        /// Second terminal.
        b: NodeId,
        /// Capacitance in farads.
        value: f64,
        /// Initial voltage `V(a) - V(b)` used with `uic`.
        ic: Option<f64>,
    },
    /// Inductor `L`.
    Inductor {
        /// Name.
        name: String,
        /// First terminal.
        a: NodeId,
        /// Second terminal.
        b: NodeId,
        /// Inductance in henries (non-zero).
        value: f64,
        /// Initial current from `a` to `b` used with `uic`.
        ic: Option<f64>,
    },
    /// Independent voltage source `V`; positive terminal `p`.
    VSource {
        /// Name.
        name: String,
        /// Positive terminal.
        p: NodeId,
        /// Negative terminal.
        n: NodeId,
        /// Value description.
        source: Source,
    },
    /// Independent current source `I`; current flows from `p` through the source to `n`.
    ISource {
        /// Name.
        name: String,
        /// Terminal the current leaves the circuit from (into the source).
        p: NodeId,
        /// Terminal the current returns to.
        n: NodeId,
        /// Value description.
        source: Source,
    },
    /// Diode `D`.
    Diode {
        /// Name.
        name: String,
        /// Anode.
        anode: NodeId,
        /// Cathode.
        cathode: NodeId,
        /// Model name.
        model: String,
        /// Area multiplier.
        area: f64,
    },
    /// Voltage-controlled voltage source `E`: `V(p,n) = gain * V(cp,cn)`.
    Vcvs {
        /// Name.
        name: String,
        /// Output positive.
        p: NodeId,
        /// Output negative.
        n: NodeId,
        /// Control positive.
        cp: NodeId,
        /// Control negative.
        cn: NodeId,
        /// Voltage gain.
        gain: f64,
    },
    /// Voltage-controlled current source `G`: current `gm * V(cp,cn)` from `p` to `n` through it.
    Vccs {
        /// Name.
        name: String,
        /// Output positive.
        p: NodeId,
        /// Output negative.
        n: NodeId,
        /// Control positive.
        cp: NodeId,
        /// Control negative.
        cn: NodeId,
        /// Transconductance in siemens.
        gm: f64,
    },
    /// Current-controlled current source `F`: current `gain * I(control)`.
    Cccs {
        /// Name.
        name: String,
        /// Output positive.
        p: NodeId,
        /// Output negative.
        n: NodeId,
        /// Name of the voltage source whose current controls this one.
        control: String,
        /// Current gain.
        gain: f64,
    },
    /// Current-controlled voltage source `H`: `V(p,n) = rm * I(control)`.
    Ccvs {
        /// Name.
        name: String,
        /// Output positive.
        p: NodeId,
        /// Output negative.
        n: NodeId,
        /// Name of the voltage source whose current controls this one.
        control: String,
        /// Transresistance in ohms.
        rm: f64,
    },
    /// Mutual inductance `K` between two inductors with coupling `k`.
    Mutual {
        /// Name.
        name: String,
        /// First inductor.
        l1: String,
        /// Second inductor.
        l2: String,
        /// Coupling coefficient, `0 < |k| < 1` (a value of exactly 1 is allowed).
        k: f64,
    },
    /// Voltage-controlled switch `S` with hysteresis (model kind `SW`).
    Switch {
        /// Name.
        name: String,
        /// Switched terminal.
        p: NodeId,
        /// Switched terminal.
        n: NodeId,
        /// Control positive.
        cp: NodeId,
        /// Control negative.
        cn: NodeId,
        /// Model name.
        model: String,
    },
    /// Bipolar transistor `Q`.
    Bjt {
        /// Name.
        name: String,
        /// Collector.
        c: NodeId,
        /// Base.
        b: NodeId,
        /// Emitter.
        e: NodeId,
        /// Model name.
        model: String,
        /// Area multiplier.
        area: f64,
    },
    /// MOSFET `M`, level 1.
    Mosfet {
        /// Name.
        name: String,
        /// Drain.
        d: NodeId,
        /// Gate.
        g: NodeId,
        /// Source.
        s: NodeId,
        /// Bulk.
        bulk: NodeId,
        /// Model name.
        model: String,
        /// Channel width in metres, if given on the card.
        w: Option<f64>,
        /// Channel length in metres, if given on the card.
        l: Option<f64>,
    },
    /// Op-amp macro-model `U`: inputs `inp`, `inn`, output `out` (model kind `OPAMP`).
    OpAmp {
        /// Name.
        name: String,
        /// Non-inverting input.
        inp: NodeId,
        /// Inverting input.
        inn: NodeId,
        /// Output.
        out: NodeId,
        /// Model name.
        model: String,
    },
    /// Subcircuit instance `X`.
    Instance {
        /// Name.
        name: String,
        /// Nodes in port order.
        nodes: Vec<NodeId>,
        /// Subcircuit name.
        subckt: String,
    },
}

impl Element {
    /// The element's name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Element::Resistor { name, .. }
            | Element::Capacitor { name, .. }
            | Element::Inductor { name, .. }
            | Element::VSource { name, .. }
            | Element::ISource { name, .. }
            | Element::Diode { name, .. }
            | Element::Vcvs { name, .. }
            | Element::Vccs { name, .. }
            | Element::Cccs { name, .. }
            | Element::Ccvs { name, .. }
            | Element::Mutual { name, .. }
            | Element::Switch { name, .. }
            | Element::Bjt { name, .. }
            | Element::Mosfet { name, .. }
            | Element::OpAmp { name, .. }
            | Element::Instance { name, .. } => name,
        }
    }

    fn name_mut(&mut self) -> &mut String {
        match self {
            Element::Resistor { name, .. }
            | Element::Capacitor { name, .. }
            | Element::Inductor { name, .. }
            | Element::VSource { name, .. }
            | Element::ISource { name, .. }
            | Element::Diode { name, .. }
            | Element::Vcvs { name, .. }
            | Element::Vccs { name, .. }
            | Element::Cccs { name, .. }
            | Element::Ccvs { name, .. }
            | Element::Mutual { name, .. }
            | Element::Switch { name, .. }
            | Element::Bjt { name, .. }
            | Element::Mosfet { name, .. }
            | Element::OpAmp { name, .. }
            | Element::Instance { name, .. } => name,
        }
    }

    /// The SPICE letter for this element kind.
    #[must_use]
    pub fn letter(&self) -> char {
        match self {
            Element::Resistor { .. } => 'R',
            Element::Capacitor { .. } => 'C',
            Element::Inductor { .. } => 'L',
            Element::VSource { .. } => 'V',
            Element::ISource { .. } => 'I',
            Element::Diode { .. } => 'D',
            Element::Vcvs { .. } => 'E',
            Element::Vccs { .. } => 'G',
            Element::Cccs { .. } => 'F',
            Element::Ccvs { .. } => 'H',
            Element::Mutual { .. } => 'K',
            Element::Switch { .. } => 'S',
            Element::Bjt { .. } => 'Q',
            Element::Mosfet { .. } => 'M',
            Element::OpAmp { .. } => 'U',
            Element::Instance { .. } => 'X',
        }
    }

    /// Every node this element touches, in card order.
    #[must_use]
    pub fn nodes(&self) -> Vec<NodeId> {
        match self {
            Element::Resistor { a, b, .. }
            | Element::Capacitor { a, b, .. }
            | Element::Inductor { a, b, .. } => vec![*a, *b],
            Element::VSource { p, n, .. }
            | Element::ISource { p, n, .. }
            | Element::Cccs { p, n, .. }
            | Element::Ccvs { p, n, .. } => vec![*p, *n],
            Element::Diode { anode, cathode, .. } => vec![*anode, *cathode],
            Element::Vcvs { p, n, cp, cn, .. }
            | Element::Vccs { p, n, cp, cn, .. }
            | Element::Switch { p, n, cp, cn, .. } => vec![*p, *n, *cp, *cn],
            Element::Mutual { .. } => vec![],
            Element::Bjt { c, b, e, .. } => vec![*c, *b, *e],
            Element::Mosfet { d, g, s, bulk, .. } => vec![*d, *g, *s, *bulk],
            Element::OpAmp { inp, inn, out, .. } => vec![*inp, *inn, *out],
            Element::Instance { nodes, .. } => nodes.clone(),
        }
    }

    /// Apply `f` to every node reference (used by subcircuit expansion).
    fn map_nodes(&mut self, f: &mut dyn FnMut(NodeId) -> NodeId) {
        match self {
            Element::Resistor { a, b, .. }
            | Element::Capacitor { a, b, .. }
            | Element::Inductor { a, b, .. } => {
                *a = f(*a);
                *b = f(*b);
            }
            Element::VSource { p, n, .. }
            | Element::ISource { p, n, .. }
            | Element::Cccs { p, n, .. }
            | Element::Ccvs { p, n, .. } => {
                *p = f(*p);
                *n = f(*n);
            }
            Element::Diode { anode, cathode, .. } => {
                *anode = f(*anode);
                *cathode = f(*cathode);
            }
            Element::Vcvs { p, n, cp, cn, .. }
            | Element::Vccs { p, n, cp, cn, .. }
            | Element::Switch { p, n, cp, cn, .. } => {
                *p = f(*p);
                *n = f(*n);
                *cp = f(*cp);
                *cn = f(*cn);
            }
            Element::Mutual { .. } => {}
            Element::Bjt { c, b, e, .. } => {
                *c = f(*c);
                *b = f(*b);
                *e = f(*e);
            }
            Element::Mosfet { d, g, s, bulk, .. } => {
                *d = f(*d);
                *g = f(*g);
                *s = f(*s);
                *bulk = f(*bulk);
            }
            Element::OpAmp { inp, inn, out, .. } => {
                *inp = f(*inp);
                *inn = f(*inn);
                *out = f(*out);
            }
            Element::Instance { nodes, .. } => {
                for n in nodes {
                    *n = f(*n);
                }
            }
        }
    }

    /// Upper-case the name and every reference to another element, model or subcircuit.
    pub(crate) fn normalize(&mut self) {
        let up = |s: &mut String| *s = s.to_uppercase();
        up(self.name_mut());
        match self {
            Element::Cccs { control, .. } | Element::Ccvs { control, .. } => up(control),
            Element::Mutual { l1, l2, .. } => {
                up(l1);
                up(l2);
            }
            Element::Diode { model, .. }
            | Element::Switch { model, .. }
            | Element::Bjt { model, .. }
            | Element::Mosfet { model, .. }
            | Element::OpAmp { model, .. } => up(model),
            Element::Instance { subckt, .. } => up(subckt),
            _ => {}
        }
    }

    /// Append `.suffix` to every element-name reference (own name, control sources, couplings).
    fn rename_with_suffix(&mut self, suffix: &str) {
        let add = |s: &mut String| {
            s.push('.');
            s.push_str(suffix);
        };
        add(self.name_mut());
        match self {
            Element::Cccs { control, .. } | Element::Ccvs { control, .. } => add(control),
            Element::Mutual { l1, l2, .. } => {
                add(l1);
                add(l2);
            }
            _ => {}
        }
    }
}

/// How an AC sweep spaces its frequencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sweep {
    /// Points per decade.
    Dec,
    /// Points per octave.
    Oct,
    /// Total number of linearly spaced points.
    Lin,
}

impl Sweep {
    /// The `.ac` keyword.
    #[must_use]
    pub fn keyword(self) -> &'static str {
        match self {
            Sweep::Dec => "dec",
            Sweep::Oct => "oct",
            Sweep::Lin => "lin",
        }
    }
}

/// An analysis request (`.op`, `.dc`, `.ac`, `.tran`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Analysis {
    /// DC operating point.
    Op,
    /// DC sweep of one source.
    Dc {
        /// Source name.
        source: String,
        /// First value.
        start: f64,
        /// Last value.
        stop: f64,
        /// Increment (positive; the direction follows `start`/`stop`).
        step: f64,
    },
    /// Small-signal frequency sweep.
    Ac {
        /// Spacing.
        sweep: Sweep,
        /// Points per decade/octave, or total for `Lin`.
        points: u32,
        /// Start frequency in hertz.
        fstart: f64,
        /// Stop frequency in hertz.
        fstop: f64,
    },
    /// Transient analysis.
    Tran {
        /// Suggested print/plot step (also the default step ceiling is derived from it).
        tstep: f64,
        /// End time.
        tstop: f64,
        /// Time at which results start being recorded.
        tstart: f64,
        /// Largest internal time step, if given.
        tmax: Option<f64>,
        /// Use initial conditions instead of solving for an operating point first.
        uic: bool,
    },
}

impl Analysis {
    /// Check that the request is usable.
    ///
    /// # Errors
    /// Describes the offending number and the range that is allowed.
    pub fn validate(&self) -> Result<(), NetlistError> {
        let bad = |m: &str| {
            Err(NetlistError::BadValue {
                element: self.keyword().to_string(),
                message: m.to_string(),
            })
        };
        match self {
            Analysis::Op => Ok(()),
            Analysis::Dc {
                start, stop, step, ..
            } => {
                if ![*start, *stop, *step].iter().all(|x| x.is_finite()) {
                    return bad("sweep values must be finite");
                }
                if *step <= 0.0 {
                    return bad("the increment must be positive");
                }
                if ((stop - start).abs() / step) > 1.0e6 {
                    return bad("the sweep has more than 1,000,000 points");
                }
                Ok(())
            }
            Analysis::Ac {
                points,
                fstart,
                fstop,
                sweep,
            } => {
                if !fstart.is_finite() || !fstop.is_finite() || *fstart <= 0.0 || fstop < fstart {
                    return bad("need 0 < fstart <= fstop (hertz, finite)");
                }
                if *points == 0 {
                    return bad("the number of points must be at least 1");
                }
                if *sweep != Sweep::Lin && (fstop / fstart).log10() * f64::from(*points) > 1.0e6 {
                    return bad("the sweep has more than 1,000,000 points");
                }
                if *sweep == Sweep::Lin && *points > 1_000_000 {
                    return bad("the sweep has more than 1,000,000 points");
                }
                Ok(())
            }
            Analysis::Tran {
                tstep,
                tstop,
                tstart,
                tmax,
                ..
            } => {
                if ![*tstep, *tstop, *tstart].iter().all(|x| x.is_finite()) {
                    return bad("times must be finite");
                }
                if *tstep <= 0.0 || *tstop <= 0.0 {
                    return bad("tstep and tstop must be positive");
                }
                if *tstart < 0.0 || *tstart >= *tstop {
                    return bad("need 0 <= tstart < tstop");
                }
                if let Some(m) = tmax {
                    if !m.is_finite() || *m <= 0.0 {
                        return bad("tmax must be positive");
                    }
                }
                Ok(())
            }
        }
    }

    /// The SPICE dot-command for this analysis.
    #[must_use]
    pub fn keyword(&self) -> &'static str {
        match self {
            Analysis::Op => ".op",
            Analysis::Dc { .. } => ".dc",
            Analysis::Ac { .. } => ".ac",
            Analysis::Tran { .. } => ".tran",
        }
    }
}

impl fmt::Display for Analysis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = format_value;
        match self {
            Analysis::Op => write!(f, ".op"),
            Analysis::Dc {
                source,
                start,
                stop,
                step,
            } => {
                write!(f, ".dc {} {} {} {}", source, v(*start), v(*stop), v(*step))
            }
            Analysis::Ac {
                sweep,
                points,
                fstart,
                fstop,
            } => {
                write!(
                    f,
                    ".ac {} {} {} {}",
                    sweep.keyword(),
                    points,
                    v(*fstart),
                    v(*fstop)
                )
            }
            Analysis::Tran {
                tstep,
                tstop,
                tstart,
                tmax,
                uic,
            } => {
                write!(f, ".tran {} {} {}", v(*tstep), v(*tstop), v(*tstart))?;
                if let Some(m) = tmax {
                    write!(f, " {}", v(*m))?;
                }
                if *uic {
                    write!(f, " uic")?;
                }
                Ok(())
            }
        }
    }
}

/// A `.subckt` definition. Ports are nodes `1..=ports.len()` of the local table.
#[derive(Debug, Clone, PartialEq)]
pub struct Subckt {
    /// Name, upper case.
    pub name: String,
    /// Port names in order.
    pub ports: Vec<String>,
    /// Local node names (ground is shared with the parent circuit).
    pub nodes: NodeTable,
    /// The body.
    pub elements: Vec<Element>,
}

impl Subckt {
    /// Append a body element (names are upper-cased like [`Netlist::add`]).
    ///
    /// # Errors
    /// A duplicate or invalid element name.
    pub fn add(&mut self, mut e: Element) -> Result<(), NetlistError> {
        e.normalize();
        if !name_ok(e.name()) {
            return Err(NetlistError::InvalidName(e.name().to_string()));
        }
        if self.elements.iter().any(|x| x.name() == e.name()) {
            return Err(NetlistError::DuplicateElement(e.name().to_string()));
        }
        if self.elements.len() >= MAX_ELEMENTS {
            return Err(NetlistError::TooLarge);
        }
        self.elements.push(e);
        Ok(())
    }

    /// Start a definition with the given port names.
    ///
    /// # Errors
    /// Fails on an invalid or repeated port name, or on ground used as a port.
    pub fn new(name: &str, ports: &[&str]) -> Result<Self, NetlistError> {
        if !name_ok(name) {
            return Err(NetlistError::InvalidName(name.to_string()));
        }
        let mut nodes = NodeTable::new();
        for p in ports {
            let before = nodes.len();
            let id = nodes.intern(p)?;
            if id.is_ground() || nodes.len() == before {
                return Err(NetlistError::BadValue {
                    element: name.to_uppercase(),
                    message: format!("port `{p}` is ground or repeated; ports must be distinct, non-ground nodes"),
                });
            }
        }
        Ok(Subckt {
            name: name.to_uppercase(),
            ports: ports.iter().map(|s| (*s).to_string()).collect(),
            nodes,
            elements: Vec::new(),
        })
    }
}

/// A whole circuit description.
#[derive(Debug, Clone, PartialEq)]
pub struct Netlist {
    /// The title line.
    pub title: String,
    /// Node names.
    pub nodes: NodeTable,
    /// Elements in file order.
    pub elements: Vec<Element>,
    /// `.model` cards by upper-case name.
    pub models: BTreeMap<String, Model>,
    /// `.subckt` definitions by upper-case name.
    pub subckts: BTreeMap<String, Subckt>,
    /// Requested analyses in file order.
    pub analyses: Vec<Analysis>,
    /// `.param` values (already evaluated), in definition order.
    pub params: Vec<(String, f64)>,
    /// `.options` values by upper-case name (`RELTOL`, `ABSTOL`, `VNTOL`, `GMIN`, `TEMP`, ...).
    pub options: BTreeMap<String, f64>,
}

impl Default for Netlist {
    fn default() -> Self {
        Netlist::new("")
    }
}

impl Netlist {
    /// An empty netlist with a title.
    #[must_use]
    pub fn new(title: &str) -> Self {
        Netlist {
            title: title.to_string(),
            nodes: NodeTable::new(),
            elements: Vec::new(),
            models: BTreeMap::new(),
            subckts: BTreeMap::new(),
            analyses: Vec::new(),
            params: Vec::new(),
            options: BTreeMap::new(),
        }
    }

    /// Find or create a node by name.
    ///
    /// # Errors
    /// For an invalid node name.
    pub fn node(&mut self, name: &str) -> Result<NodeId, NetlistError> {
        self.nodes.intern(name)
    }

    /// Append an element. Names are upper-cased; a repeated name is an error.
    ///
    /// # Errors
    /// Duplicate or invalid names, or a netlist that is too large.
    pub fn add(&mut self, mut e: Element) -> Result<(), NetlistError> {
        e.normalize();
        let up = e.name().to_string();
        if !name_ok(&up) {
            return Err(NetlistError::InvalidName(up));
        }
        if self.elements.len() >= MAX_ELEMENTS {
            return Err(NetlistError::TooLarge);
        }
        if self.element(&up).is_some() {
            return Err(NetlistError::DuplicateElement(up));
        }
        self.elements.push(e);
        Ok(())
    }

    /// Add a model card.
    ///
    /// # Errors
    /// A duplicate or invalid name.
    pub fn add_model(&mut self, m: Model) -> Result<(), NetlistError> {
        if !name_ok(&m.name) {
            return Err(NetlistError::InvalidName(m.name));
        }
        if self.models.contains_key(&m.name) {
            return Err(NetlistError::DuplicateModel(m.name));
        }
        self.models.insert(m.name.clone(), m);
        Ok(())
    }

    /// Add a subcircuit definition.
    ///
    /// # Errors
    /// A duplicate name.
    pub fn add_subckt(&mut self, s: Subckt) -> Result<(), NetlistError> {
        if self.subckts.contains_key(&s.name) {
            return Err(NetlistError::DuplicateSubckt(s.name));
        }
        self.subckts.insert(s.name.clone(), s);
        Ok(())
    }

    /// Find an element by (case-insensitive) name.
    #[must_use]
    pub fn element(&self, name: &str) -> Option<&Element> {
        let up = name.to_uppercase();
        self.elements.iter().find(|e| e.name() == up)
    }

    /// Check the whole netlist: values, models, control references, subcircuit instances and
    /// analyses.
    ///
    /// # Errors
    /// The first problem found, naming the element involved.
    pub fn validate(&self) -> Result<(), NetlistError> {
        let mut names = std::collections::HashSet::new();
        for e in &self.elements {
            if !names.insert(e.name()) {
                return Err(NetlistError::DuplicateElement(e.name().to_string()));
            }
        }
        validate_elements(&self.elements, &self.models, &self.subckts, None)?;
        for s in self.subckts.values() {
            let mut local = std::collections::HashSet::new();
            for e in &s.elements {
                if !local.insert(e.name()) {
                    return Err(NetlistError::DuplicateElement(format!(
                        "{} (in .subckt {})",
                        e.name(),
                        s.name
                    )));
                }
            }
            validate_elements(&s.elements, &self.models, &self.subckts, Some(s))?;
        }
        for a in &self.analyses {
            a.validate()?;
            if let Analysis::Dc { source, .. } = a {
                match self.element(source) {
                    Some(Element::VSource { .. } | Element::ISource { .. }) => {}
                    _ => {
                        return Err(NetlistError::UnknownControl {
                            element: ".dc".into(),
                            control: source.clone(),
                        })
                    }
                }
            }
        }
        Ok(())
    }

    /// Expand every subcircuit instance into flat elements. The result has no subcircuits.
    ///
    /// Expanded names are `<inner>.<instance path>` (so `R1` inside `X1` becomes `R1.X1`) and
    /// internal nodes become `<instance path>.<node>`.
    ///
    /// # Errors
    /// Unknown subcircuit, wrong port count, nesting deeper than 16 levels or a recursive
    /// definition, or too many elements.
    pub fn flatten(&self) -> Result<Netlist, NetlistError> {
        let mut out = Netlist {
            title: self.title.clone(),
            nodes: self.nodes.clone(),
            elements: Vec::new(),
            models: self.models.clone(),
            subckts: BTreeMap::new(),
            analyses: self.analyses.clone(),
            params: self.params.clone(),
            options: self.options.clone(),
        };
        let mut stack: Vec<String> = Vec::new();
        for e in &self.elements {
            match e {
                Element::Instance {
                    name,
                    nodes,
                    subckt,
                } => {
                    self.expand(&mut out, name, nodes, subckt, &mut stack)?;
                }
                other => push_checked(&mut out, other.clone())?,
            }
        }
        Ok(out)
    }

    fn expand(
        &self,
        out: &mut Netlist,
        inst: &str,
        actual: &[NodeId],
        subckt: &str,
        stack: &mut Vec<String>,
    ) -> Result<(), NetlistError> {
        let def = self.subckts.get(&subckt.to_uppercase()).ok_or_else(|| {
            NetlistError::UnknownSubckt {
                element: inst.to_string(),
                subckt: subckt.to_string(),
            }
        })?;
        if def.ports.len() != actual.len() {
            return Err(NetlistError::SubcktPorts {
                element: inst.to_string(),
                subckt: def.name.clone(),
                expected: def.ports.len(),
                got: actual.len(),
            });
        }
        if stack.len() >= MAX_SUBCKT_DEPTH || stack.contains(&def.name) {
            return Err(NetlistError::SubcktDepth(inst.to_string()));
        }
        stack.push(def.name.clone());
        // Local node id -> outer node id. Ground stays ground, ports map to the actual nodes.
        let mut map: Vec<NodeId> = Vec::with_capacity(def.nodes.len());
        for (i, local) in def.nodes.names().enumerate() {
            if i == 0 {
                map.push(NodeId::GROUND);
            } else if i <= actual.len() {
                map.push(actual[i - 1]);
            } else {
                map.push(out.nodes.intern(&format!("{inst}.{local}"))?);
            }
        }
        for e in &def.elements {
            let mut e = e.clone();
            e.map_nodes(&mut |n| map.get(n.0 as usize).copied().unwrap_or(NodeId::GROUND));
            match e {
                Element::Instance {
                    name,
                    nodes,
                    subckt,
                } => {
                    let nested = format!("{name}.{inst}");
                    self.expand(out, &nested, &nodes, &subckt, stack)?;
                }
                mut other => {
                    other.rename_with_suffix(inst);
                    push_checked(out, other)?;
                }
            }
        }
        stack.pop();
        Ok(())
    }
}

fn push_checked(out: &mut Netlist, e: Element) -> Result<(), NetlistError> {
    if out.elements.len() >= MAX_ELEMENTS {
        return Err(NetlistError::TooLarge);
    }
    out.elements.push(e);
    Ok(())
}

fn bad(e: &str, m: impl Into<String>) -> NetlistError {
    NetlistError::BadValue {
        element: e.to_string(),
        message: m.into(),
    }
}

fn check_model(
    models: &BTreeMap<String, Model>,
    element: &str,
    model: &str,
    ok_kinds: &[ModelKind],
) -> Result<(), NetlistError> {
    let m = models
        .get(&model.to_uppercase())
        .ok_or_else(|| NetlistError::UnknownModel {
            element: element.to_string(),
            model: model.to_string(),
        })?;
    if !ok_kinds.contains(&m.kind) {
        return Err(NetlistError::ModelKind {
            element: element.to_string(),
            model: model.to_string(),
            found: m.kind.keyword().to_string(),
            expected: ok_kinds
                .iter()
                .map(|k| k.keyword())
                .collect::<Vec<_>>()
                .join(" or "),
        });
    }
    for (k, v) in &m.params {
        if !v.is_finite() {
            return Err(bad(
                &format!(".model {}", m.name),
                format!("parameter {k} is not finite"),
            ));
        }
    }
    Ok(())
}

fn validate_elements(
    elements: &[Element],
    models: &BTreeMap<String, Model>,
    subckts: &BTreeMap<String, Subckt>,
    within: Option<&Subckt>,
) -> Result<(), NetlistError> {
    let find = |n: &str| elements.iter().find(|e| e.name() == n.to_uppercase());
    let fin = |name: &str, what: &str, x: f64| {
        if x.is_finite() {
            Ok(())
        } else {
            Err(bad(name, format!("{what} must be a finite number")))
        }
    };
    for e in elements {
        let name = e.name();
        match e {
            Element::Resistor { value, .. } => {
                fin(name, "resistance", *value)?;
                if *value == 0.0 {
                    return Err(bad(
                        name,
                        "resistance is zero; use a 0 V source (or a wire) for a short circuit",
                    ));
                }
            }
            Element::Capacitor { value, ic, .. } => {
                fin(name, "capacitance", *value)?;
                if let Some(i) = ic {
                    fin(name, "IC", *i)?;
                }
            }
            Element::Inductor { value, ic, .. } => {
                fin(name, "inductance", *value)?;
                if *value == 0.0 {
                    return Err(bad(
                        name,
                        "inductance is zero; use a 0 V source for a short circuit",
                    ));
                }
                if let Some(i) = ic {
                    fin(name, "IC", *i)?;
                }
            }
            Element::VSource { source, .. } | Element::ISource { source, .. } => {
                fin(name, "DC value", source.dc.unwrap_or(0.0))?;
            }
            Element::Diode { model, area, .. } => {
                check_model(models, name, model, &[ModelKind::D])?;
                if !area.is_finite() || *area <= 0.0 {
                    return Err(bad(name, "AREA must be positive"));
                }
            }
            Element::Vcvs { gain, .. } => fin(name, "gain", *gain)?,
            Element::Vccs { gm, .. } => fin(name, "transconductance", *gm)?,
            Element::Cccs { control, gain, .. } => {
                fin(name, "gain", *gain)?;
                match find(control) {
                    Some(Element::VSource { .. }) => {}
                    Some(_) => {
                        return Err(NetlistError::WrongControlKind {
                            element: name.to_string(),
                            control: control.clone(),
                            expected: "a voltage source (its current is the control)",
                        })
                    }
                    None => {
                        return Err(NetlistError::UnknownControl {
                            element: name.to_string(),
                            control: control.clone(),
                        })
                    }
                }
            }
            Element::Ccvs { control, rm, .. } => {
                fin(name, "transresistance", *rm)?;
                match find(control) {
                    Some(Element::VSource { .. }) => {}
                    Some(_) => {
                        return Err(NetlistError::WrongControlKind {
                            element: name.to_string(),
                            control: control.clone(),
                            expected: "a voltage source (its current is the control)",
                        })
                    }
                    None => {
                        return Err(NetlistError::UnknownControl {
                            element: name.to_string(),
                            control: control.clone(),
                        })
                    }
                }
            }
            Element::Mutual { l1, l2, k, .. } => {
                if !k.is_finite() || k.abs() > 1.0 || *k == 0.0 {
                    return Err(bad(name, "coupling k must satisfy 0 < |k| <= 1"));
                }
                if l1.eq_ignore_ascii_case(l2) {
                    return Err(bad(
                        name,
                        "a mutual inductance needs two different inductors",
                    ));
                }
                for l in [l1, l2] {
                    match find(l) {
                        Some(Element::Inductor { .. }) => {}
                        Some(_) => {
                            return Err(NetlistError::WrongControlKind {
                                element: name.to_string(),
                                control: l.clone(),
                                expected: "an inductor",
                            })
                        }
                        None => {
                            return Err(NetlistError::UnknownControl {
                                element: name.to_string(),
                                control: l.clone(),
                            })
                        }
                    }
                }
            }
            Element::Switch { model, .. } => check_model(models, name, model, &[ModelKind::Sw])?,
            Element::Bjt { model, area, .. } => {
                check_model(models, name, model, &[ModelKind::Npn, ModelKind::Pnp])?;
                if !area.is_finite() || *area <= 0.0 {
                    return Err(bad(name, "AREA must be positive"));
                }
            }
            Element::Mosfet { model, w, l, .. } => {
                check_model(models, name, model, &[ModelKind::Nmos, ModelKind::Pmos])?;
                for (what, v) in [("W", w), ("L", l)] {
                    if let Some(x) = v {
                        if !x.is_finite() || *x <= 0.0 {
                            return Err(bad(name, format!("{what} must be positive")));
                        }
                    }
                }
            }
            Element::OpAmp { model, .. } => check_model(models, name, model, &[ModelKind::OpAmp])?,
            Element::Instance { nodes, subckt, .. } => {
                let def = subckts.get(&subckt.to_uppercase()).ok_or_else(|| {
                    NetlistError::UnknownSubckt {
                        element: name.to_string(),
                        subckt: subckt.clone(),
                    }
                })?;
                if def.ports.len() != nodes.len() {
                    return Err(NetlistError::SubcktPorts {
                        element: name.to_string(),
                        subckt: def.name.clone(),
                        expected: def.ports.len(),
                        got: nodes.len(),
                    });
                }
                if let Some(w) = within {
                    if w.name == def.name {
                        return Err(NetlistError::SubcktDepth(name.to_string()));
                    }
                }
            }
        }
    }
    Ok(())
}

fn write_element(out: &mut String, e: &Element, nodes: &NodeTable) {
    let n = |id: &NodeId| nodes.name(*id).to_string();
    let v = format_value;
    let _ = match e {
        Element::Resistor { name, a, b, value } => {
            write!(out, "{name} {} {} {}", n(a), n(b), v(*value))
        }
        Element::Capacitor {
            name,
            a,
            b,
            value,
            ic,
        }
        | Element::Inductor {
            name,
            a,
            b,
            value,
            ic,
        } => {
            let _ = write!(out, "{name} {} {} {}", n(a), n(b), v(*value));
            match ic {
                Some(i) => write!(out, " IC={}", v(*i)),
                None => Ok(()),
            }
        }
        Element::VSource {
            name,
            p,
            n: nn,
            source,
        }
        | Element::ISource {
            name,
            p,
            n: nn,
            source,
        } => {
            write!(out, "{name} {} {} {source}", n(p), n(nn))
        }
        Element::Diode {
            name,
            anode,
            cathode,
            model,
            area,
        } => {
            let _ = write!(out, "{name} {} {} {model}", n(anode), n(cathode));
            if *area != 1.0 {
                let _ = write!(out, " AREA={}", v(*area));
            }
            Ok(())
        }
        Element::Vcvs {
            name,
            p,
            n: nn,
            cp,
            cn,
            gain,
        } => {
            write!(
                out,
                "{name} {} {} {} {} {}",
                n(p),
                n(nn),
                n(cp),
                n(cn),
                v(*gain)
            )
        }
        Element::Vccs {
            name,
            p,
            n: nn,
            cp,
            cn,
            gm,
        } => {
            write!(
                out,
                "{name} {} {} {} {} {}",
                n(p),
                n(nn),
                n(cp),
                n(cn),
                v(*gm)
            )
        }
        Element::Cccs {
            name,
            p,
            n: nn,
            control,
            gain,
        } => {
            write!(out, "{name} {} {} {control} {}", n(p), n(nn), v(*gain))
        }
        Element::Ccvs {
            name,
            p,
            n: nn,
            control,
            rm,
        } => {
            write!(out, "{name} {} {} {control} {}", n(p), n(nn), v(*rm))
        }
        Element::Mutual { name, l1, l2, k } => write!(out, "{name} {l1} {l2} {}", v(*k)),
        Element::Switch {
            name,
            p,
            n: nn,
            cp,
            cn,
            model,
        } => {
            write!(out, "{name} {} {} {} {} {model}", n(p), n(nn), n(cp), n(cn))
        }
        Element::Bjt {
            name,
            c,
            b,
            e,
            model,
            area,
        } => {
            let _ = write!(out, "{name} {} {} {} {model}", n(c), n(b), n(e));
            if *area != 1.0 {
                let _ = write!(out, " AREA={}", v(*area));
            }
            Ok(())
        }
        Element::Mosfet {
            name,
            d,
            g,
            s,
            bulk,
            model,
            w,
            l,
        } => {
            let _ = write!(out, "{name} {} {} {} {} {model}", n(d), n(g), n(s), n(bulk));
            if let Some(w) = w {
                let _ = write!(out, " W={}", v(*w));
            }
            if let Some(l) = l {
                let _ = write!(out, " L={}", v(*l));
            }
            Ok(())
        }
        Element::OpAmp {
            name,
            inp,
            inn,
            out: o,
            model,
        } => {
            write!(out, "{name} {} {} {} {model}", n(inp), n(inn), n(o))
        }
        Element::Instance {
            name,
            nodes: ns,
            subckt,
        } => {
            let _ = write!(out, "{name}");
            for x in ns {
                let _ = write!(out, " {}", n(x));
            }
            write!(out, " {subckt}")
        }
    };
    out.push('\n');
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".model {} {}", self.name, self.kind.keyword())?;
        if !self.params.is_empty() {
            write!(f, "(")?;
            for (i, (k, v)) in self.params.iter().enumerate() {
                if i > 0 {
                    write!(f, " ")?;
                }
                write!(f, "{k}={}", format_value(*v))?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }
}

impl fmt::Display for Netlist {
    /// Canonical SPICE text; reading it back with `circuito-spice` gives an equal netlist.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        s.push_str(&self.title.replace(['\n', '\r'], " "));
        s.push('\n');
        for (k, v) in &self.params {
            let _ = writeln!(s, ".param {k}={}", format_value(*v));
        }
        if !self.options.is_empty() {
            s.push_str(".options");
            for (k, v) in &self.options {
                let _ = write!(s, " {k}={}", format_value(*v));
            }
            s.push('\n');
        }
        for m in self.models.values() {
            let _ = writeln!(s, "{m}");
        }
        for d in self.subckts.values() {
            let _ = write!(s, ".subckt {}", d.name);
            for p in &d.ports {
                let _ = write!(s, " {p}");
            }
            s.push('\n');
            for e in &d.elements {
                write_element(&mut s, e, &d.nodes);
            }
            let _ = writeln!(s, ".ends {}", d.name);
        }
        for e in &self.elements {
            write_element(&mut s, e, &self.nodes);
        }
        for a in &self.analyses {
            let _ = writeln!(s, "{a}");
        }
        s.push_str(".end\n");
        f.write_str(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn divider() -> Netlist {
        let mut n = Netlist::new("divider");
        let (a, b) = (n.node("in").unwrap(), n.node("out").unwrap());
        n.add(Element::VSource {
            name: "v1".into(),
            p: a,
            n: NodeId::GROUND,
            source: Source::dc(10.0),
        })
        .unwrap();
        n.add(Element::Resistor {
            name: "r1".into(),
            a,
            b,
            value: 1000.0,
        })
        .unwrap();
        n.add(Element::Resistor {
            name: "r2".into(),
            a: b,
            b: NodeId::GROUND,
            value: 1000.0,
        })
        .unwrap();
        n
    }

    #[test]
    fn nodes_are_interned_case_insensitively() {
        let mut t = NodeTable::new();
        let a = t.intern("Out").unwrap();
        assert_eq!(t.intern("OUT").unwrap(), a);
        assert_eq!(t.name(a), "Out");
        assert_eq!(t.intern("GND").unwrap(), NodeId::GROUND);
        assert_eq!(t.intern("0").unwrap(), NodeId::GROUND);
        assert!(t.intern("").is_err());
        assert!(t.intern("a b").is_err());
        assert!(t.intern("a(b").is_err());
        assert_eq!(t.name(NodeId(99)), "?");
    }

    #[test]
    fn duplicate_and_invalid_elements_are_rejected() {
        let mut n = divider();
        let e = Element::Resistor {
            name: "R1".into(),
            a: NodeId::GROUND,
            b: NodeId::GROUND,
            value: 1.0,
        };
        assert!(matches!(n.add(e), Err(NetlistError::DuplicateElement(_))));
        n.add(Element::Resistor {
            name: "r9".into(),
            a: NodeId(1),
            b: NodeId(2),
            value: 0.0,
        })
        .unwrap();
        let err = n.validate().unwrap_err();
        assert!(err.to_string().contains("R9"), "{err}");
    }

    #[test]
    fn display_is_deterministic() {
        let n = divider();
        let text = n.to_string();
        assert_eq!(
            text,
            "divider\nV1 in 0 DC 10\nR1 in out 1k\nR2 out 0 1k\n.end\n"
        );
        assert_eq!(text, n.to_string());
    }

    #[test]
    fn model_and_control_validation() {
        let mut n = divider();
        n.add(Element::Diode {
            name: "d1".into(),
            anode: NodeId(2),
            cathode: NodeId(0),
            model: "dm".into(),
            area: 1.0,
        })
        .unwrap();
        assert!(matches!(
            n.validate(),
            Err(NetlistError::UnknownModel { .. })
        ));
        n.add_model(Model::new("dm", ModelKind::Npn)).unwrap();
        assert!(matches!(n.validate(), Err(NetlistError::ModelKind { .. })));
        n.models.insert("DM".into(), Model::new("dm", ModelKind::D));
        n.validate().unwrap();
        n.add(Element::Cccs {
            name: "f1".into(),
            p: NodeId(1),
            n: NodeId(0),
            control: "r1".into(),
            gain: 2.0,
        })
        .unwrap();
        assert!(matches!(
            n.validate(),
            Err(NetlistError::WrongControlKind { .. })
        ));
    }

    #[test]
    fn flatten_expands_nested_subcircuits() {
        let mut n = Netlist::new("t");
        let mut half = Subckt::new("half", &["a", "b"]).unwrap();
        let mid = half.nodes.intern("mid").unwrap();
        half.elements.push(Element::Resistor {
            name: "R1".into(),
            a: NodeId(1),
            b: mid,
            value: 1.0,
        });
        half.elements.push(Element::Resistor {
            name: "R2".into(),
            a: mid,
            b: NodeId(2),
            value: 2.0,
        });
        n.add_subckt(half).unwrap();
        let mut full = Subckt::new("full", &["p", "q"]).unwrap();
        let m2 = full.nodes.intern("m2").unwrap();
        full.elements.push(Element::Instance {
            name: "X1".into(),
            nodes: vec![NodeId(1), m2],
            subckt: "HALF".into(),
        });
        full.elements.push(Element::Instance {
            name: "X2".into(),
            nodes: vec![m2, NodeId(2)],
            subckt: "HALF".into(),
        });
        n.add_subckt(full).unwrap();
        let (i, o) = (n.node("i").unwrap(), n.node("o").unwrap());
        n.add(Element::Instance {
            name: "XA".into(),
            nodes: vec![i, o],
            subckt: "full".into(),
        })
        .unwrap();
        n.validate().unwrap();
        let flat = n.flatten().unwrap();
        assert_eq!(flat.elements.len(), 4);
        assert!(flat.subckts.is_empty());
        let names: Vec<&str> = flat.elements.iter().map(Element::name).collect();
        assert_eq!(names, ["R1.X1.XA", "R2.X1.XA", "R1.X2.XA", "R2.X2.XA"]);
        // Total resistance i..o is 1+2+1+2 through three internal nodes.
        assert!(flat.nodes.lookup("XA.m2").is_some());
        assert!(flat.nodes.lookup("X1.XA.mid").is_some());
    }

    #[test]
    fn flatten_rejects_recursion_and_bad_instances() {
        let mut n = Netlist::new("t");
        let mut a = Subckt::new("a", &["p"]).unwrap();
        a.elements.push(Element::Instance {
            name: "X1".into(),
            nodes: vec![NodeId(1)],
            subckt: "A".into(),
        });
        n.add_subckt(a).unwrap();
        let p = n.node("p").unwrap();
        n.add(Element::Instance {
            name: "X9".into(),
            nodes: vec![p],
            subckt: "a".into(),
        })
        .unwrap();
        assert!(matches!(n.flatten(), Err(NetlistError::SubcktDepth(_))));
        let mut m = Netlist::new("t");
        let p = m.node("p").unwrap();
        m.add(Element::Instance {
            name: "X1".into(),
            nodes: vec![p],
            subckt: "none".into(),
        })
        .unwrap();
        assert!(matches!(
            m.flatten(),
            Err(NetlistError::UnknownSubckt { .. })
        ));
        let mut k = Netlist::new("t");
        k.add_subckt(Subckt::new("two", &["a", "b"]).unwrap())
            .unwrap();
        let p = k.node("p").unwrap();
        k.add(Element::Instance {
            name: "X1".into(),
            nodes: vec![p],
            subckt: "two".into(),
        })
        .unwrap();
        assert!(matches!(k.flatten(), Err(NetlistError::SubcktPorts { .. })));
    }

    #[test]
    fn analyses_validate() {
        assert!(Analysis::Op.validate().is_ok());
        assert!(Analysis::Ac {
            sweep: Sweep::Dec,
            points: 10,
            fstart: 1.0,
            fstop: 1e6
        }
        .validate()
        .is_ok());
        assert!(Analysis::Ac {
            sweep: Sweep::Dec,
            points: 0,
            fstart: 1.0,
            fstop: 1e6
        }
        .validate()
        .is_err());
        assert!(Analysis::Ac {
            sweep: Sweep::Dec,
            points: 10,
            fstart: 0.0,
            fstop: 1e6
        }
        .validate()
        .is_err());
        assert!(Analysis::Tran {
            tstep: 1e-6,
            tstop: 1e-3,
            tstart: 0.0,
            tmax: None,
            uic: false
        }
        .validate()
        .is_ok());
        assert!(Analysis::Tran {
            tstep: 1e-6,
            tstop: -1.0,
            tstart: 0.0,
            tmax: None,
            uic: false
        }
        .validate()
        .is_err());
        assert!(Analysis::Dc {
            source: "V1".into(),
            start: 0.0,
            stop: 1e12,
            step: 1e-3
        }
        .validate()
        .is_err());
        assert!(Analysis::Dc {
            source: "V1".into(),
            start: 0.0,
            stop: 5.0,
            step: 0.0
        }
        .validate()
        .is_err());
    }
}

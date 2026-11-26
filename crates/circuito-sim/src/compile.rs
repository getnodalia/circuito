// SPDX-License-Identifier: MIT OR Apache-2.0
//! From a flat netlist to the numbered circuit the solver works on: unknown layout, resolved
//! device parameters, storage slots and the topology checks that give precise errors before any
//! matrix is built.

use std::collections::{HashMap, VecDeque};

use circuito_core::netlist::{Element, Model, ModelKind, Netlist, NodeId};
use circuito_core::source::Source;

use crate::devices::{thermal_voltage, vcrit, BjtP, DiodeP, MosP};
use crate::error::SimError;
use crate::options::Options;
use crate::result::Unit;

/// The index used for ground: it has no unknown.
pub(crate) const NONE: usize = usize::MAX;
/// The model temperature in kelvin (27 degrees Celsius).
const TNOM_K: f64 = 300.15;
/// The most unknowns a circuit may have (the matrices are dense).
pub const MAX_UNKNOWNS: usize = 2_500;

/// One resolved device. Node fields are unknown indices (`NONE` for ground).
#[derive(Debug, Clone)]
pub(crate) enum Dev {
    R {
        a: usize,
        b: usize,
        g: f64,
    },
    C {
        a: usize,
        b: usize,
        c: f64,
        k: usize,
        ic: Option<f64>,
        ubr: usize,
    },
    L {
        a: usize,
        b: usize,
        br: usize,
        l: f64,
        k: usize,
        ic: Option<f64>,
        mutual: Vec<(usize, f64)>,
    },
    V {
        p: usize,
        n: usize,
        br: usize,
        source: Source,
    },
    I {
        p: usize,
        n: usize,
        source: Source,
    },
    E {
        p: usize,
        n: usize,
        cp: usize,
        cn: usize,
        br: usize,
        gain: f64,
    },
    G {
        p: usize,
        n: usize,
        cp: usize,
        cn: usize,
        gm: f64,
    },
    F {
        p: usize,
        n: usize,
        ctl: usize,
        gain: f64,
    },
    H {
        p: usize,
        n: usize,
        br: usize,
        ctl: usize,
        rm: f64,
    },
    S {
        p: usize,
        n: usize,
        cp: usize,
        cn: usize,
        vt: f64,
        vh: f64,
        ron: f64,
        roff: f64,
    },
    D {
        a: usize,
        ai: usize,
        c: usize,
        rs_g: f64,
        p: DiodeP,
        k: usize,
        vcrit: f64,
    },
    Q {
        c: usize,
        cx: usize,
        bx: usize,
        ex: usize,
        b: usize,
        e: usize,
        rc_g: f64,
        rb_g: f64,
        re_g: f64,
        p: BjtP,
        pol: f64,
        kbe: usize,
        kbc: usize,
        vcrit_f: f64,
        vcrit_r: f64,
    },
    M {
        d: usize,
        dx: usize,
        g: usize,
        sx: usize,
        s: usize,
        b: usize,
        rd_g: f64,
        rs_g: f64,
        p: MosP,
        pol: f64,
        caps: Vec<(usize, usize, f64, usize)>,
    },
    U {
        inp: usize,
        inn: usize,
        out: usize,
        gain: f64,
        vmax: Option<f64>,
        vmin: Option<f64>,
        gout: f64,
    },
}

/// How an output signal is computed from the solution vector.
#[derive(Debug, Clone)]
pub(crate) enum Sig {
    Node(usize),
    Branch(usize),
    Res { a: usize, b: usize, g: f64 },
    Diode { dev: usize },
}

/// A compiled circuit.
#[derive(Debug)]
pub(crate) struct Circuit {
    /// Number of unknowns.
    pub n: usize,
    /// Number of storage slots (charges and fluxes).
    pub n_store: usize,
    /// Description of each unknown, for error messages.
    pub names: Vec<String>,
    /// True for unknowns that are currents (tolerance `abstol`) rather than voltages.
    pub is_current: Vec<bool>,
    /// The plain name of each user node unknown (`names` for the others).
    pub node_plain: Vec<String>,
    pub devs: Vec<Dev>,
    /// Output signals in order.
    pub sigs: Vec<(String, Unit, Sig)>,
    /// Device index of every V and I source by upper-case name.
    pub sources: HashMap<String, usize>,
    /// True when some device needs Newton iteration.
    pub nonlinear: bool,
    /// True when some switch has state.
    pub has_switch: bool,
    /// Element names by device index.
    pub dev_names: Vec<String>,
}

fn nix(id: NodeId) -> usize {
    if id.is_ground() {
        NONE
    } else {
        id.0 as usize - 1
    }
}

fn bad(model: &Model, msg: String) -> SimError {
    SimError::BadModel {
        model: model.name.clone(),
        message: msg,
    }
}

/// Fetch a model parameter and check it with `ok`; `what` completes "`KEY` must be ...".
fn param(
    m: &Model,
    key: &str,
    default: f64,
    ok: impl Fn(f64) -> bool,
    what: &str,
) -> Result<f64, SimError> {
    let v = m.get(key, default);
    if v.is_finite() && ok(v) {
        Ok(v)
    } else {
        Err(bad(m, format!("{key} must be {what}, got {v}")))
    }
}

fn find_model<'a>(
    n: &'a Netlist,
    name: &str,
    kinds: &[ModelKind],
    element: &str,
) -> Result<&'a Model, SimError> {
    let m = n
        .models
        .get(&name.to_uppercase())
        .ok_or_else(|| SimError::Netlist(format!("{element}: no .model named {name}")))?;
    if kinds.contains(&m.kind) {
        Ok(m)
    } else {
        Err(SimError::Netlist(format!(
            "{element}: model {} is a {} model, which this device cannot use",
            m.name, m.kind
        )))
    }
}

/// Saturation-current temperature scaling (SPICE2): exponential in the band-gap term and a power
/// law with `xti`, at `temp_c` against the 27 degree reference.
fn is_at_temp(is: f64, n: f64, xti: f64, temp_c: f64) -> f64 {
    let t = temp_c + 273.15;
    let ratio = t / TNOM_K;
    let vt = thermal_voltage(temp_c);
    is * (1.11 * (ratio - 1.0) / (n * vt)).exp() * ratio.powf(xti / n)
}

struct Builder {
    names: Vec<String>,
    is_current: Vec<bool>,
    n_store: usize,
}

impl Builder {
    fn internal(&mut self, desc: String) -> usize {
        self.names.push(desc);
        self.is_current.push(false);
        self.names.len() - 1
    }
    fn branch(&mut self, desc: String) -> usize {
        self.names.push(desc);
        self.is_current.push(true);
        self.names.len() - 1
    }
    fn store(&mut self) -> usize {
        self.n_store += 1;
        self.n_store - 1
    }
}

impl Circuit {
    /// Compile `netlist` for the options' temperature. With `uic`, every capacitor also gets a
    /// branch unknown so the initial-condition solve can pin its voltage.
    pub(crate) fn compile(
        netlist: &Netlist,
        opts: &Options,
        uic: bool,
    ) -> Result<Circuit, SimError> {
        netlist.validate()?;
        let flat = netlist.flatten()?;
        let n_user = flat.nodes.len().saturating_sub(1);
        if n_user == 0
            || !flat
                .elements
                .iter()
                .any(|e| e.nodes().iter().any(|n| n.is_ground()))
        {
            return Err(SimError::NoGround);
        }
        let mut b = Builder {
            names: Vec::new(),
            is_current: Vec::new(),
            n_store: 0,
        };
        let mut node_plain = Vec::new();
        for i in 1..=n_user {
            let name = flat.nodes.name(NodeId(i as u32)).to_string();
            b.names.push(format!("node {name}"));
            b.is_current.push(false);
            node_plain.push(name);
        }
        // Branch unknowns first, so control references can be resolved in one pass.
        let mut branch_of: HashMap<String, usize> = HashMap::new();
        for e in &flat.elements {
            match e {
                Element::VSource { name, .. }
                | Element::Inductor { name, .. }
                | Element::Vcvs { name, .. }
                | Element::Ccvs { name, .. } => {
                    let br = b.branch(format!("the current of {name}"));
                    branch_of.insert(name.to_uppercase(), br);
                }
                _ => {}
            }
        }
        let mut devs: Vec<Dev> = Vec::with_capacity(flat.elements.len());
        let mut dev_names: Vec<String> = Vec::with_capacity(flat.elements.len());
        let mut sources = HashMap::new();
        let mut l_dev: HashMap<String, usize> = HashMap::new();
        let mut mutuals: Vec<(String, String, f64, String)> = Vec::new();
        let vt = thermal_voltage(opts.temp);
        let finite_nonzero = |v: f64, what: &str, name: &str| {
            if v.is_finite() && v != 0.0 {
                Ok(v)
            } else {
                Err(SimError::Netlist(format!(
                    "{name}: {what} must be a finite non-zero number, got {v}"
                )))
            }
        };
        let ctl_of = |element: &str, control: &str| {
            branch_of.get(&control.to_uppercase()).copied().ok_or_else(|| {
                SimError::Netlist(format!(
                    "{element}: controlling element {control} is not a voltage source, inductor or controlled voltage source"
                ))
            })
        };
        for e in &flat.elements {
            let dev = match e {
                Element::Resistor {
                    name,
                    a,
                    b: bb,
                    value,
                } => {
                    let v = finite_nonzero(*value, "the resistance", name)?;
                    Dev::R {
                        a: nix(*a),
                        b: nix(*bb),
                        g: 1.0 / v,
                    }
                }
                Element::Capacitor {
                    name,
                    a,
                    b: bb,
                    value,
                    ic,
                } => {
                    if !value.is_finite() || *value < 0.0 {
                        return Err(SimError::Netlist(format!(
                            "{name}: the capacitance must be finite and not negative, got {value}"
                        )));
                    }
                    let ubr = if uic {
                        b.branch(format!("the initial-condition current of {name}"))
                    } else {
                        NONE
                    };
                    let k = b.store();
                    Dev::C {
                        a: nix(*a),
                        b: nix(*bb),
                        c: *value,
                        k,
                        ic: *ic,
                        ubr,
                    }
                }
                Element::Inductor {
                    name,
                    a,
                    b: bb,
                    value,
                    ic,
                } => {
                    if !value.is_finite() || *value < 0.0 {
                        return Err(SimError::Netlist(format!(
                            "{name}: the inductance must be finite and not negative, got {value}"
                        )));
                    }
                    let br = branch_of.get(&name.to_uppercase()).copied().unwrap_or(NONE);
                    let k = b.store();
                    l_dev.insert(name.to_uppercase(), devs.len());
                    Dev::L {
                        a: nix(*a),
                        b: nix(*bb),
                        br,
                        l: *value,
                        k,
                        ic: *ic,
                        mutual: Vec::new(),
                    }
                }
                Element::VSource { name, p, n, source } => {
                    let br = branch_of.get(&name.to_uppercase()).copied().unwrap_or(NONE);
                    sources.insert(name.to_uppercase(), devs.len());
                    Dev::V {
                        p: nix(*p),
                        n: nix(*n),
                        br,
                        source: source.clone(),
                    }
                }
                Element::ISource { name, p, n, source } => {
                    sources.insert(name.to_uppercase(), devs.len());
                    Dev::I {
                        p: nix(*p),
                        n: nix(*n),
                        source: source.clone(),
                    }
                }
                Element::Vcvs {
                    name,
                    p,
                    n,
                    cp,
                    cn,
                    gain,
                } => {
                    let br = branch_of.get(&name.to_uppercase()).copied().unwrap_or(NONE);
                    Dev::E {
                        p: nix(*p),
                        n: nix(*n),
                        cp: nix(*cp),
                        cn: nix(*cn),
                        br,
                        gain: *gain,
                    }
                }
                Element::Vccs {
                    p, n, cp, cn, gm, ..
                } => Dev::G {
                    p: nix(*p),
                    n: nix(*n),
                    cp: nix(*cp),
                    cn: nix(*cn),
                    gm: *gm,
                },
                Element::Cccs {
                    name,
                    p,
                    n,
                    control,
                    gain,
                } => Dev::F {
                    p: nix(*p),
                    n: nix(*n),
                    ctl: ctl_of(name, control)?,
                    gain: *gain,
                },
                Element::Ccvs {
                    name,
                    p,
                    n,
                    control,
                    rm,
                } => {
                    let br = branch_of.get(&name.to_uppercase()).copied().unwrap_or(NONE);
                    Dev::H {
                        p: nix(*p),
                        n: nix(*n),
                        br,
                        ctl: ctl_of(name, control)?,
                        rm: *rm,
                    }
                }
                Element::Mutual { name, l1, l2, k } => {
                    mutuals.push((l1.to_uppercase(), l2.to_uppercase(), *k, name.clone()));
                    dev_names.push(name.clone());
                    // Mutual inductance has no device of its own: it couples two inductors.
                    devs.push(Dev::R {
                        a: NONE,
                        b: NONE,
                        g: 0.0,
                    });
                    continue;
                }
                Element::Switch {
                    name,
                    p,
                    n,
                    cp,
                    cn,
                    model,
                } => {
                    let m = find_model(&flat, model, &[ModelKind::Sw], name)?;
                    let ron = param(m, "RON", 1.0, |v| v > 0.0, "positive")?;
                    let roff = param(m, "ROFF", 1e12, |v| v > 0.0, "positive")?;
                    if roff <= ron {
                        return Err(bad(m, format!("ROFF ({roff}) must exceed RON ({ron})")));
                    }
                    let vh = param(m, "VH", 0.0, |v| v >= 0.0, "not negative")?;
                    let vth = param(m, "VT", 0.0, |_| true, "finite")?;
                    Dev::S {
                        p: nix(*p),
                        n: nix(*n),
                        cp: nix(*cp),
                        cn: nix(*cn),
                        vt: vth,
                        vh,
                        ron,
                        roff,
                    }
                }
                Element::Diode {
                    name,
                    anode,
                    cathode,
                    model,
                    area,
                } => {
                    let m = find_model(&flat, model, &[ModelKind::D], name)?;
                    let area = check_area(*area, name)?;
                    let n_em = param(m, "N", 1.0, |v| v > 0.0, "positive")?;
                    let is = param(m, "IS", 1e-14, |v| v > 0.0, "positive")?;
                    let rs = param(m, "RS", 0.0, |v| v >= 0.0, "not negative")?;
                    let tt = param(m, "TT", 0.0, |v| v >= 0.0, "not negative")?;
                    let cjo = param(m, "CJO", 0.0, |v| v >= 0.0, "not negative")?;
                    let vj = param(m, "VJ", 1.0, |v| v > 0.0, "positive")?;
                    let mg = param(m, "M", 0.5, |v| v >= 0.0, "not negative")?;
                    let fc = param(m, "FC", 0.5, |v| (0.0..1.0).contains(&v), "in [0, 1)")?;
                    let bv = if m.params.contains_key("BV") {
                        Some(param(m, "BV", 0.0, |v| v > 0.0, "positive")?)
                    } else {
                        None
                    };
                    let ibv = param(m, "IBV", 1e-3, |v| v > 0.0, "positive")?;
                    let p = DiodeP {
                        is: is_at_temp(is, n_em, 3.0, opts.temp) * area,
                        n: n_em,
                        rs: rs / area,
                        tt,
                        cjo: cjo * area,
                        vj,
                        m: mg,
                        fc,
                        bv,
                        ibv: ibv * area,
                        vt,
                    };
                    let a = nix(*anode);
                    let ai = if p.rs > 0.0 {
                        b.internal(format!("the internal anode of {name}"))
                    } else {
                        a
                    };
                    let k = if p.cjo > 0.0 || p.tt > 0.0 {
                        b.store()
                    } else {
                        NONE
                    };
                    Dev::D {
                        a,
                        ai,
                        c: nix(*cathode),
                        rs_g: if p.rs > 0.0 { 1.0 / p.rs } else { 0.0 },
                        vcrit: vcrit(n_em * vt, p.is),
                        p,
                        k,
                    }
                }
                Element::Bjt {
                    name,
                    c,
                    b: bb,
                    e: ee,
                    model,
                    area,
                } => {
                    let m = find_model(&flat, model, &[ModelKind::Npn, ModelKind::Pnp], name)?;
                    let area = check_area(*area, name)?;
                    let pol = if m.kind == ModelKind::Npn { 1.0 } else { -1.0 };
                    let pos = |key: &str, d: f64| param(m, key, d, |v| v > 0.0, "positive");
                    let nonneg = |key: &str, d: f64| param(m, key, d, |v| v >= 0.0, "not negative");
                    let nf = pos("NF", 1.0)?;
                    let nr = pos("NR", 1.0)?;
                    let is = pos("IS", 1e-16)?;
                    let vaf = nonneg("VAF", 0.0)?;
                    let var = nonneg("VAR", 0.0)?;
                    let fc = param(m, "FC", 0.5, |v| (0.0..1.0).contains(&v), "in [0, 1)")?;
                    let p = BjtP {
                        is: is_at_temp(is, 1.0, 3.0, opts.temp) * area,
                        bf: pos("BF", 100.0)?,
                        br: pos("BR", 1.0)?,
                        nf,
                        nr,
                        inv_vaf: if vaf > 0.0 { 1.0 / vaf } else { 0.0 },
                        inv_var: if var > 0.0 { 1.0 / var } else { 0.0 },
                        vt,
                        cje: nonneg("CJE", 0.0)? * area,
                        vje: pos("VJE", 0.75)?,
                        mje: nonneg("MJE", 0.33)?,
                        cjc: nonneg("CJC", 0.0)? * area,
                        vjc: pos("VJC", 0.75)?,
                        mjc: nonneg("MJC", 0.33)?,
                        fc,
                        tf: nonneg("TF", 0.0)?,
                        tr: nonneg("TR", 0.0)?,
                    };
                    let rc = nonneg("RC", 0.0)? / area;
                    let rb = nonneg("RB", 0.0)? / area;
                    let re = nonneg("RE", 0.0)? / area;
                    let (c0, b0, e0) = (nix(*c), nix(*bb), nix(*ee));
                    let cx = if rc > 0.0 {
                        b.internal(format!("the internal collector of {name}"))
                    } else {
                        c0
                    };
                    let bx = if rb > 0.0 {
                        b.internal(format!("the internal base of {name}"))
                    } else {
                        b0
                    };
                    let ex = if re > 0.0 {
                        b.internal(format!("the internal emitter of {name}"))
                    } else {
                        e0
                    };
                    let kbe = if p.cje > 0.0 || p.tf > 0.0 {
                        b.store()
                    } else {
                        NONE
                    };
                    let kbc = if p.cjc > 0.0 || p.tr > 0.0 {
                        b.store()
                    } else {
                        NONE
                    };
                    Dev::Q {
                        c: c0,
                        cx,
                        bx,
                        ex,
                        b: b0,
                        e: e0,
                        rc_g: if rc > 0.0 { 1.0 / rc } else { 0.0 },
                        rb_g: if rb > 0.0 { 1.0 / rb } else { 0.0 },
                        re_g: if re > 0.0 { 1.0 / re } else { 0.0 },
                        vcrit_f: vcrit(nf * vt, p.is),
                        vcrit_r: vcrit(nr * vt, p.is),
                        p,
                        pol,
                        kbe,
                        kbc,
                    }
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
                    let m = find_model(&flat, model, &[ModelKind::Nmos, ModelKind::Pmos], name)?;
                    let pol = if m.kind == ModelKind::Nmos { 1.0 } else { -1.0 };
                    let level = m.get("LEVEL", 1.0);
                    if level != 1.0 {
                        return Err(bad(
                            m,
                            format!("only LEVEL 1 is implemented, got LEVEL {level}"),
                        ));
                    }
                    let wv = match w {
                        Some(v) => *v,
                        None => m.get("W", 1e-4),
                    };
                    let lv = match l {
                        Some(v) => *v,
                        None => m.get("L", 1e-4),
                    };
                    if !(wv.is_finite() && wv > 0.0 && lv.is_finite() && lv > 0.0) {
                        return Err(SimError::Netlist(format!(
                            "{name}: W and L must be positive, got W={wv} L={lv}"
                        )));
                    }
                    let kp = param(m, "KP", 2e-5, |v| v > 0.0, "positive")?;
                    let t_ratio = (opts.temp + 273.15) / TNOM_K;
                    let p = MosP {
                        vto: pol * param(m, "VTO", 0.0, |_| true, "finite")?,
                        beta: kp * wv / lv * t_ratio.powf(-1.5),
                        gamma: param(m, "GAMMA", 0.0, |v| v >= 0.0, "not negative")?,
                        phi: param(m, "PHI", 0.6, |v| v > 0.0, "positive")?,
                        lambda: param(m, "LAMBDA", 0.0, |v| v >= 0.0, "not negative")?,
                    };
                    let rd = param(m, "RD", 0.0, |v| v >= 0.0, "not negative")?;
                    let rs = param(m, "RS", 0.0, |v| v >= 0.0, "not negative")?;
                    let (d0, g0, s0, b0) = (nix(*d), nix(*g), nix(*s), nix(*bulk));
                    let dx = if rd > 0.0 {
                        b.internal(format!("the internal drain of {name}"))
                    } else {
                        d0
                    };
                    let sx = if rs > 0.0 {
                        b.internal(format!("the internal source of {name}"))
                    } else {
                        s0
                    };
                    let cgso = param(m, "CGSO", 0.0, |v| v >= 0.0, "not negative")? * wv;
                    let cgdo = param(m, "CGDO", 0.0, |v| v >= 0.0, "not negative")? * wv;
                    let cgbo = param(m, "CGBO", 0.0, |v| v >= 0.0, "not negative")? * lv;
                    let cbd = param(m, "CBD", 0.0, |v| v >= 0.0, "not negative")?;
                    let cbs = param(m, "CBS", 0.0, |v| v >= 0.0, "not negative")?;
                    let mut caps = Vec::new();
                    for (a, bb, c) in [
                        (g0, sx, cgso),
                        (g0, dx, cgdo),
                        (g0, b0, cgbo),
                        (b0, dx, cbd),
                        (b0, sx, cbs),
                    ] {
                        if c > 0.0 {
                            caps.push((a, bb, c, b.store()));
                        }
                    }
                    Dev::M {
                        d: d0,
                        dx,
                        g: g0,
                        sx,
                        s: s0,
                        b: b0,
                        rd_g: if rd > 0.0 { 1.0 / rd } else { 0.0 },
                        rs_g: if rs > 0.0 { 1.0 / rs } else { 0.0 },
                        p,
                        pol,
                        caps,
                    }
                }
                Element::OpAmp {
                    name,
                    inp,
                    inn,
                    out,
                    model,
                } => {
                    let m = find_model(&flat, model, &[ModelKind::OpAmp], name)?;
                    let ideal = m.get("IDEAL", 0.0) != 0.0;
                    let gain = if ideal {
                        1e9
                    } else {
                        param(m, "GAIN", 1e5, |v| v > 0.0, "positive")?
                    };
                    let rout = if ideal {
                        1e-3
                    } else {
                        param(m, "ROUT", 75.0, |v| v > 0.0, "positive")?
                    };
                    let vmax = if m.params.contains_key("VMAX") {
                        Some(param(m, "VMAX", 0.0, |v| v > 0.0, "positive")?)
                    } else {
                        None
                    };
                    let vmin = if m.params.contains_key("VMIN") {
                        Some(param(m, "VMIN", 0.0, |v| v < 0.0, "negative")?)
                    } else {
                        None
                    };
                    Dev::U {
                        inp: nix(*inp),
                        inn: nix(*inn),
                        out: nix(*out),
                        gain,
                        vmax,
                        vmin,
                        gout: 1.0 / rout,
                    }
                }
                Element::Instance { name, .. } => {
                    return Err(SimError::Netlist(format!(
                        "{name}: subcircuit instance left after flattening"
                    )));
                }
            };
            dev_names.push(e.name().to_string());
            devs.push(dev);
        }
        // Mutual inductors couple two inductors' fluxes.
        for (l1, l2, k, name) in mutuals {
            let (Some(&i1), Some(&i2)) = (l_dev.get(&l1), l_dev.get(&l2)) else {
                return Err(SimError::Netlist(format!(
                    "{name}: coupled inductor {l1} or {l2} does not exist"
                )));
            };
            if i1 == i2 {
                return Err(SimError::Netlist(format!(
                    "{name}: an inductor cannot be coupled to itself"
                )));
            }
            if !(k.is_finite() && k.abs() <= 1.0) {
                return Err(SimError::Netlist(format!(
                    "{name}: the coupling must satisfy |k| <= 1, got {k}"
                )));
            }
            let get = |d: &Dev| match d {
                Dev::L { br, l, .. } => Some((*br, *l)),
                _ => None,
            };
            let (Some(a), Some(c)) = (devs.get(i1).and_then(get), devs.get(i2).and_then(get))
            else {
                return Err(SimError::Netlist(format!(
                    "{name}: {l1} and {l2} must be inductors"
                )));
            };
            let m = k * (a.1 * c.1).sqrt();
            if let Some(Dev::L { mutual, .. }) = devs.get_mut(i1) {
                mutual.push((c.0, m));
            }
            if let Some(Dev::L { mutual, .. }) = devs.get_mut(i2) {
                mutual.push((a.0, m));
            }
        }
        let n = b.names.len();
        if n > MAX_UNKNOWNS {
            return Err(SimError::TooLarge(format!(
                "the circuit needs {n} unknowns; the dense solver is limited to {MAX_UNKNOWNS}"
            )));
        }
        let nonlinear = devs.iter().any(|d| {
            matches!(
                d,
                Dev::D { .. }
                    | Dev::Q { .. }
                    | Dev::M { .. }
                    | Dev::U { vmax: Some(_), .. }
                    | Dev::U { vmin: Some(_), .. }
            )
        });
        let has_switch = devs.iter().any(|d| matches!(d, Dev::S { .. }));
        // Output signals.
        let mut sigs = Vec::new();
        for (i, name) in node_plain.iter().enumerate() {
            sigs.push((format!("V({name})"), Unit::Volt, Sig::Node(i)));
        }
        for (i, d) in devs.iter().enumerate() {
            let name = dev_names.get(i).cloned().unwrap_or_default();
            let s = match d {
                Dev::V { br, .. } | Dev::L { br, .. } | Dev::E { br, .. } | Dev::H { br, .. } => {
                    Sig::Branch(*br)
                }
                Dev::R { a, b: bb, g } if !(*a == NONE && *bb == NONE) => Sig::Res {
                    a: *a,
                    b: *bb,
                    g: *g,
                },
                Dev::D { .. } => Sig::Diode { dev: i },
                _ => continue,
            };
            sigs.push((format!("I({name})"), Unit::Ampere, s));
        }
        Ok(Circuit {
            n,
            n_store: b.n_store,
            names: b.names,
            is_current: b.is_current,
            node_plain,
            devs,
            sigs,
            sources,
            nonlinear,
            has_switch,
            dev_names,
        })
    }

    /// Check the topology for the problems that make the DC equations unsolvable, and say which
    /// elements or node are to blame. `dc` is true when a DC solution will be computed (inductors
    /// are then short circuits and capacitors open).
    pub(crate) fn check_topology(&self, dc: bool) -> Result<(), SimError> {
        let ground = self.n;
        let m = |i: usize| if i == NONE { ground } else { i };
        // 1. Loops of ideal voltage sources (and, at DC, inductors).
        let mut uf = UnionFind::new(self.n + 1);
        let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); self.n + 1];
        for (i, d) in self.devs.iter().enumerate() {
            let (p, n, is_l) = match d {
                Dev::V { p, n, .. } | Dev::E { p, n, .. } | Dev::H { p, n, .. } => {
                    (m(*p), m(*n), false)
                }
                Dev::L { a, b, .. } if dc => (m(*a), m(*b), true),
                _ => continue,
            };
            if uf.find(p) == uf.find(n) {
                let mut names = Vec::new();
                if p != n {
                    if let Some(path) = bfs_path(&adj, p, n) {
                        for e in path {
                            names.push(self.dev_names.get(e).cloned().unwrap_or_default());
                        }
                    }
                }
                names.push(self.dev_names.get(i).cloned().unwrap_or_default());
                let with_l = names.iter().any(|nm| {
                    self.dev_names
                        .iter()
                        .position(|x| x == nm)
                        .and_then(|ix| self.devs.get(ix))
                        .is_some_and(|d| matches!(d, Dev::L { .. }))
                }) || is_l;
                return Err(SimError::VoltageLoop {
                    elements: names,
                    inductors: if with_l {
                        " (inductors are short circuits at DC)".to_string()
                    } else {
                        String::new()
                    },
                });
            }
            uf.union(p, n);
            if let Some(v) = adj.get_mut(p) {
                v.push((n, i));
            }
            if let Some(v) = adj.get_mut(n) {
                v.push((p, i));
            }
        }
        if !dc {
            return Ok(());
        }
        // 2. Every node needs a DC path to ground.
        let mut uf = UnionFind::new(self.n + 1);
        for d in &self.devs {
            match d {
                Dev::R { a, b, g } if *g != 0.0 => uf.union(m(*a), m(*b)),
                Dev::L { a, b, .. } => uf.union(m(*a), m(*b)),
                Dev::V { p, n, .. } | Dev::E { p, n, .. } | Dev::H { p, n, .. } => {
                    uf.union(m(*p), m(*n))
                }
                Dev::S { p, n, .. } => uf.union(m(*p), m(*n)),
                Dev::D { a, ai, c, .. } => {
                    uf.union(m(*a), m(*ai));
                    uf.union(m(*ai), m(*c));
                }
                Dev::Q {
                    c,
                    cx,
                    bx,
                    ex,
                    b,
                    e,
                    ..
                } => {
                    uf.union(m(*c), m(*cx));
                    uf.union(m(*b), m(*bx));
                    uf.union(m(*e), m(*ex));
                    uf.union(m(*bx), m(*ex));
                    uf.union(m(*bx), m(*cx));
                }
                Dev::M { d, dx, sx, s, .. } => {
                    uf.union(m(*d), m(*dx));
                    uf.union(m(*s), m(*sx));
                    uf.union(m(*dx), m(*sx));
                }
                Dev::U { out, .. } => uf.union(m(*out), ground),
                _ => {}
            }
        }
        let g_root = uf.find(ground);
        for i in 0..self.n {
            let is_node = !self.is_current.get(i).copied().unwrap_or(true);
            if is_node && uf.find(i) != g_root {
                let node = self
                    .node_plain
                    .get(i)
                    .cloned()
                    .or_else(|| self.names.get(i).cloned())
                    .unwrap_or_default();
                return Err(SimError::NoDcPath { node });
            }
        }
        Ok(())
    }

    /// Group unknowns that appear in the same DC-connected component, used to name the
    /// culprit of a singular pivot.
    pub(crate) fn singular_error(&self, column: usize) -> SimError {
        let unknown = self
            .names
            .get(column)
            .cloned()
            .unwrap_or_else(|| format!("unknown {column}"));
        let hint = if self.is_current.get(column).copied().unwrap_or(false) {
            "a loop of voltage sources, inductors or controlled sources over-constrains the circuit, or two sources fight over one node"
        } else {
            "the node is floating or connected only through current sources; connect it to ground through a resistor"
        };
        SimError::Singular {
            unknown,
            hint: hint.to_string(),
        }
    }
}

fn check_area(a: f64, name: &str) -> Result<f64, SimError> {
    if a.is_finite() && a > 0.0 {
        Ok(a)
    } else {
        Err(SimError::Netlist(format!(
            "{name}: the area factor must be positive, got {a}"
        )))
    }
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, mut x: usize) -> usize {
        loop {
            let p = self.parent.get(x).copied().unwrap_or(x);
            if p == x {
                return x;
            }
            let gp = self.parent.get(p).copied().unwrap_or(p);
            if let Some(slot) = self.parent.get_mut(x) {
                *slot = gp;
            }
            x = gp;
        }
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            if let Some(slot) = self.parent.get_mut(ra) {
                *slot = rb;
            }
        }
    }
}

/// The device indices along some path from `from` to `to` in the graph `adj`.
fn bfs_path(adj: &[Vec<(usize, usize)>], from: usize, to: usize) -> Option<Vec<usize>> {
    let mut prev: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut q = VecDeque::new();
    q.push_back(from);
    prev.insert(from, (usize::MAX, usize::MAX));
    while let Some(u) = q.pop_front() {
        if u == to {
            let mut path = Vec::new();
            let mut cur = to;
            while let Some(&(p, e)) = prev.get(&cur) {
                if p == usize::MAX {
                    break;
                }
                path.push(e);
                cur = p;
            }
            path.reverse();
            return Some(path);
        }
        for &(v, e) in adj.get(u).map(Vec::as_slice).unwrap_or(&[]) {
            if let std::collections::hash_map::Entry::Vacant(slot) = prev.entry(v) {
                slot.insert((u, e));
                q.push_back(v);
            }
        }
    }
    None
}

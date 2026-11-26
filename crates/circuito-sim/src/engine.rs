// SPDX-License-Identifier: MIT OR Apache-2.0
//! The modified-nodal-analysis engine: stamping every device into the linearised system, the
//! Newton-Raphson loop and the DC fallback strategies (gmin stepping, source stepping).
//!
//! Layout of one linearised system, written `A x = b` with `A = G + alpha C`:
//! - `G` is the static Jacobian (conductances, branch incidence, Newton companions);
//! - `C` is the charge Jacobian `dq/dx` of every storage slot (capacitor charge, inductor flux,
//!   junction charge);
//! - `alpha` is the integration coefficient (`0` at DC, `1/h` for backward Euler, `2/h` for the
//!   trapezoidal rule), and each storage slot `k` adds `alpha q + beta_k` to its rows, where
//!   `beta_k` carries the history.

use nodalia_numerics::{Lu, Matrix, NumError};

use circuito_core::source::Source;

use crate::compile::{Circuit, Dev, NONE};
use crate::devices::{
    bjt_eval, bjt_q_bc, bjt_q_be, diode_iv, diode_q, fet_limit, mos_eval, opamp_transfer, pnj_limit,
};
use crate::error::SimError;
use crate::options::{Control, Options};

/// Charge (or flux) and its time derivative for one storage slot, at the last accepted point.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Store {
    pub q: f64,
    pub qdot: f64,
}

/// The mutable state of a run.
#[derive(Debug, Clone)]
pub(crate) struct State {
    pub x: Vec<f64>,
    /// Limited junction voltages, per device (up to three).
    pub lim: Vec<[f64; 3]>,
    /// Switch states per device index (only switch entries are used).
    pub sw_on: Vec<bool>,
    pub store: Vec<Store>,
}

impl State {
    pub(crate) fn new(ckt: &Circuit) -> State {
        State {
            x: vec![0.0; ckt.n],
            lim: vec![[0.0; 3]; ckt.devs.len()],
            sw_on: vec![false; ckt.devs.len()],
            store: vec![Store::default(); ckt.n_store],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// DC: sources at their DC values, storage contributes nothing.
    Dc,
    /// Transient: sources at time `t`.
    Tran,
    /// The `uic` initial solve at `t = 0`: capacitor voltages and inductor currents pinned.
    Uic,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub mode: Mode,
    pub alpha: f64,
    pub beta: &'a [f64],
    pub t: f64,
    /// Source scale for source stepping.
    pub scale: f64,
    /// Extra conductance from every node to ground (gmin stepping).
    pub gshunt: f64,
    pub gmin: f64,
    /// A swept source: (device index, value).
    pub sweep: Option<(usize, f64)>,
    /// Apply Newton step limiting to junction voltages.
    pub limit: bool,
}

impl<'a> Ctx<'a> {
    pub(crate) fn dc(gmin: f64) -> Ctx<'static> {
        Ctx {
            mode: Mode::Dc,
            alpha: 0.0,
            beta: &[],
            t: 0.0,
            scale: 1.0,
            gshunt: 0.0,
            gmin,
            sweep: None,
            limit: true,
        }
    }
    fn beta_of(&self, k: usize) -> f64 {
        self.beta.get(k).copied().unwrap_or(0.0)
    }
}

/// The scratch system.
pub(crate) struct Asm {
    n: usize,
    pub g: Vec<f64>,
    pub c: Vec<f64>,
    pub b: Vec<f64>,
    /// Charge of every storage slot at the linearisation point.
    pub q: Vec<f64>,
}

impl Asm {
    pub(crate) fn new(n: usize, n_store: usize) -> Asm {
        Asm {
            n,
            g: vec![0.0; n * n],
            c: vec![0.0; n * n],
            b: vec![0.0; n],
            q: vec![0.0; n_store],
        }
    }
    pub(crate) fn clear(&mut self) {
        self.g.iter_mut().for_each(|v| *v = 0.0);
        self.c.iter_mut().for_each(|v| *v = 0.0);
        self.b.iter_mut().for_each(|v| *v = 0.0);
        self.q.iter_mut().for_each(|v| *v = 0.0);
    }
    #[inline]
    fn ga(&mut self, r: usize, c: usize, v: f64) {
        if r != NONE && c != NONE {
            if let Some(e) = self.g.get_mut(r * self.n + c) {
                *e += v;
            }
        }
    }
    #[inline]
    fn ca(&mut self, r: usize, c: usize, v: f64) {
        if r != NONE && c != NONE {
            if let Some(e) = self.c.get_mut(r * self.n + c) {
                *e += v;
            }
        }
    }
    #[inline]
    fn ba(&mut self, r: usize, v: f64) {
        if r != NONE {
            if let Some(e) = self.b.get_mut(r) {
                *e += v;
            }
        }
    }
    fn cond(&mut self, a: usize, b: usize, g: f64) {
        self.ga(a, a, g);
        self.ga(b, b, g);
        self.ga(a, b, -g);
        self.ga(b, a, -g);
    }
    /// A current `i` flowing from `a` through the element to `b`.
    fn cur(&mut self, a: usize, b: usize, i: f64) {
        self.ba(a, -i);
        self.ba(b, i);
    }
    /// A Norton row: the current leaving node `r` is `sum(d * x[col]) - ieq` after linearisation.
    fn row(&mut self, r: usize, terms: &[(usize, f64)], ieq: f64) {
        for &(c, d) in terms {
            self.ga(r, c, d);
        }
        self.ba(r, -ieq);
    }
    /// A two-terminal storage element between `a` and `b` holding charge `q0` at voltage `v0`
    /// with incremental capacitance `cap`.
    fn store2(
        &mut self,
        ctx: &Ctx,
        k: usize,
        (a, b): (usize, usize),
        (q0, cap, v0): (f64, f64, f64),
    ) {
        if k == NONE {
            return;
        }
        if let Some(slot) = self.q.get_mut(k) {
            *slot = q0;
        }
        self.ca(a, a, cap);
        self.ca(b, b, cap);
        self.ca(a, b, -cap);
        self.ca(b, a, -cap);
        let adj = ctx.alpha * (q0 - cap * v0) + ctx.beta_of(k);
        self.ba(a, -adj);
        self.ba(b, adj);
    }
}

#[inline]
pub(crate) fn v(x: &[f64], i: usize) -> f64 {
    if i == NONE {
        0.0
    } else {
        x.get(i).copied().unwrap_or(0.0)
    }
}

impl Circuit {
    fn source_value(&self, idx: usize, s: &Source, ctx: &Ctx) -> f64 {
        if let Some((si, val)) = ctx.sweep {
            if si == idx {
                return val * ctx.scale;
            }
        }
        let raw = if ctx.mode == Mode::Dc {
            s.dc_value()
        } else {
            s.value_at(ctx.t)
        };
        raw * ctx.scale
    }

    /// Stamp every device into `asm`, linearised about the voltages in `x` (limited against
    /// `lim` when `ctx.limit`). Returns true when any junction voltage was limited.
    pub(crate) fn assemble(
        &self,
        x: &[f64],
        lim: &mut [[f64; 3]],
        sw_on: &[bool],
        ctx: &Ctx,
        asm: &mut Asm,
    ) -> bool {
        let mut limited = false;
        if ctx.gshunt > 0.0 {
            for i in 0..self.n {
                if !self.is_current.get(i).copied().unwrap_or(true) {
                    asm.ga(i, i, ctx.gshunt);
                }
            }
        }
        for (di, d) in self.devs.iter().enumerate() {
            match d {
                Dev::R { a, b, g } => asm.cond(*a, *b, *g),
                Dev::C {
                    a,
                    b,
                    c,
                    k,
                    ic,
                    ubr,
                } => {
                    let vc = v(x, *a) - v(x, *b);
                    if ctx.mode == Mode::Uic && *ubr != NONE {
                        asm.ga(*a, *ubr, 1.0);
                        asm.ga(*b, *ubr, -1.0);
                        asm.ga(*ubr, *a, 1.0);
                        asm.ga(*ubr, *b, -1.0);
                        asm.ba(*ubr, ic.unwrap_or(0.0));
                        if let Some(slot) = asm.q.get_mut(*k) {
                            *slot = c * vc;
                        }
                    } else {
                        // The initial-condition unknown is idle after t = 0: pin it to zero.
                        asm.ga(*ubr, *ubr, 1.0);
                        asm.store2(ctx, *k, (*a, *b), (c * vc, *c, vc));
                    }
                }
                Dev::L {
                    a,
                    b,
                    br,
                    l,
                    k,
                    ic,
                    mutual,
                } => {
                    asm.ga(*a, *br, 1.0);
                    asm.ga(*b, *br, -1.0);
                    let mut flux = l * v(x, *br);
                    for &(obr, m) in mutual {
                        flux += m * v(x, obr);
                    }
                    if let Some(slot) = asm.q.get_mut(*k) {
                        *slot = flux;
                    }
                    if ctx.mode == Mode::Uic {
                        asm.ga(*br, *br, 1.0);
                        asm.ba(*br, ic.unwrap_or(0.0));
                    } else {
                        asm.ga(*br, *a, 1.0);
                        asm.ga(*br, *b, -1.0);
                        asm.ca(*br, *br, -l);
                        for &(obr, m) in mutual {
                            asm.ca(*br, obr, -m);
                        }
                        asm.ba(*br, ctx.beta_of(*k));
                    }
                }
                Dev::V { p, n, br, source } => {
                    asm.ga(*p, *br, 1.0);
                    asm.ga(*n, *br, -1.0);
                    asm.ga(*br, *p, 1.0);
                    asm.ga(*br, *n, -1.0);
                    asm.ba(*br, self.source_value(di, source, ctx));
                }
                Dev::I { p, n, source } => asm.cur(*p, *n, self.source_value(di, source, ctx)),
                Dev::E {
                    p,
                    n,
                    cp,
                    cn,
                    br,
                    gain,
                } => {
                    asm.ga(*p, *br, 1.0);
                    asm.ga(*n, *br, -1.0);
                    asm.ga(*br, *p, 1.0);
                    asm.ga(*br, *n, -1.0);
                    asm.ga(*br, *cp, -gain);
                    asm.ga(*br, *cn, *gain);
                }
                Dev::G { p, n, cp, cn, gm } => {
                    asm.ga(*p, *cp, *gm);
                    asm.ga(*p, *cn, -gm);
                    asm.ga(*n, *cp, -gm);
                    asm.ga(*n, *cn, *gm);
                }
                Dev::F { p, n, ctl, gain } => {
                    asm.ga(*p, *ctl, *gain);
                    asm.ga(*n, *ctl, -gain);
                }
                Dev::H { p, n, br, ctl, rm } => {
                    asm.ga(*p, *br, 1.0);
                    asm.ga(*n, *br, -1.0);
                    asm.ga(*br, *p, 1.0);
                    asm.ga(*br, *n, -1.0);
                    asm.ga(*br, *ctl, -rm);
                }
                Dev::S {
                    p, n, ron, roff, ..
                } => {
                    let on = sw_on.get(di).copied().unwrap_or(false);
                    asm.cond(*p, *n, 1.0 / if on { *ron } else { *roff });
                }
                Dev::D {
                    a,
                    ai,
                    c,
                    rs_g,
                    p,
                    k,
                    vcrit,
                } => {
                    if *rs_g > 0.0 {
                        asm.cond(*a, *ai, *rs_g);
                    }
                    let nvt = p.n * p.vt;
                    let vd = v(x, *ai) - v(x, *c);
                    let vold = lim.get(di).map_or(0.0, |l| l[0]);
                    let (vd_l, ch) = if ctx.limit {
                        match p.bv {
                            Some(bv) if vd < (-bv + 10.0 * nvt).min(0.0) => {
                                let (t, ch) = pnj_limit(-(vd + bv), -(vold + bv), nvt, *vcrit);
                                (-(t + bv), ch)
                            }
                            _ => pnj_limit(vd, vold, nvt, *vcrit),
                        }
                    } else {
                        (vd, false)
                    };
                    limited |= ch;
                    if let Some(l) = lim.get_mut(di) {
                        l[0] = vd_l;
                    }
                    let (i, g) = diode_iv(p, vd_l);
                    let ieq = i - g * vd_l;
                    asm.row(*ai, &[(*ai, g), (*c, -g)], ieq);
                    asm.row(*c, &[(*ai, -g), (*c, g)], -ieq);
                    asm.cond(*ai, *c, ctx.gmin);
                    if *k != NONE {
                        let (q, cap) = diode_q(p, vd_l);
                        asm.store2(ctx, *k, (*ai, *c), (q, cap, vd_l));
                    }
                }
                Dev::Q {
                    c,
                    cx,
                    bx,
                    ex,
                    b,
                    e,
                    rc_g,
                    rb_g,
                    re_g,
                    p,
                    pol,
                    kbe,
                    kbc,
                    vcrit_f,
                    vcrit_r,
                } => {
                    if *rc_g > 0.0 {
                        asm.cond(*c, *cx, *rc_g);
                    }
                    if *rb_g > 0.0 {
                        asm.cond(*b, *bx, *rb_g);
                    }
                    if *re_g > 0.0 {
                        asm.cond(*e, *ex, *re_g);
                    }
                    let vbe = pol * (v(x, *bx) - v(x, *ex));
                    let vbc = pol * (v(x, *bx) - v(x, *cx));
                    let old = lim.get(di).copied().unwrap_or([0.0; 3]);
                    let (vbe_l, c1) = if ctx.limit {
                        pnj_limit(vbe, old[0], p.nf * p.vt, *vcrit_f)
                    } else {
                        (vbe, false)
                    };
                    let (vbc_l, c2) = if ctx.limit {
                        pnj_limit(vbc, old[1], p.nr * p.vt, *vcrit_r)
                    } else {
                        (vbc, false)
                    };
                    limited |= c1 | c2;
                    if let Some(l) = lim.get_mut(di) {
                        l[0] = vbe_l;
                        l[1] = vbc_l;
                    }
                    let ev = bjt_eval(p, vbe_l, vbc_l);
                    let ic_eq = pol * (ev.ic - ev.gc_be * vbe_l - ev.gc_bc * vbc_l);
                    let ib_eq = pol * (ev.ib - ev.gb_be * vbe_l - ev.gb_bc * vbc_l);
                    asm.row(
                        *cx,
                        &[
                            (*bx, ev.gc_be + ev.gc_bc),
                            (*ex, -ev.gc_be),
                            (*cx, -ev.gc_bc),
                        ],
                        ic_eq,
                    );
                    asm.row(
                        *bx,
                        &[
                            (*bx, ev.gb_be + ev.gb_bc),
                            (*ex, -ev.gb_be),
                            (*cx, -ev.gb_bc),
                        ],
                        ib_eq,
                    );
                    asm.row(
                        *ex,
                        &[
                            (*bx, -(ev.gc_be + ev.gc_bc + ev.gb_be + ev.gb_bc)),
                            (*ex, ev.gc_be + ev.gb_be),
                            (*cx, ev.gc_bc + ev.gb_bc),
                        ],
                        -(ic_eq + ib_eq),
                    );
                    asm.cond(*bx, *ex, ctx.gmin);
                    asm.cond(*bx, *cx, ctx.gmin);
                    if *kbe != NONE {
                        let (q, cap) = bjt_q_be(p, vbe_l);
                        asm.store2(ctx, *kbe, (*bx, *ex), (pol * q, cap, pol * vbe_l));
                    }
                    if *kbc != NONE {
                        let (q, cap) = bjt_q_bc(p, vbc_l);
                        asm.store2(ctx, *kbc, (*bx, *cx), (pol * q, cap, pol * vbc_l));
                    }
                }
                Dev::M {
                    d,
                    dx,
                    g,
                    sx,
                    s,
                    b,
                    rd_g,
                    rs_g,
                    p,
                    pol,
                    caps,
                } => {
                    if *rd_g > 0.0 {
                        asm.cond(*d, *dx, *rd_g);
                    }
                    if *rs_g > 0.0 {
                        asm.cond(*s, *sx, *rs_g);
                    }
                    let vgs = pol * (v(x, *g) - v(x, *sx));
                    let vds = pol * (v(x, *dx) - v(x, *sx));
                    let vbs = pol * (v(x, *b) - v(x, *sx));
                    let old = lim.get(di).copied().unwrap_or([0.0; 3]);
                    let (vgs_l, c1) = if ctx.limit {
                        fet_limit(vgs, old[0], p.vto)
                    } else {
                        (vgs, false)
                    };
                    let (vds_l, c2) = if ctx.limit {
                        fet_limit(vds, old[1], 0.0)
                    } else {
                        (vds, false)
                    };
                    limited |= c1 | c2;
                    if let Some(l) = lim.get_mut(di) {
                        l[0] = vgs_l;
                        l[1] = vds_l;
                    }
                    let (id, gm, gds, gmbs) = mos_eval(p, vgs_l, vds_l, vbs);
                    let id_eq = pol * (id - gm * vgs_l - gds * vds_l - gmbs * vbs);
                    let tot = gm + gds + gmbs;
                    asm.row(*dx, &[(*g, gm), (*dx, gds), (*b, gmbs), (*sx, -tot)], id_eq);
                    asm.row(
                        *sx,
                        &[(*g, -gm), (*dx, -gds), (*b, -gmbs), (*sx, tot)],
                        -id_eq,
                    );
                    asm.cond(*dx, *sx, ctx.gmin);
                    for &(a, bb, cap, k) in caps {
                        let vc = v(x, a) - v(x, bb);
                        asm.store2(ctx, k, (a, bb), (cap * vc, cap, vc));
                    }
                }
                Dev::U {
                    inp,
                    inn,
                    out,
                    gain,
                    vmax,
                    vmin,
                    gout,
                } => {
                    let vd = v(x, *inp) - v(x, *inn);
                    let (f, df) = opamp_transfer(*gain, *vmax, *vmin, vd);
                    let ieq = gout * (df * vd - f);
                    asm.row(
                        *out,
                        &[(*out, *gout), (*inp, -gout * df), (*inn, gout * df)],
                        ieq,
                    );
                }
            }
        }
        limited
    }

    /// Update every switch from the control voltages in `x`. Returns true when one changed.
    pub(crate) fn update_switches(&self, x: &[f64], sw_on: &mut [bool]) -> bool {
        let mut changed = false;
        for (i, d) in self.devs.iter().enumerate() {
            if let Dev::S { cp, cn, vt, vh, .. } = d {
                let vc = v(x, *cp) - v(x, *cn);
                if let Some(on) = sw_on.get_mut(i) {
                    if *on && vc < vt - vh {
                        *on = false;
                        changed = true;
                    } else if !*on && vc > vt + vh {
                        *on = true;
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}

/// Counters and notes collected over a run.
#[derive(Debug, Default, Clone)]
pub(crate) struct Stats {
    pub newton_iterations: usize,
    pub factorizations: usize,
    pub accepted: usize,
    pub rejected: usize,
    pub warnings: Vec<String>,
    /// 0 = plain Newton, 1 = gmin stepping, 2 = source stepping.
    pub dc_rank: u8,
}

pub(crate) enum Newton {
    Converged,
    Failed { iters: usize, worst: usize },
}

/// The solver: a circuit, options, hooks and scratch space.
pub(crate) struct Solver<'a> {
    pub ckt: &'a Circuit,
    pub opts: &'a Options,
    pub ctl: &'a Control,
    pub stats: Stats,
    pub asm: Asm,
    tol_abs: Vec<f64>,
}

const SWITCH_ROUNDS: usize = 40;

impl<'a> Solver<'a> {
    pub(crate) fn new(ckt: &'a Circuit, opts: &'a Options, ctl: &'a Control) -> Solver<'a> {
        let tol_abs = ckt
            .is_current
            .iter()
            .map(|&c| if c { opts.abstol } else { opts.vntol })
            .collect();
        Solver {
            ckt,
            opts,
            ctl,
            stats: Stats::default(),
            asm: Asm::new(ckt.n, ckt.n_store),
            tol_abs,
        }
    }

    fn map_num(&self, e: NumError) -> SimError {
        match e {
            NumError::Singular { column } => self.ckt.singular_error(column),
            NumError::NonFinite(w) => SimError::NonFinite(w.to_string()),
            other => SimError::Invalid(other.to_string()),
        }
    }

    /// Factor `G + alpha C` from the scratch system and solve against its right-hand side.
    fn solve_system(&mut self, alpha: f64) -> Result<Vec<f64>, SimError> {
        let n = self.ckt.n;
        let mut a = self.asm.g.clone();
        if alpha != 0.0 {
            for (ai, ci) in a.iter_mut().zip(&self.asm.c) {
                *ai += alpha * ci;
            }
        }
        let m = Matrix::new(n, n, a).map_err(|e| self.map_num(e))?;
        let lu = Lu::factor(&m).map_err(|e| self.map_num(e))?;
        self.stats.factorizations += 1;
        let x = lu.solve(&self.asm.b).map_err(|e| self.map_num(e))?;
        self.check_solution(&x)?;
        Ok(x)
    }

    fn check_solution(&self, x: &[f64]) -> Result<(), SimError> {
        for (i, &xi) in x.iter().enumerate() {
            if !xi.is_finite() {
                return Err(SimError::NonFinite("the solution vector".into()));
            }
            if xi.abs() > 1e15 {
                return Err(self.ckt.singular_error(i));
            }
        }
        Ok(())
    }

    /// Newton-Raphson from `st.x` at the conditions in `ctx`.
    pub(crate) fn newton(
        &mut self,
        st: &mut State,
        ctx: &Ctx,
        max_iter: usize,
    ) -> Result<Newton, SimError> {
        let n = self.ckt.n;
        let mut worst = 0usize;
        for it in 1..=max_iter {
            self.ctl.check()?;
            self.asm.clear();
            let limited = self
                .ckt
                .assemble(&st.x, &mut st.lim, &st.sw_on, ctx, &mut self.asm);
            let xn = self.solve_system(ctx.alpha)?;
            self.stats.newton_iterations += 1;
            if !self.ckt.nonlinear {
                st.x = xn;
                return Ok(Newton::Converged);
            }
            let mut conv = !limited;
            let mut worst_ratio = -1.0;
            for i in 0..n {
                let (a, b) = (
                    xn.get(i).copied().unwrap_or(0.0),
                    st.x.get(i).copied().unwrap_or(0.0),
                );
                let tol = self.opts.reltol * a.abs().max(b.abs())
                    + self.tol_abs.get(i).copied().unwrap_or(1e-6);
                let ratio = (a - b).abs() / tol;
                if ratio > 1.0 {
                    conv = false;
                }
                if ratio > worst_ratio {
                    worst_ratio = ratio;
                    worst = i;
                }
            }
            st.x = xn;
            if conv {
                return Ok(Newton::Converged);
            }
            if it == max_iter {
                return Ok(Newton::Failed { iters: it, worst });
            }
        }
        Ok(Newton::Failed {
            iters: max_iter,
            worst,
        })
    }

    /// A DC solution with the fallback strategies. `st.x` is the starting guess.
    pub(crate) fn dc_solve(
        &mut self,
        st: &mut State,
        base: &Ctx,
        analysis: &str,
    ) -> Result<(), SimError> {
        let itl = self.opts.itl1;
        let x0 = st.x.clone();
        let lim0 = st.lim.clone();
        let first = self.newton(st, base, itl)?;
        let (iters, worst) = match first {
            Newton::Converged => return Ok(()),
            Newton::Failed { iters, worst } => (iters, worst),
        };
        // gmin stepping.
        st.x = vec![0.0; self.ckt.n];
        st.lim = vec![[0.0; 3]; self.ckt.devs.len()];
        if self.gmin_stepping(st, base)? {
            self.stats.dc_rank = self.stats.dc_rank.max(1);
            return Ok(());
        }
        // Source stepping.
        st.x = vec![0.0; self.ckt.n];
        st.lim = vec![[0.0; 3]; self.ckt.devs.len()];
        if self.source_stepping(st, base)? {
            self.stats.dc_rank = self.stats.dc_rank.max(2);
            return Ok(());
        }
        st.x = x0;
        st.lim = lim0;
        Err(SimError::NoConvergence {
            analysis: analysis.to_string(),
            at: String::new(),
            iterations: iters,
            worst: self.ckt.names.get(worst).cloned().unwrap_or_default(),
            tried: "Newton-Raphson with junction limiting, gmin stepping and source stepping"
                .into(),
        })
    }

    fn gmin_stepping(&mut self, st: &mut State, base: &Ctx) -> Result<bool, SimError> {
        let itl = self.opts.itl1;
        let mut prev: Option<f64> = None;
        let mut next = 1e-2f64;
        let mut retries = 0;
        loop {
            let save = (st.x.clone(), st.lim.clone());
            let ctx = Ctx {
                gshunt: next,
                ..*base
            };
            match self.newton(st, &ctx, itl)? {
                Newton::Converged => {
                    if next == 0.0 {
                        return Ok(true);
                    }
                    prev = Some(next);
                    next = if next <= 1e-12 { 0.0 } else { next / 10.0 };
                }
                Newton::Failed { .. } => {
                    st.x = save.0;
                    st.lim = save.1;
                    let Some(p) = prev else { return Ok(false) };
                    retries += 1;
                    if retries > 30 {
                        return Ok(false);
                    }
                    let mid = if next == 0.0 {
                        p * 0.1
                    } else {
                        (p * next).sqrt()
                    };
                    if mid >= p * 0.999 {
                        return Ok(false);
                    }
                    next = mid;
                }
            }
        }
    }

    fn source_stepping(&mut self, st: &mut State, base: &Ctx) -> Result<bool, SimError> {
        let itl = self.opts.itl1;
        let mut lam = 0.0f64;
        let mut step = 0.1f64;
        // lambda = 0 must solve: all sources are off.
        let ctx0 = Ctx {
            scale: 0.0,
            ..*base
        };
        if let Newton::Failed { .. } = self.newton(st, &ctx0, itl)? {
            return Ok(false);
        }
        while lam < 1.0 {
            let target = (lam + step).min(1.0);
            let save = (st.x.clone(), st.lim.clone());
            let ctx = Ctx {
                scale: target,
                ..*base
            };
            match self.newton(st, &ctx, itl)? {
                Newton::Converged => {
                    lam = target;
                    step = (step * 1.5).min(0.25);
                }
                Newton::Failed { .. } => {
                    st.x = save.0;
                    st.lim = save.1;
                    step /= 2.0;
                    if step < 1e-6 {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// A DC solution including the switch-state loop.
    pub(crate) fn dc_op(
        &mut self,
        st: &mut State,
        base: &Ctx,
        analysis: &str,
    ) -> Result<(), SimError> {
        for _ in 0..SWITCH_ROUNDS {
            self.dc_solve(st, base, analysis)?;
            if !self.ckt.has_switch || !self.ckt.update_switches(&st.x, &mut st.sw_on) {
                return Ok(());
            }
        }
        Err(SimError::NoConvergence {
            analysis: analysis.to_string(),
            at: String::new(),
            iterations: SWITCH_ROUNDS,
            worst: "a switch".into(),
            tried: "switch states that never settle (a switch's own control voltage flips it back and forth; give it hysteresis with VH)".into(),
        })
    }

    /// The reciprocal condition estimate of `G` at `x` (the DC matrix), for diagnostics.
    pub(crate) fn rcond_at(&mut self, st: &mut State, base: &Ctx) -> Option<f64> {
        self.asm.clear();
        let mut lim = st.lim.clone();
        let ctx = Ctx {
            limit: false,
            ..*base
        };
        self.ckt
            .assemble(&st.x, &mut lim, &st.sw_on, &ctx, &mut self.asm);
        let m = Matrix::new(self.ckt.n, self.ckt.n, self.asm.g.clone()).ok()?;
        Lu::factor(&m).ok().map(|lu| lu.rcond_estimate())
    }
}

// SPDX-License-Identifier: MIT OR Apache-2.0
//! The analyses: `.op`, `.dc`, `.ac` and `.tran`.

use nodalia_numerics::{Complex64, Lu, Matrix};

use circuito_core::netlist::{Analysis, Netlist, Sweep};

use crate::compile::{Circuit, Dev, Sig, NONE};
use crate::devices::diode_iv;
use crate::engine::{v, Ctx, Mode, Solver, State, Stats};
use crate::error::SimError;
use crate::options::{Control, Integration, Options};
use crate::result::{AnalysisResult, Kind, RunInfo, Signal, Table, Values};

/// Most samples (points times signals) a result may hold.
pub const MAX_SAMPLES: usize = 40_000_000;

const ENGINE: &str = concat!("circuito-sim ", env!("CARGO_PKG_VERSION"));

/// Run one analysis of `netlist`.
///
/// # Errors
/// An invalid request, a netlist the simulator cannot build, a topology that has no solution, a
/// failure to converge, or cancellation. The message names the element or node involved.
pub fn simulate(
    netlist: &Netlist,
    analysis: &Analysis,
    options: &Options,
    control: &Control,
) -> Result<AnalysisResult, SimError> {
    options.validate()?;
    analysis.validate()?;
    match analysis {
        Analysis::Op => run_op(netlist, options, control),
        Analysis::Dc {
            source,
            start,
            stop,
            step,
        } => run_dc(netlist, source, *start, *stop, *step, options, control),
        Analysis::Ac {
            sweep,
            points,
            fstart,
            fstop,
        } => run_ac(netlist, *sweep, *points, *fstart, *fstop, options, control),
        Analysis::Tran {
            tstep,
            tstop,
            tstart,
            tmax,
            uic,
        } => run_tran(
            netlist, *tstep, *tstop, *tstart, *tmax, *uic, options, control,
        ),
    }
}

/// Run every analysis the netlist asks for, with the options its `.options` cards set. Each
/// analysis succeeds or fails on its own.
///
/// # Errors
/// An `.options` value out of range.
pub fn simulate_all(
    netlist: &Netlist,
    control: &Control,
) -> Result<Vec<Result<AnalysisResult, SimError>>, SimError> {
    let opts = Options::from_netlist(netlist)?;
    Ok(netlist
        .analyses
        .iter()
        .map(|a| simulate(netlist, a, &opts, control))
        .collect())
}

/// The frequencies of an `.ac` sweep: `points` per decade or octave (inclusive of `fstart`,
/// stopping at the last value not above `fstop`), or `points` evenly spaced values.
#[must_use]
pub fn ac_frequencies(sweep: Sweep, points: u32, fstart: f64, fstop: f64) -> Vec<f64> {
    let np = f64::from(points.max(1));
    match sweep {
        Sweep::Lin => {
            if points <= 1 {
                vec![fstart]
            } else {
                (0..points)
                    .map(|i| fstart + (fstop - fstart) * f64::from(i) / (np - 1.0))
                    .collect()
            }
        }
        Sweep::Dec | Sweep::Oct => {
            let base: f64 = if sweep == Sweep::Dec { 10.0 } else { 2.0 };
            let span = (fstop / fstart).ln() / base.ln();
            let n = (span * np + 1e-9).floor().max(0.0) as usize;
            (0..=n).map(|i| fstart * base.powf(i as f64 / np)).collect()
        }
    }
}

fn dc_method_name(rank: u8) -> &'static str {
    match rank {
        0 => "newton",
        1 => "gmin stepping",
        _ => "source stepping",
    }
}

fn make_info(
    opts: &Options,
    analysis: String,
    stats: &Stats,
    dc_method: String,
    integration: Option<String>,
    rcond: Option<f64>,
    warnings: Vec<String>,
) -> RunInfo {
    let mut w = warnings;
    w.extend(stats.warnings.iter().cloned());
    RunInfo {
        engine: ENGINE.to_string(),
        analysis,
        dc_method,
        integration,
        reltol: opts.reltol,
        abstol: opts.abstol,
        vntol: opts.vntol,
        gmin: opts.gmin,
        temp: opts.temp,
        newton_iterations: stats.newton_iterations,
        factorizations: stats.factorizations,
        accepted_steps: stats.accepted,
        rejected_steps: stats.rejected,
        rcond,
        warnings: w,
    }
}

fn model_warnings(n: &Netlist) -> Vec<String> {
    let mut out = Vec::new();
    for m in n.models.values() {
        for p in m.unknown_params() {
            out.push(format!(
                "model {}: parameter {p} is not used by the simulator and was ignored",
                m.name
            ));
        }
    }
    out
}

impl Circuit {
    fn sig_real(&self, s: &Sig, x: &[f64], gmin: f64) -> f64 {
        match s {
            Sig::Node(i) | Sig::Branch(i) => v(x, *i),
            Sig::Res { a, b, g } => (v(x, *a) - v(x, *b)) * g,
            Sig::Diode { dev } => match self.devs.get(*dev) {
                Some(Dev::D { ai, c, p, .. }) => {
                    let vd = v(x, *ai) - v(x, *c);
                    diode_iv(p, vd).0 + gmin * vd
                }
                _ => 0.0,
            },
        }
    }

    fn sig_complex(&self, s: &Sig, x: &[Complex64]) -> Option<Complex64> {
        let at = |i: usize| {
            if i == NONE {
                Complex64::new(0.0, 0.0)
            } else {
                x.get(i).copied().unwrap_or(Complex64::new(0.0, 0.0))
            }
        };
        match s {
            Sig::Node(i) | Sig::Branch(i) => Some(at(*i)),
            Sig::Res { a, b, g } => Some((at(*a) - at(*b)) * *g),
            Sig::Diode { .. } => None,
        }
    }

    /// The charge of every storage slot at `st.x`, written into `st.store[..].q`.
    fn load_charges(&self, s: &mut Solver<'_>, st: &mut State, base: &Ctx) {
        s.asm.clear();
        let ctx = Ctx {
            limit: false,
            alpha: 0.0,
            beta: &[],
            ..*base
        };
        let mut lim = st.lim.clone();
        self.assemble(&st.x, &mut lim, &st.sw_on, &ctx, &mut s.asm);
        for (slot, q) in st.store.iter_mut().zip(&s.asm.q) {
            slot.q = *q;
        }
    }
}

fn real_table(ckt: &Circuit, axis_name: &str, axis: Vec<f64>, cols: Vec<Vec<f64>>) -> Table {
    let signals = ckt
        .sigs
        .iter()
        .zip(cols)
        .map(|((name, unit, _), vals)| Signal {
            name: name.clone(),
            unit: *unit,
            values: Values::Real(vals),
        })
        .collect();
    Table {
        axis_name: axis_name.to_string(),
        axis,
        signals,
    }
}

fn run_op(netlist: &Netlist, opts: &Options, ctl: &Control) -> Result<AnalysisResult, SimError> {
    let ckt = Circuit::compile(netlist, opts, false)?;
    ckt.check_topology(true)?;
    let mut s = Solver::new(&ckt, opts, ctl);
    let mut st = State::new(&ckt);
    let base = Ctx::dc(opts.gmin);
    s.dc_op(&mut st, &base, ".op")?;
    let rcond = s.rcond_at(&mut st, &base);
    let mut warnings = model_warnings(netlist);
    if let Some(r) = rcond {
        if r < 1e-16 {
            warnings.push(format!(
                "the operating-point matrix is badly conditioned (reciprocal condition estimate {r:.1e}); the result may carry few correct digits"
            ));
        }
    }
    let cols = ckt
        .sigs
        .iter()
        .map(|(_, _, sig)| vec![ckt.sig_real(sig, &st.x, opts.gmin)])
        .collect();
    let info = make_info(
        opts,
        ".op".into(),
        &s.stats,
        dc_method_name(s.stats.dc_rank).into(),
        None,
        rcond,
        warnings,
    );
    Ok(AnalysisResult {
        kind: Kind::Op,
        table: real_table(&ckt, "op", vec![0.0], cols),
        info,
    })
}

fn run_dc(
    netlist: &Netlist,
    source: &str,
    start: f64,
    stop: f64,
    step: f64,
    opts: &Options,
    ctl: &Control,
) -> Result<AnalysisResult, SimError> {
    let ckt = Circuit::compile(netlist, opts, false)?;
    let dev = *ckt
        .sources
        .get(&source.to_uppercase())
        .ok_or_else(|| SimError::UnknownSource(source.to_string()))?;
    ckt.check_topology(true)?;
    let count = ((stop - start).abs() / step + 1e-9).floor() as usize + 1;
    if count.saturating_mul(ckt.sigs.len().max(1)) > MAX_SAMPLES {
        return Err(SimError::TooLarge(format!(
            "the sweep would store {count} points of {} signals",
            ckt.sigs.len()
        )));
    }
    let dir = if stop >= start { 1.0 } else { -1.0 };
    let mut s = Solver::new(&ckt, opts, ctl);
    let mut st = State::new(&ckt);
    let mut axis = Vec::with_capacity(count);
    let mut cols: Vec<Vec<f64>> = vec![Vec::with_capacity(count); ckt.sigs.len()];
    for i in 0..count {
        ctl.check()?;
        let val = start + dir * step * i as f64;
        let base = Ctx {
            sweep: Some((dev, val)),
            ..Ctx::dc(opts.gmin)
        };
        s.dc_op(&mut st, &base, ".dc")?;
        axis.push(val);
        for (col, (_, _, sig)) in cols.iter_mut().zip(&ckt.sigs) {
            col.push(ckt.sig_real(sig, &st.x, opts.gmin));
        }
        ctl.report((i + 1) as f64 / count as f64);
    }
    let info = make_info(
        opts,
        format!(".dc {source}"),
        &s.stats,
        dc_method_name(s.stats.dc_rank).into(),
        None,
        None,
        model_warnings(netlist),
    );
    Ok(AnalysisResult {
        kind: Kind::Dc,
        table: real_table(&ckt, &source.to_uppercase(), axis, cols),
        info,
    })
}

fn run_ac(
    netlist: &Netlist,
    sweep: Sweep,
    points: u32,
    fstart: f64,
    fstop: f64,
    opts: &Options,
    ctl: &Control,
) -> Result<AnalysisResult, SimError> {
    let ckt = Circuit::compile(netlist, opts, false)?;
    let mut have_ac = false;
    for d in &ckt.devs {
        if let Dev::V { source, .. } | Dev::I { source, .. } = d {
            if source.ac.is_some() {
                have_ac = true;
            }
        }
    }
    if !have_ac {
        return Err(SimError::Invalid("no source has an AC value: give one `ac <magnitude> [phase]` so there is a stimulus to analyse".into()));
    }
    ckt.check_topology(true)?;
    let freqs = ac_frequencies(sweep, points, fstart, fstop);
    let sig_idx: Vec<usize> = ckt
        .sigs
        .iter()
        .enumerate()
        .filter(|(_, (_, _, s))| !matches!(s, Sig::Diode { .. }))
        .map(|(i, _)| i)
        .collect();
    if freqs.len().saturating_mul(sig_idx.len().max(1)) > MAX_SAMPLES {
        return Err(SimError::TooLarge(format!(
            "the sweep would store {} points of {} signals",
            freqs.len(),
            sig_idx.len()
        )));
    }
    let mut s = Solver::new(&ckt, opts, ctl);
    let mut st = State::new(&ckt);
    let base = Ctx::dc(opts.gmin);
    s.dc_op(&mut st, &base, ".ac (operating point)")?;
    // Linearise at the operating point.
    s.asm.clear();
    let mut lim = st.lim.clone();
    let lin = Ctx {
        limit: false,
        ..base
    };
    ckt.assemble(&st.x, &mut lim, &st.sw_on, &lin, &mut s.asm);
    let (g, c) = (s.asm.g.clone(), s.asm.c.clone());
    let n = ckt.n;
    let mut rhs = vec![Complex64::new(0.0, 0.0); n];
    for d in &ckt.devs {
        match d {
            Dev::V { br, source, .. } => {
                if let (Some(ac), Some(slot)) = (&source.ac, rhs.get_mut(*br)) {
                    *slot += Complex64::from_polar(ac.magnitude, ac.phase_deg.to_radians());
                }
            }
            Dev::I { p, n: nn, source } => {
                if let Some(ac) = &source.ac {
                    let ph = Complex64::from_polar(ac.magnitude, ac.phase_deg.to_radians());
                    if *p != NONE {
                        if let Some(slot) = rhs.get_mut(*p) {
                            *slot -= ph;
                        }
                    }
                    if *nn != NONE {
                        if let Some(slot) = rhs.get_mut(*nn) {
                            *slot += ph;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut cols: Vec<Vec<Complex64>> = vec![Vec::with_capacity(freqs.len()); sig_idx.len()];
    for (fi, &f) in freqs.iter().enumerate() {
        ctl.check()?;
        let w = std::f64::consts::TAU * f;
        let data: Vec<Complex64> = g
            .iter()
            .zip(&c)
            .map(|(&gi, &ci)| Complex64::new(gi, w * ci))
            .collect();
        let m = Matrix::new(n, n, data).map_err(|e| SimError::Invalid(e.to_string()))?;
        let lu = Lu::factor(&m).map_err(|e| match e {
            nodalia_numerics::NumError::Singular { column } => ckt.singular_error(column),
            other => SimError::Invalid(other.to_string()),
        })?;
        s.stats.factorizations += 1;
        let x = lu
            .solve(&rhs)
            .map_err(|e| SimError::Invalid(e.to_string()))?;
        if x.iter().any(|z| !z.re.is_finite() || !z.im.is_finite()) {
            return Err(SimError::NonFinite(format!("the AC solution at {f} Hz")));
        }
        for (col, &si) in cols.iter_mut().zip(&sig_idx) {
            let sig = ckt.sigs.get(si).map(|(_, _, s)| s);
            col.push(
                sig.and_then(|s| ckt.sig_complex(s, &x))
                    .unwrap_or(Complex64::new(0.0, 0.0)),
            );
        }
        ctl.report((fi + 1) as f64 / freqs.len() as f64);
    }
    let signals = sig_idx
        .iter()
        .zip(cols)
        .filter_map(|(&si, vals)| {
            ckt.sigs.get(si).map(|(name, unit, _)| Signal {
                name: name.clone(),
                unit: *unit,
                values: Values::Complex(vals),
            })
        })
        .collect();
    let table = Table {
        axis_name: "frequency".into(),
        axis: freqs,
        signals,
    };
    let info = make_info(
        opts,
        format!(".ac {} {points} {fstart} {fstop}", sweep.keyword()),
        &s.stats,
        dc_method_name(s.stats.dc_rank).into(),
        None,
        None,
        model_warnings(netlist),
    );
    Ok(AnalysisResult {
        kind: Kind::Ac,
        table,
        info,
    })
}

/// Divided differences over (t, q) points.
fn dd1(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.1 - a.1) / (b.0 - a.0)
}

#[allow(clippy::too_many_arguments)]
fn run_tran(
    netlist: &Netlist,
    tstep: f64,
    tstop: f64,
    tstart: f64,
    tmax: Option<f64>,
    uic: bool,
    opts: &Options,
    ctl: &Control,
) -> Result<AnalysisResult, SimError> {
    let ckt = Circuit::compile(netlist, opts, uic)?;
    ckt.check_topology(!uic)?;
    let hmax = tmax
        .unwrap_or(f64::INFINITY)
        .min(tstep)
        .min((tstop - tstart) / 50.0)
        .max(tstop * 1e-12);
    let hmin = (tstop * 1e-12).max(1e-21);
    let est = tstop / hmax;
    if est > opts.max_points as f64 {
        return Err(SimError::TooLarge(format!(
            "the transient needs at least {est:.0} time points (tstop / min(tstep, tmax)); the limit is {}. Raise tstep or tmax",
            opts.max_points
        )));
    }
    if (opts.max_points as f64).min(est * 4.0) * ckt.sigs.len().max(1) as f64 > MAX_SAMPLES as f64 {
        return Err(SimError::TooLarge(format!(
            "the result could hold more than {MAX_SAMPLES} samples; raise tstep or tmax"
        )));
    }
    let mut s = Solver::new(&ckt, opts, ctl);
    let mut st = State::new(&ckt);
    // Initial state.
    let dc_method: String;
    if uic {
        let base = Ctx {
            mode: Mode::Uic,
            ..Ctx::dc(opts.gmin)
        };
        s.dc_op(&mut st, &base, ".tran (initial conditions)")?;
        dc_method = "uic (initial conditions)".to_string();
        ckt.load_charges(&mut s, &mut st, &base);
        for d in &ckt.devs {
            match d {
                Dev::C { k, ubr, .. } => {
                    if let Some(slot) = st.store.get_mut(*k) {
                        slot.qdot = v(&st.x, *ubr);
                    }
                }
                Dev::L { a, b, k, .. } => {
                    let vl = v(&st.x, *a) - v(&st.x, *b);
                    if let Some(slot) = st.store.get_mut(*k) {
                        slot.qdot = vl;
                    }
                }
                _ => {}
            }
        }
    } else {
        let base = Ctx::dc(opts.gmin);
        s.dc_op(&mut st, &base, ".tran (operating point)")?;
        dc_method = dc_method_name(s.stats.dc_rank).into();
        ckt.load_charges(&mut s, &mut st, &base);
    }
    let tran_base = Ctx {
        mode: Mode::Tran,
        ..Ctx::dc(opts.gmin)
    };
    // Breakpoints.
    let mut bps: Vec<f64> = Vec::new();
    for d in &ckt.devs {
        if let Dev::V { source, .. } | Dev::I { source, .. } = d {
            if let Some(w) = &source.wave {
                bps.extend(w.breakpoints(tstop, 200_000));
            }
        }
    }
    if tstart > 0.0 {
        bps.push(tstart);
    }
    bps.push(tstop);
    bps.sort_by(f64::total_cmp);
    bps.dedup_by(|a, b| (*a - *b).abs() <= hmin);
    let mut bp_idx = 0usize;

    let nst = ckt.n_store;
    let mut axis: Vec<f64> = Vec::new();
    let mut cols: Vec<Vec<f64>> = vec![Vec::new(); ckt.sigs.len()];
    let record = |t: f64, x: &[f64], axis: &mut Vec<f64>, cols: &mut Vec<Vec<f64>>| {
        axis.push(t);
        for (col, (_, _, sig)) in cols.iter_mut().zip(&ckt.sigs) {
            col.push(ckt.sig_real(sig, x, opts.gmin));
        }
    };
    if tstart <= 0.0 {
        record(0.0, &st.x, &mut axis, &mut cols);
    }

    let mut t = 0.0f64;
    let mut h = (tstep.min(tstop / 100.0) / 10.0).clamp(hmin, hmax);
    let mut times = [0.0f64; 3];
    let mut qh: Vec<[f64; 3]> = st.store.iter().map(|s| [s.q, 0.0, 0.0]).collect();
    let mut qpeak: Vec<f64> = st.store.iter().map(|s| s.q.abs()).collect();
    let mut nh = 1usize;
    let mut restart = true;
    let mut beta = vec![0.0f64; nst];
    let mut bisects = 0usize;
    let mut last_report = 0usize;
    while t < tstop * (1.0 - 1e-14) {
        ctl.check()?;
        if s.stats.accepted + s.stats.rejected > opts.max_points.saturating_mul(10) {
            return Err(SimError::TooLarge(format!(
                "the transient needed more than {} step attempts by t = {t:e} s; the circuit may be stiff or oscillating beyond what the step control can follow",
                opts.max_points.saturating_mul(10)
            )));
        }
        // Skip breakpoints already behind us.
        while bps.get(bp_idx).is_some_and(|&b| b <= t + hmin * 0.5) {
            bp_idx += 1;
        }
        h = h.clamp(hmin, hmax);
        let mut hit_bp = false;
        let mut t_new = t + h;
        if let Some(&b) = bps.get(bp_idx) {
            if t_new >= b - 0.02 * h {
                t_new = b;
                h = b - t;
                hit_bp = true;
            }
        }
        let be = restart || opts.method == Integration::BackwardEuler;
        let alpha = if be { 1.0 / h } else { 2.0 / h };
        for (bk, sk) in beta.iter_mut().zip(&st.store) {
            *bk = if be {
                -sk.q / h
            } else {
                -(2.0 / h) * sk.q - sk.qdot
            };
        }
        let mut ctx = Ctx {
            alpha,
            beta: &beta,
            t: t_new,
            ..tran_base
        };
        let mut trial = st.clone();
        match s.newton(&mut trial, &ctx, opts.itl4)? {
            crate::engine::Newton::Converged => {}
            crate::engine::Newton::Failed { .. } => {
                s.stats.rejected += 1;
                h /= 8.0;
                if h < hmin {
                    return Err(SimError::StepTooSmall { time: t, step: h });
                }
                continue;
            }
        }
        // Switch events.
        let mut flipped = false;
        if ckt.has_switch {
            let mut sw = trial.sw_on.clone();
            if ckt.update_switches(&trial.x, &mut sw) {
                let mut frac = 1.0f64;
                for (i, d) in ckt.devs.iter().enumerate() {
                    if let Dev::S { cp, cn, vt, vh, .. } = d {
                        let was = trial.sw_on.get(i).copied().unwrap_or(false);
                        let now = sw.get(i).copied().unwrap_or(false);
                        if was != now {
                            let thr = if now { vt + vh } else { vt - vh };
                            let vc_old = v(&st.x, *cp) - v(&st.x, *cn);
                            let vc_new = v(&trial.x, *cp) - v(&trial.x, *cn);
                            let f = if vc_new != vc_old {
                                ((thr - vc_old) / (vc_new - vc_old)).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            frac = frac.min(f);
                        }
                    }
                }
                let h_event = (hmin * 100.0).max(1e-9 * tstop);
                if h * frac > h_event && bisects < 14 {
                    bisects += 1;
                    s.stats.rejected += 1;
                    h = (h * frac).max(h_event);
                    continue;
                }
                // Take the event: switch, and re-solve this step in the new state.
                let mut rounds = 0;
                loop {
                    trial.sw_on = sw.clone();
                    match s.newton(&mut trial, &ctx, opts.itl4)? {
                        crate::engine::Newton::Converged => {}
                        crate::engine::Newton::Failed { .. } => {
                            s.stats.rejected += 1;
                            h /= 8.0;
                            if h < hmin {
                                return Err(SimError::StepTooSmall { time: t, step: h });
                            }
                            break;
                        }
                    }
                    rounds += 1;
                    if rounds >= 5 || !ckt.update_switches(&trial.x, &mut sw) {
                        break;
                    }
                }
                if h < hmin * 8.0 && rounds == 0 {
                    continue;
                }
                flipped = true;
            }
        }
        bisects = 0;
        ctx.limit = false;
        // Charges at the accepted point and the truncation-error test.
        ckt.load_charges(&mut s, &mut trial, &ctx);
        let (order_exp, need) = if be { (0.5, 2) } else { (1.0 / 3.0, 3) };
        let mut ratio = f64::INFINITY;
        if nh >= need && !flipped {
            for k in 0..nst {
                let q_new = trial.store.get(k).map_or(0.0, |s| s.q);
                let hist = qh.get(k).copied().unwrap_or([0.0; 3]);
                let q_old = hist[0];
                let tol = opts.reltol
                    * q_new
                        .abs()
                        .max(q_old.abs())
                        .max(0.1 * qpeak.get(k).copied().unwrap_or(0.0))
                    + opts.chgtol;
                let lte = if be {
                    let p1 = (times[1], hist[1]);
                    let p0 = (times[0], hist[0]);
                    let pn = (t_new, q_new);
                    let d2 = (dd1(p0, pn) - dd1(p1, p0)) / (t_new - times[1]);
                    h * h * d2.abs()
                } else {
                    let p2 = (times[2], hist[2]);
                    let p1 = (times[1], hist[1]);
                    let p0 = (times[0], hist[0]);
                    let pn = (t_new, q_new);
                    let d2a = (dd1(p1, p0) - dd1(p2, p1)) / (times[0] - times[2]);
                    let d2b = (dd1(p0, pn) - dd1(p1, p0)) / (t_new - times[1]);
                    let d3 = (d2b - d2a) / (t_new - times[2]);
                    h * h * h * d3.abs() / 2.0
                };
                if lte > 0.0 && lte.is_finite() {
                    ratio = ratio.min((opts.trtol * tol / lte).powf(order_exp));
                }
            }
        }
        if ratio < 1.0 && h > hmin * 1.5 {
            s.stats.rejected += 1;
            h *= (0.9 * ratio).clamp(0.1, 0.9);
            continue;
        }
        // Accept.
        for k in 0..nst {
            let q_new = trial.store.get(k).map_or(0.0, |s| s.q);
            let b = beta.get(k).copied().unwrap_or(0.0);
            if let Some(slot) = trial.store.get_mut(k) {
                slot.qdot = alpha * q_new + b;
            }
            if let Some(hist) = qh.get_mut(k) {
                *hist = [q_new, hist[0], hist[1]];
            }
            if let Some(p) = qpeak.get_mut(k) {
                *p = p.max(q_new.abs());
            }
        }
        times = [t_new, times[0], times[1]];
        nh = (nh + 1).min(3);
        t = t_new;
        st = trial;
        s.stats.accepted += 1;
        if t >= tstart - hmin {
            record(t, &st.x, &mut axis, &mut cols);
        }
        if axis.len() > opts.max_points {
            return Err(SimError::TooLarge(format!(
                "the transient stored more than {} time points; raise tstep or tmax",
                opts.max_points
            )));
        }
        restart = hit_bp || flipped;
        if restart {
            nh = 1;
            h = h.min(hmax / 10.0).max(hmin);
        } else if ratio.is_finite() {
            h *= (0.9 * ratio).clamp(0.9, 2.0);
        } else {
            h *= 1.5;
        }
        if s.stats.accepted - last_report >= 32 {
            last_report = s.stats.accepted;
            ctl.report(t / tstop);
        }
    }
    ctl.report(1.0);
    let integ = match opts.method {
        Integration::Trapezoidal => {
            "trapezoidal (backward Euler for the first step and after breakpoints)"
        }
        Integration::BackwardEuler => "backward Euler",
    };
    let info = make_info(
        opts,
        format!(".tran {tstep} {tstop}"),
        &s.stats,
        dc_method,
        Some(integ.into()),
        None,
        model_warnings(netlist),
    );
    Ok(AnalysisResult {
        kind: Kind::Tran,
        table: real_table(&ckt, "time", axis, cols),
        info,
    })
}
